//! Middle tier people: profiles, people picker and profile pictures.
use crate::path::enc;
use crate::service::{decode_csa, decode_value_list};
use crate::{ChatClient, Error};
use http::{HeaderMap, Method};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use teams_auth::{Authenticator, Service};
use teams_i18n::t;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub mri: String,
    pub display_name: String,
    pub friendly_name: String,
    pub email: String,
    pub role: Option<String>,
}
impl<'de> Deserialize<'de> for Person {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Profile {
            mri: Option<String>,
            id: Option<String>,
            object_id: Option<String>,
            #[serde(alias = "displayname")]
            display_name: Option<String>,
            friendly_name: Option<String>,
            given_name: Option<String>,
            surname: Option<String>,
            email: Option<String>,
            mail: Option<String>,
            user_principal_name: Option<String>,
            #[serde(default, deserialize_with = "csa_role")]
            role: Option<String>,
        }
        let profile = Profile::deserialize(deserializer)?;
        let id = profile
            .mri
            .filter(|s| !s.is_empty())
            .or(profile.id)
            .or(profile.object_id)
            .unwrap_or_default();
        let mri = if uuid::Uuid::parse_str(&id).is_ok() {
            format!("8:orgid:{id}")
        } else {
            id
        };
        Ok(Self {
            mri,
            display_name: profile
                .display_name
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    [profile.given_name, profile.surname]
                        .into_iter()
                        .flatten()
                        .filter(|s| !s.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join(" ")
                }),
            friendly_name: profile.friendly_name.unwrap_or_default(),
            email: [profile.email, profile.mail, profile.user_principal_name]
                .into_iter()
                .flatten()
                .find(|s| !s.trim().is_empty())
                .unwrap_or_default(),
            role: profile.role,
        })
    }
}
impl Person {
    pub fn known_name(&self) -> Option<&str> {
        [
            self.display_name.as_str(),
            self.friendly_name.as_str(),
            self.email.as_str(),
        ]
        .into_iter()
        .map(str::trim)
        .find(|s| !s.is_empty() && !crate::is_placeholder_name(&self.mri, s))
    }
    pub fn name(&self) -> &str {
        if !self.display_name.is_empty() {
            &self.display_name
        } else if !self.friendly_name.is_empty() {
            &self.friendly_name
        } else {
            &self.mri
        }
    }
}

impl<A: Authenticator> ChatClient<A> {
    pub async fn search_people(&self, query: &str) -> Result<Vec<Person>, Error> {
        let response: Value = self.service_json(Service::MiddleTier, "people", Method::POST,
            "beta/users/searchV2?includeDLs=false&includeBots=false&enableGuest=true&source=newChat", Some(json!(query)), HeaderMap::new()).await?;
        decode_value_list("people", response)
    }
    pub async fn profile_picture(&self, mri: &str, name: &str) -> Result<Vec<u8>, Error> {
        let path = format!(
            "beta/users/{}/profilepicturev2?displayname={}&size=HR96x96",
            enc(mri),
            enc(name)
        );
        let mut response = self
            .service_request(
                Service::ProfileImages,
                "profile picture",
                Method::GET,
                &path,
                None,
                HeaderMap::new(),
            )
            .await?;
        let limit = 2 * 1024 * 1024;
        if response
            .content_length()
            .is_some_and(|size| size > limit as u64)
        {
            return Err(Error::Media(t("chat.csa.profile_picture_is_too_large")));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > limit {
                return Err(Error::Media(t("chat.csa.profile_picture_is_too_large")));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    pub async fn my_profile(&self) -> Result<Person, Error> {
        let response: Value = self.service_json(Service::MiddleTier, "my profile", Method::GET,
            "beta/users/me/?skypeTeamsInfo=true&forceRefresh=false&ephemeralMeProfile=false&ggEnabled=false",
            None, HeaderMap::new()).await?;
        let value = response.get("value").cloned().unwrap_or(response);
        decode_csa("my profile", value)
    }
    pub async fn people_profiles(&self, mris: &[String]) -> Result<Vec<Person>, Error> {
        let regular = self.short_people_profiles(mris).await;
        let mut profiles = regular.as_ref().ok().cloned().unwrap_or_default();
        let unresolved: Vec<_> = mris
            .iter()
            .filter(|mri| {
                !profiles
                    .iter()
                    .any(|p| p.mri.eq_ignore_ascii_case(mri) && p.known_name().is_some())
            })
            .cloned()
            .collect();
        if unresolved.is_empty() {
            return Ok(profiles);
        }
        // Teams resolves external tenant users, absent from the local directory, via the federated service.
        let federated = self.federated_people_profiles(&unresolved).await;
        if let Ok(federated) = &federated {
            for person in federated.iter().filter(|p| p.known_name().is_some()) {
                if let Some(existing) = profiles
                    .iter_mut()
                    .find(|p| p.mri.eq_ignore_ascii_case(&person.mri))
                {
                    *existing = person.clone();
                } else {
                    profiles.push(person.clone());
                }
            }
        }
        if profiles.iter().any(|p| p.known_name().is_some()) || regular.is_ok() {
            Ok(profiles)
        } else {
            federated
        }
    }
    async fn short_people_profiles(&self, mris: &[String]) -> Result<Vec<Person>, Error> {
        let response:Value=self.service_json(Service::MiddleTier,"profiles",Method::POST,
            "beta/users/fetchShortProfile?isMailAddress=false&enableGuest=true&skypeTeamsInfo=true&canBeSmtpAddress=false&includeIBBarredUsers=true&includeDisabledAccounts=true",Some(json!(mris)),HeaderMap::new()).await?;
        decode_value_list("profiles", response)
    }
    async fn federated_people_profiles(&self, mris: &[String]) -> Result<Vec<Person>, Error> {
        let response: Value = self
            .service_json(
                Service::MiddleTier,
                "federated profiles",
                Method::POST,
                "beta/users/fetchFederated?edEnabled=false&includeDisabledAccounts=true",
                Some(json!(mris)),
                HeaderMap::new(),
            )
            .await?;
        decode_value_list("federated profiles", response)
    }
}

fn csa_role<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    match Value::deserialize(deserializer)? {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s)),
        Value::Number(n) => Ok(Some(match n.as_i64() {
            Some(0) => "Undefined".into(),
            Some(1) => "User".into(),
            Some(2) => "Admin".into(),
            Some(3) => "Guest".into(),
            Some(4) => "Shadow".into(),
            _ => format!("Unknown({n})"),
        })),
        _ => Err(serde::de::Error::custom(
            "role doit être un entier ou un libellé",
        )),
    }
}
