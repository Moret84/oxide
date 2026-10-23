//! The app behind a bot's messages (`fromAppMetadata.id`): Teams shows its catalog name, localized, and its icon.
use crate::app::*;
use serde_json::Value;
use teams_chat::AppDefinition;

/// One app id's lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Lookup {
    /// Asked for; the bots (MRI, name) whose avatar waits for the answer.
    Pending(Vec<(String, String)>),
    Found(AppDefinition),
    /// Not in the catalog, or the request failed: the message's own name and the directory photo.
    Missing,
}
/// `fromAppMetadata.id`, the app to look up.
pub(crate) fn app_id(m: &Message) -> Option<String> {
    metadata(m, "id")
}
/// A field of a bot message's `properties.fromAppMetadata`: an object or a JSON string `{id, name}`.
pub(crate) fn metadata(m: &Message, key: &str) -> Option<String> {
    if !m.sender_mri().starts_with("28:") {
        return None;
    }
    let meta = match m.properties.get("fromAppMetadata")? {
        Value::String(s) => serde_json::from_str(s).ok()?,
        other => other.clone(),
    };
    meta.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
/// As the web client checks it: the definition counts only if one of its bots sent the message.
pub(crate) fn sent_by(app: &AppDefinition, mri: &str) -> bool {
    let bot = mri.strip_prefix("28:").unwrap_or(mri);
    app.bot_ids.iter().any(|id| id.eq_ignore_ascii_case(bot))
}

impl App {
    /// The catalog's definition of the app that sent `m`, once looked up.
    pub(crate) fn bot_app(&self, m: &Message) -> Option<&AppDefinition> {
        match self.bot_apps.get(&app_id(m)?)? {
            Lookup::Found(app) if sent_by(app, m.sender_mri()) => Some(app),
            _ => None,
        }
    }
    /// A bot's avatar: its app's icon, looked up first if needed; `None` when the directory photo should be used.
    pub(crate) fn load_bot_avatar(&mut self, mri: &str, name: &str) -> Option<Task<Msg>> {
        let app_id = self
            .conv
            .all_messages()
            .filter(|m| m.sender_mri().eq_ignore_ascii_case(mri))
            .find_map(app_id)?;
        match self.bot_apps.get_mut(&app_id) {
            Some(Lookup::Found(app)) => {
                let icon = app.icon.clone().filter(|_| sent_by(app, mri))?;
                self.avatars
                    .lock()
                    .unwrap()
                    .request_icon(mri.to_owned(), icon);
                self.mark(Parts::AVATARS);
                Some(Task::none())
            }
            Some(Lookup::Missing) => None,
            Some(Lookup::Pending(waiting)) => {
                waiting.push((mri.to_owned(), name.to_owned()));
                Some(Task::none())
            }
            None => {
                let client = self.client.clone()?;
                self.bot_apps.insert(
                    app_id.clone(),
                    Lookup::Pending(vec![(mri.to_owned(), name.to_owned())]),
                );
                let generation = self.generation;
                let language = crate::emoji::locale();
                Some(Task::perform(
                    async move {
                        let ids = std::slice::from_ref(&app_id);
                        let found = client.app_definitions(ids, language).await;
                        (app_id, found.map_err(|e| e.to_string()))
                    },
                    move |(id, result)| Msg::BotAppLoaded {
                        generation,
                        id,
                        result,
                    },
                ))
            }
        }
    }
    /// Names the app's messages and gives its waiting bots their icon, or their directory photo when it has none.
    pub(crate) fn bot_app_loaded(
        &mut self,
        id: String,
        result: Result<Vec<AppDefinition>, String>,
    ) -> Task<Msg> {
        let found = result
            .ok()
            .and_then(|apps| apps.into_iter().find(|a| a.id.eq_ignore_ascii_case(&id)));
        let lookup = found.map_or(Lookup::Missing, Lookup::Found);
        let Some(Lookup::Pending(waiting)) = self.bot_apps.insert(id, lookup.clone()) else {
            return Task::none();
        };
        let mut avatars = self.avatars.lock().unwrap();
        for (mri, name) in waiting {
            match &lookup {
                Lookup::Found(app) if sent_by(app, &mri) && app.icon.is_some() => {
                    avatars.request_icon(mri, app.icon.clone().unwrap_or_default())
                }
                _ => avatars.request(mri.clone(), mri, name),
            }
        }
        drop(avatars);
        self.mark(Parts::AVATARS);
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn app_ids_come_from_bot_messages_only() {
        let mut m = Message {
            from: "https://x/v1/users/ME/contacts/28:bot-1".into(),
            ..Default::default()
        };
        m.properties.insert(
            "fromAppMetadata".into(),
            serde_json::json!("{\"id\":\"app-1\",\"name\":\"Workflows\"}"),
        );
        assert_eq!(app_id(&m).as_deref(), Some("app-1"));
        let app = AppDefinition {
            id: "app-1".into(),
            name: "Flux de travail".into(),
            icon: None,
            bot_ids: vec!["BOT-1".into()],
        };
        assert!(sent_by(&app, "28:bot-1"));
        assert!(!sent_by(&app, "28:bot-2"));
        m.from = "https://x/v1/users/ME/contacts/8:orgid:someone".into();
        assert_eq!(app_id(&m), None);
    }
    #[test]
    fn a_found_app_names_its_bots_messages_in_the_catalog_language() {
        let mut app = App::empty();
        let mut m = Message {
            from: "https://x/v1/users/ME/contacts/28:bot-1".into(),
            imdisplayname: Some("Workflows".into()),
            ..Default::default()
        };
        m.properties.insert(
            "fromAppMetadata".into(),
            serde_json::json!({"id": "app-1", "name": "Workflows"}),
        );
        assert_eq!(app.sender_name(&m), "Workflows");
        app.bot_apps.insert(
            "app-1".into(),
            Lookup::Pending(vec![("28:bot-1".into(), "Workflows".into())]),
        );
        let found = AppDefinition {
            id: "app-1".into(),
            name: "Flux de travail".into(),
            icon: Some("https://cdn.test/workflows.png".into()),
            bot_ids: vec!["bot-1".into()],
        };
        let generation = app.generation;
        let _ = app.update(Msg::BotAppLoaded {
            generation,
            id: "app-1".into(),
            result: Ok(vec![found]),
        });
        assert_eq!(app.sender_name(&m), "Flux de travail");
        // Another bot claiming the app keeps its own name.
        m.from = "https://x/v1/users/ME/contacts/28:bot-2".into();
        assert_eq!(app.sender_name(&m), "Workflows");
    }
}
