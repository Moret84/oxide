//! Teams message emotions, including JSON-string properties and summary-only counts.
use crate::Message;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    pub key: String,
    pub count: u64,
    pub users: Vec<String>,
    pub mine: bool,
}
fn decoded(value: &Value) -> Value {
    if let Some(s) = value.as_str() {
        serde_json::from_str(s).unwrap_or(Value::Null)
    } else {
        value.clone()
    }
}
impl Message {
    pub fn reactions(&self, me: &str) -> Vec<Reaction> {
        let mut reactions: BTreeMap<String, Reaction> = BTreeMap::new();
        if let Some(raw) = self.properties.get("emotions") {
            let value = decoded(raw);
            for item in value.as_array().into_iter().flatten() {
                let Some(key) = item["key"]
                    .as_str()
                    .filter(|s| !s.is_empty() && !matches!(*s, "follow" | "star"))
                else {
                    continue;
                };
                let users: BTreeSet<String> = item["users"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|u| u.as_str().or_else(|| u["mri"].as_str()))
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
                let r = reactions.entry(key.into()).or_insert_with(|| Reaction {
                    key: key.into(),
                    count: 0,
                    users: vec![],
                    mine: false,
                });
                r.users.extend(users);
                r.users.sort();
                r.users.dedup();
                r.count = r.users.len() as u64;
                r.mine = r.users.iter().any(|u| u.eq_ignore_ascii_case(me));
            }
        }
        if let Some(raw) = self.extra.get("annotationsSummary") {
            let summary = decoded(raw);
            let emotions = decoded(&summary["emotions"]);
            for (key, count) in emotions.as_object().into_iter().flatten() {
                if matches!(key.as_str(), "follow" | "star") {
                    continue;
                }
                let count = count
                    .as_u64()
                    .or_else(|| count.as_str()?.parse().ok())
                    .unwrap_or(0);
                let r = reactions.entry(key.clone()).or_insert_with(|| Reaction {
                    key: key.clone(),
                    count: 0,
                    users: vec![],
                    mine: false,
                });
                r.count = r.count.max(count);
            }
        }
        reactions.into_values().filter(|r| r.count > 0).collect()
    }
    /// Apply an acknowledged single-reaction change while preserving other users/counts.
    pub fn apply_reaction(&mut self, me: &str, key: &str, remove: bool) {
        let mut reactions = self.reactions(me);
        for r in &mut reactions {
            if r.mine && (!remove || r.key == key) {
                r.users.retain(|u| !u.eq_ignore_ascii_case(me));
                r.count = r.count.saturating_sub(1);
                r.mine = false;
            }
        }
        if !remove {
            if let Some(r) = reactions.iter_mut().find(|r| r.key == key) {
                r.users.push(me.into());
                r.count += 1;
                r.mine = true;
            } else {
                reactions.push(Reaction {
                    key: key.into(),
                    count: 1,
                    users: vec![me.into()],
                    mine: true,
                });
            }
        }
        let mut summary = serde_json::Map::new();
        let preserved: Vec<Value> = self
            .properties
            .get("emotions")
            .map(decoded)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|r| matches!(r["key"].as_str(), Some("follow" | "star")))
            .collect();
        let mut emotions:Vec<_>=reactions.into_iter().filter(|r|r.count>0).map(|r|{
            summary.insert(r.key.clone(),json!(r.count));
            json!({"key":r.key,"users":r.users.into_iter().map(|mri|json!({"mri":mri})).collect::<Vec<_>>()})
        }).collect();
        emotions.extend(preserved);
        self.properties.insert("emotions".into(), json!(emotions));
        let mut annotations = self
            .extra
            .get("annotationsSummary")
            .map(decoded)
            .unwrap_or_else(|| json!({}));
        if !annotations.is_object() {
            annotations = json!({});
        }
        if let Some(previous) = annotations.get("emotions").map(decoded) {
            for key in ["follow", "star"] {
                if let Some(value) = previous.get(key) {
                    summary.insert(key.into(), value.clone());
                }
            }
        }
        annotations["emotions"] = json!(summary);
        self.extra.insert("annotationsSummary".into(), annotations);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merges_summary_and_rosters_then_replaces_only_current_users_reaction() {
        let mut m:Message=serde_json::from_value(json!({"properties":{"emotions":json!([{"key":"like","users":[{"mri":"me"},{"mri":"peer"},{"mri":"peer"}]},{"key":"star","users":[{"mri":"me"}]}]).to_string()},"annotationsSummary":{"emotions":{"like":5,"heart":2}}})).unwrap();
        assert_eq!(m.reactions("me").len(), 2);
        assert!(
            m.reactions("me")
                .iter()
                .find(|r| r.key == "like")
                .unwrap()
                .mine
        );
        m.apply_reaction("me", "heart", false);
        assert_eq!(
            m.reactions("me")
                .iter()
                .find(|r| r.key == "like")
                .unwrap()
                .count,
            4
        );
        assert_eq!(
            m.reactions("me")
                .iter()
                .find(|r| r.key == "heart")
                .unwrap()
                .count,
            3
        );
        m.apply_reaction("me", "heart", true);
        assert_eq!(
            m.reactions("me")
                .iter()
                .find(|r| r.key == "heart")
                .unwrap()
                .count,
            2
        );
        assert!(m.reactions("me").iter().all(|r| !r.mine));
    }
}
