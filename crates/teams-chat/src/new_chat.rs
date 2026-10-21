//! Chat creation through the chat service's thread endpoint.
use crate::people::Person;
use crate::{ChatClient, Error, Member, NewThread};
use http::{HeaderMap, Method};
use serde_json::{Map, Value, json};
use teams_auth::{Authenticator, Service};
use teams_i18n::t;

impl<A: Authenticator> ChatClient<A> {
    /// Teams returns the new chat ID in Location, frequently with no JSON body.
    pub async fn create_chat(
        &self,
        me: &str,
        people: &[Person],
        title: &str,
    ) -> Result<String, Error> {
        let mut members = vec![Member {
            id: me.into(),
            role: Some("Admin".into()),
            ..Default::default()
        }];
        for p in people {
            if !p.mri.is_empty()
                && !p.mri.eq_ignore_ascii_case(me)
                && !members.iter().any(|m| m.id.eq_ignore_ascii_case(&p.mri))
            {
                members.push(Member {
                    id: p.mri.clone(),
                    role: Some("Admin".into()),
                    ..Default::default()
                });
            }
        }
        if members.len() < 2 {
            return Err(Error::Auth(teams_auth::AuthError::Failed(
                t("chat.csa.choose_at_least_one_recipient").into(),
            )));
        }
        let mut properties = Map::from_iter([
            ("threadType".into(), json!("chat")),
            ("chatFilesIndexId".into(), json!(2)),
        ]);
        if members.len() == 2 {
            properties.insert("fixedRoster".into(), json!(true));
            properties.insert("uniquerosterthread".into(), json!(true));
        }
        if !title.trim().is_empty() {
            properties.insert("topic".into(), json!(title.trim()));
        }
        let body = serde_json::to_value(NewThread {
            members,
            properties,
        })
        .expect("thread serialization");
        let response = self
            .service_request(
                Service::ChatService,
                "createChat",
                Method::POST,
                "v1/threads",
                Some(body),
                HeaderMap::new(),
            )
            .await?;
        if let Some(location) = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
        {
            let id = location
                .split('?')
                .next()
                .unwrap_or(location)
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or_default();
            if !id.is_empty() {
                return Ok(percent_encoding::percent_decode_str(id)
                    .decode_utf8_lossy()
                    .into_owned());
            }
        }
        let value: Value = response.json().await?;
        value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                Error::Auth(teams_auth::AuthError::Failed(
                    t("chat.csa.the_server_didn_t_return_the").into(),
                ))
            })
    }
}
