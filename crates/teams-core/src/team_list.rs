//! Teams sidebar preferences: service visibility, roaming order, local expansion.
use super::view::matches_search;
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use teams_i18n::tf;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TeamOrder {
    pub team_id: String,
    pub order: i64,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum TeamSort {
    #[default]
    Teams,
    Alphabetical,
}
#[derive(Debug, Clone, uniffi::Enum)]
pub enum TeamListAction {
    Sort {
        sort: TeamSort,
    },
    HiddenTeams {
        expanded: bool,
    },
    HiddenChannels {
        team: String,
        expanded: bool,
    },
    Show {
        team: String,
        channel: Option<String>,
        shown: bool,
    },
    Move {
        id: String,
        before: Option<String>,
    },
    Shift {
        id: String,
        up: bool,
    },
}
#[derive(Default)]
pub(crate) struct TeamListState {
    sort: TeamSort,
    show_hidden: bool,
    hidden_channels: HashSet<String>,
    initialized: HashSet<String>,
    shown: HashMap<String, bool>,
    pending: HashSet<String>,
    order: Option<Vec<TeamOrder>>,
    saving_order: bool,
    order_override: bool,
}
#[derive(Debug, Clone)]
pub(crate) enum TeamListMsg {
    Action(TeamListAction),
    ShownSaved(u64, String, Option<bool>, Result<(), String>),
    OrderSaved(u64, Vec<TeamOrder>, Result<(), String>),
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct TeamListControls {
    pub sort: TeamSort,
    pub shown_count: u32,
    pub hidden_count: u32,
    pub show_hidden: bool,
    pub saving_order: bool,
    pub order_available: bool,
}
pub(crate) fn flag(extra: &serde_json::Map<String, Value>, name: &str) -> Option<bool> {
    extra
        .get(name)
        .and_then(|v| v.as_bool().or_else(|| v.as_str()?.parse().ok()))
}
fn toggle<T: std::hash::Hash + Eq>(set: &mut HashSet<T>, value: T, on: bool) {
    if on {
        set.insert(value);
    } else {
        set.remove(&value);
    }
}
pub(crate) fn parse_order(value: &Value) -> Option<Vec<TeamOrder>> {
    if !value.is_object() {
        return None;
    }
    let value = value
        .get("teamsOrder")
        .or_else(|| value.get("properties")?.get("teamsOrder"));
    // A successfully read properties object without an order means the default order.
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Some(vec![]);
    };
    let mut order: Vec<TeamOrder> = if let Some(text) = value.as_str() {
        if text.trim().is_empty() {
            return Some(vec![]);
        }
        serde_json::from_str(text).ok()?
    } else {
        serde_json::from_value(value.clone()).ok()?
    };
    let mut seen = HashSet::new();
    order.sort_by_key(|item| item.order);
    order.retain(|item| !item.team_id.is_empty() && seen.insert(item.team_id.clone()));
    Some(order)
}
impl App {
    pub(super) fn first_shown_channel(&self) -> Option<(String, String)> {
        self.ordered_teams()
            .into_iter()
            .filter(|t| self.team_shown(t))
            .find_map(|t| {
                t.channels
                    .iter()
                    .find(|c| self.channel_shown(t, c))
                    .map(|c| (t.id.clone(), c.id.clone()))
            })
    }
    pub(super) fn reveal_channel(&mut self, team_id: &str, channel_id: &str) {
        self.expanded.insert(team_id.into());
        self.team_list.initialized.insert(team_id.into());
        let Some(team) = self.teams.iter().find(|t| t.id == team_id) else {
            return;
        };
        let hidden = !self.team_shown(team);
        let channel_hidden = team
            .channels
            .iter()
            .find(|c| c.id == channel_id)
            .is_some_and(|c| !self.channel_shown(team, c));
        if hidden {
            self.team_list.show_hidden = true;
        }
        if channel_hidden {
            self.team_list.hidden_channels.insert(team_id.into());
        }
    }
    pub(super) fn initialize_team_list(&mut self, order: Option<Vec<TeamOrder>>) {
        if !self.team_list.saving_order && !self.team_list.order_override && order.is_some() {
            self.team_list.order = order;
        }
        for team in &*self.teams {
            if self.team_list.initialized.insert(team.id.clone())
                && flag(&team.extra, "isCollapsed") == Some(false)
            {
                self.expanded.insert(team.id.clone());
            }
        }
    }
    fn visibility_key(team: &str, channel: Option<&str>) -> String {
        format!("{team}/{}", channel.unwrap_or(""))
    }
    pub(super) fn team_shown(&self, team: &Team) -> bool {
        self.team_list
            .shown
            .get(&Self::visibility_key(&team.id, None))
            .copied()
            .unwrap_or_else(|| {
                flag(&team.extra, "isShown")
                    .or_else(|| flag(&team.extra, "isHidden").map(|h| !h))
                    .or_else(|| flag(&team.extra, "isFavorite"))
                    .unwrap_or(true)
            })
    }
    pub(super) fn channel_shown(&self, team: &Team, channel: &Channel) -> bool {
        self.team_list
            .shown
            .get(&Self::visibility_key(&team.id, Some(&channel.id)))
            .copied()
            .unwrap_or_else(|| {
                flag(&channel.extra, "isShown")
                    .or_else(|| flag(&channel.extra, "isHidden").map(|h| !h))
                    .or_else(|| {
                        if channel.is_general {
                            flag(&team.extra, "isGeneralChannelFavorite")
                                .or_else(|| flag(&channel.extra, "isFavorite"))
                        } else {
                            flag(&channel.extra, "isFavorite")
                        }
                    })
                    .or_else(|| flag(&channel.extra, "isFavoriteByDefault"))
                    // A channel the user never joined waits among the hidden ones.
                    .or_else(|| flag(&channel.extra, "isUnjoinedChannel").map(|u| !u))
                    .unwrap_or(true)
            })
    }
    fn ordered_teams(&self) -> Vec<&Team> {
        let mut teams: Vec<_> = self.teams.iter().collect();
        if self.team_list.sort == TeamSort::Alphabetical {
            teams.sort_by_cached_key(|t| t.display_name.to_lowercase());
        } else if let Some(order) = &self.team_list.order {
            let positions: HashMap<_, _> = order
                .iter()
                .enumerate()
                .map(|(i, t)| (t.team_id.as_str(), i))
                .collect();
            teams.sort_by_key(|t| positions.get(t.id.as_str()).copied().unwrap_or(usize::MAX));
        }
        teams
    }
    pub(super) fn team_list_rows(&self) -> (Vec<view::TeamRow>, TeamListControls) {
        let searching = !self.search.trim().is_empty();
        let mut rows = Vec::new();
        let mut counts = [0usize; 2];
        for team in self.ordered_teams() {
            let hidden = !self.team_shown(team);
            let index = usize::from(hidden);
            counts[index] += 1;
            // Every team is listed, as in Teams; only the hidden ones wait behind their toggle.
            if !searching && hidden && !self.team_list.show_hidden {
                continue;
            }
            let team_match = matches_search(&team.display_name, &self.search);
            let mut channels = Vec::new();
            let mut channel_counts = [0usize; 2];
            for channel in &team.channels {
                if !team_match && !matches_search(&channel.display_name, &self.search) {
                    continue;
                }
                let channel_hidden = !self.channel_shown(team, channel);
                let bucket = usize::from(channel_hidden);
                channel_counts[bucket] += 1;
                if !searching
                    && channel_hidden
                    && !self.team_list.hidden_channels.contains(&team.id)
                {
                    continue;
                }
                channels.push(view::ChannelRow {
                    team: team.id.clone(),
                    id: channel.id.clone(),
                    name: channel.display_name.clone(),
                    selected: self.selected.as_ref().is_some_and(|s| {
                        matches!(s, Selection::Channel { team: t, channel: c } if t == &team.id && c == &channel.id)
                    }),
                    hidden: channel_hidden,
                    kind: channel.channel_type.clone(),
                    archived: channel.is_archived,
                    pending: self
                        .team_list
                        .pending
                        .contains(&Self::visibility_key(&team.id, Some(&channel.id))),
                    muted: self.channel_muted(&channel.id),
                    unread: channel.is_unread(&self.me),
                });
            }
            if !team_match && channels.is_empty() {
                continue;
            }
            let expanded = self.expanded.contains(&team.id);
            rows.push(view::TeamRow {
                id: team.id.clone(),
                name: team.display_name.clone(),
                description: team.description.clone(),
                archived: team.is_archived,
                hidden,
                pending: self
                    .team_list
                    .pending
                    .contains(&Self::visibility_key(&team.id, None)),
                expanded,
                show_channels: expanded || searching,
                unread: team
                    .channels
                    .iter()
                    .any(|c| self.channel_shown(team, c) && c.is_unread(&self.me)),
                channels,
                loading: self.channels_loading.contains(&team.id),
                failed: self.channel_errors.contains_key(&team.id),
                hidden_channel_count: channel_counts[1] as u32,
                show_hidden_channels: searching
                    || self.team_list.hidden_channels.contains(&team.id),
            });
        }
        let controls = TeamListControls {
            sort: self.team_list.sort,
            shown_count: counts[0] as u32,
            hidden_count: counts[1] as u32,
            show_hidden: self.team_list.show_hidden,
            saving_order: self.team_list.saving_order,
            order_available: self.team_list.order.is_some(),
        };
        (rows, controls)
    }
    pub(super) fn visible_team_ids(&self) -> HashSet<String> {
        if !self.search.trim().is_empty() {
            return self.teams.iter().map(|t| t.id.clone()).collect();
        }
        self.team_list_rows()
            .0
            .into_iter()
            .filter(|r| r.show_channels)
            .map(|r| r.id)
            .collect()
    }
    pub(super) fn update_team_list(&mut self, msg: TeamListMsg) -> Task<Msg> {
        match msg {
            TeamListMsg::ShownSaved(g, key, old, result) => {
                if g != self.generation {
                    return Task::none();
                }
                self.team_list.pending.remove(&key);
                if let Err(e) = result {
                    if let Some(old) = old {
                        self.team_list.shown.insert(key, old);
                    } else {
                        self.team_list.shown.remove(&key);
                    }
                    *self.error = Some(tf!("core.team_list.visibility_not_saved", error = e));
                }
            }
            TeamListMsg::OrderSaved(g, previous, result) => {
                if g != self.generation {
                    return Task::none();
                }
                self.team_list.saving_order = false;
                if let Err(e) = result {
                    self.team_list.order = Some(previous);
                    self.team_list.order_override = false;
                    *self.error = Some(tf!("core.team_list.team_order_not_saved", error = e));
                }
            }
            TeamListMsg::Action(action) => match action {
                TeamListAction::Sort { sort } => self.team_list.sort = sort,
                TeamListAction::HiddenTeams { expanded } => self.team_list.show_hidden = expanded,
                TeamListAction::HiddenChannels { team, expanded } => {
                    toggle(&mut self.team_list.hidden_channels, team, expanded)
                }
                TeamListAction::Show {
                    team,
                    channel,
                    shown,
                } => {
                    let Some(client) = self.client.clone() else {
                        return Task::none();
                    };
                    let Some(t) = self.teams.iter().find(|t| t.id == team) else {
                        return Task::none();
                    };
                    let general = if let Some(id) = &channel {
                        let Some(c) = t.channels.iter().find(|c| &c.id == id) else {
                            return Task::none();
                        };
                        c.is_general
                    } else {
                        false
                    };
                    let key = Self::visibility_key(&team, channel.as_deref());
                    if !self.team_list.pending.insert(key.clone()) {
                        return Task::none();
                    }
                    let old = self.team_list.shown.insert(key.clone(), shown);
                    let generation = self.generation;
                    return Task::perform(
                        async move {
                            if general {
                                client
                                    .set_conversation_property(
                                        &team,
                                        "isGeneralChannelFavorite",
                                        json!(shown.to_string()),
                                    )
                                    .await
                                    .map(drop)
                            } else {
                                client
                                    .set_conversation_shown(
                                        channel.as_deref().unwrap_or(&team),
                                        shown,
                                    )
                                    .await
                            }
                            .map_err(|e| e.to_string())
                        },
                        move |r| Msg::TeamList(TeamListMsg::ShownSaved(generation, key, old, r)),
                    );
                }
                TeamListAction::Shift { id, up } => {
                    let Some(team) = self.teams.iter().find(|t| t.id == id) else {
                        return Task::none();
                    };
                    let shown = self.team_shown(team);
                    let peers: Vec<_> = self
                        .ordered_teams()
                        .into_iter()
                        .filter(|t| self.team_shown(t) == shown)
                        .map(|t| t.id.clone())
                        .collect();
                    let Some(index) = peers.iter().position(|t| t == &id) else {
                        return Task::none();
                    };
                    if (up && index == 0) || (!up && index + 1 == peers.len()) {
                        return Task::none();
                    }
                    let before = if up {
                        Some(peers[index - 1].clone())
                    } else {
                        peers.get(index + 2).cloned()
                    };
                    return self.update_team_list(TeamListMsg::Action(TeamListAction::Move {
                        id,
                        before,
                    }));
                }
                TeamListAction::Move { id, before } => {
                    let (Some(client), Some(previous)) =
                        (self.client.clone(), self.team_list.order.clone())
                    else {
                        return Task::none();
                    };
                    if self.team_list.saving_order
                        || before.as_ref() == Some(&id)
                        || !self.teams.iter().any(|t| t.id == id)
                        || before
                            .as_ref()
                            .is_some_and(|b| !self.teams.iter().any(|t| &t.id == b))
                    {
                        return Task::none();
                    }
                    let mut ids: Vec<_> = previous.iter().map(|t| t.team_id.clone()).collect();
                    for team in &*self.teams {
                        if !ids.contains(&team.id) {
                            ids.push(team.id.clone());
                        }
                    }
                    ids.retain(|t| t != &id);
                    let position = before
                        .and_then(|b| ids.iter().position(|t| t == &b))
                        .unwrap_or(ids.len());
                    ids.insert(position, id);
                    let order: Vec<_> = ids
                        .into_iter()
                        .enumerate()
                        .map(|(i, team_id)| TeamOrder {
                            team_id,
                            order: i as i64,
                        })
                        .collect();
                    self.team_list.order = Some(order.clone());
                    self.team_list.sort = TeamSort::Teams;
                    self.team_list.saving_order = true;
                    self.team_list.order_override = true;
                    let generation = self.generation;
                    return Task::perform(
                        async move {
                            client
                                .set_user_property(
                                    "teamsOrder",
                                    json!(serde_json::to_string(&order).expect("teams order")),
                                )
                                .await
                                .map_err(|e| e.to_string())
                        },
                        move |r| Msg::TeamList(TeamListMsg::OrderSaved(generation, previous, r)),
                    );
                }
            },
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn team(id: &str, shown: bool) -> Team {
        serde_json::from_value(json!({"id":id,"displayName":id,"isFavorite":shown,"isCollapsed":false,"isGeneralChannelFavorite":false,"channels":
            (0..9).map(|i|json!({"id":format!("{id}-{i}"),"displayName":format!("Canal {i}"),"isFavorite":i<7,"channelType":"Standard"})).collect::<Vec<_>>()
        })).unwrap()
    }
    fn app() -> App {
        let mut app = App::empty();
        *app.nav = Nav::Teams;
        *app.teams = (0..12)
            .map(|i| team(&format!("team-{i:02}"), i < 10))
            .collect();
        for t in &mut *app.teams {
            normalize_team(t);
        }
        app.initialize_team_list(Some(vec![]));
        app
    }
    fn action(app: &mut App, action: TeamListAction) {
        let _ = app.update_team_list(TeamListMsg::Action(action));
    }
    #[test]
    fn unjoined_channels_wait_among_the_hidden_ones() {
        let app = App::empty();
        let team: Team = serde_json::from_value(json!({"id":"t","displayName":"T","channels":[
            {"id":"a","displayName":"A","channelType":"Standard"},
            {"id":"b","displayName":"B","channelType":"Standard","isUnjoinedChannel":true},
            {"id":"c","displayName":"C","channelType":"Standard","isUnjoinedChannel":true,"isFavoriteByDefault":true}
        ]}))
        .unwrap();
        let shown: Vec<_> = team.channels.iter().map(|c| app.channel_shown(&team, c)).collect();
        assert_eq!(shown, [true, false, true]);
    }
    #[test]
    fn lists_every_shown_team_and_channel_and_keeps_hidden_ones_behind_toggles() {
        let mut app = app();
        let (rows, c) = app.team_list_rows();
        assert_eq!((c.shown_count, c.hidden_count), (10, 2));
        assert_eq!(rows.len(), 10);
        assert_eq!(rows[0].channels.len(), 7);
        assert_eq!(rows[0].hidden_channel_count, 3);
        assert!(
            !rows[0].channels.iter().any(|c| c.id == "team-00"),
            "the independently hidden general channel stays hidden"
        );
        action(&mut app, TeamListAction::HiddenTeams { expanded: true });
        assert_eq!(app.team_list_rows().0.len(), 12);
        action(
            &mut app,
            TeamListAction::HiddenChannels {
                team: "team-00".into(),
                expanded: true,
            },
        );
        let rows = app.team_list_rows().0;
        assert_eq!(rows[0].channels.iter().filter(|c| c.hidden).count(), 3);
        assert_eq!(rows[0].channels.len(), 10);
        app.teams[0].extra.insert("isFollowed".into(), json!(false));
        assert!(app.team_shown(&app.teams[0]));
    }
    #[test]
    fn channels_are_unread_past_their_consumption_horizon_until_opened() {
        let mut app = app();
        *app.me = "8:orgid:me".into();
        let last = |from: &str, at: &str| json!({"id": "1700000002000", "from": from, "originalArrivalTime": at, "messageType": "RichText/Html"});
        let channel = &mut app.teams[0].channels[1];
        channel.extra.insert(
            "consumptionHorizon".into(),
            json!("1700000001000;1700000001000;1"),
        );
        channel.extra.insert(
            "lastMessage".into(),
            last(
                "https://x/contacts/8:orgid:other",
                "2023-11-14T22:13:22.000Z",
            ),
        );
        app.teams[1].channels[1].extra.insert(
            "lastMessage".into(),
            last("8:orgid:me", "2023-11-14T22:13:22.000Z"),
        );
        app.teams[2].channels[1].extra.insert(
            "userConsumptionHorizon".into(),
            json!({"originalArrivalTime": 1700000000000_i64, "timeStamp": 1, "clientMessageId": "42"}),
        );
        app.expanded.clear();
        let id = app.teams[0].channels[1].id.clone();
        let rows = app.team_list_rows().0;
        assert!(rows[0].unread);
        assert!(rows[0].channels.iter().all(|c| c.unread == (c.id == id)));
        assert!(
            !rows[1].unread,
            "my own last message leaves the channel read"
        );
        assert!(rows[2].unread, "marked as unread");
        assert!(!rows[3].unread);

        let selection = Selection::Channel {
            team: "team-00".into(),
            channel: id,
        };
        let _ = app.handle(Msg::MarkRead(selection, "1700000002000".into()));
        assert!(!app.team_list_rows().0[0].unread);
    }
    #[test]
    fn search_finds_hidden_channels_without_changing_saved_visibility_or_expansion() {
        let mut app = app();
        app.expanded.clear();
        *app.search = "Canal 8".into();
        let (rows, _) = app.team_list_rows();
        assert_eq!(rows.len(), 12);
        assert!(
            rows.iter()
                .all(|r| r.show_channels && r.channels.len() == 1 && r.channels[0].hidden)
        );
        app.search.clear();
        assert!(app.team_list_rows().0.iter().all(|r| !r.show_channels));
        let visible = app.visible_team_ids();
        assert!(visible.is_empty());
        let _ = app.handle(Msg::WorkspaceLoaded(
            0,
            Ok(workspace(Startup {
                teams: app.teams.clone(),
                ..Default::default()
            })),
        ));
        assert!(
            app.expanded.is_empty(),
            "refresh must not reopen a team after collapsing all"
        );
    }
    #[test]
    fn roaming_order_and_manual_moves_preserve_unknown_teams_and_roll_back_failures() {
        let mut app = app();
        *app.client = Some(ChatClient::new(Arc::new(AppAuthenticator::Demo(
            StaticAuthenticator::new(),
        ))));
        let order=parse_order(&json!({"teamsOrder":"[{\"teamId\":\"unknown\",\"order\":0},{\"teamId\":\"team-03\",\"order\":1},{\"teamId\":\"team-00\",\"order\":2}]"})).unwrap();
        app.initialize_team_list(Some(order.clone()));
        assert_eq!(app.team_list_rows().0[0].id, "team-03");
        action(
            &mut app,
            TeamListAction::Move {
                id: "team-00".into(),
                before: Some("team-03".into()),
            },
        );
        assert_eq!(app.team_list_rows().0[0].id, "team-00");
        assert!(
            app.team_list
                .order
                .as_ref()
                .unwrap()
                .iter()
                .any(|t| t.team_id == "unknown")
        );
        app.initialize_team_list(Some(order.clone()));
        assert_eq!(
            app.team_list_rows().0[0].id,
            "team-00",
            "an in-flight refresh must not override a move"
        );
        let _ = app.update_team_list(TeamListMsg::OrderSaved(0, order, Err("offline".into())));
        assert_eq!(app.team_list_rows().0[0].id, "team-03");
        assert!(!app.team_list.saving_order);
        assert!(app.error.is_some());
        action(
            &mut app,
            TeamListAction::Sort {
                sort: TeamSort::Alphabetical,
            },
        );
        assert_eq!(app.team_list_rows().0[0].id, "team-00");
    }
    #[test]
    fn failed_visibility_change_restores_state_and_late_results_do_not_cross_logout() {
        let mut app = app();
        *app.client = Some(ChatClient::new(Arc::new(AppAuthenticator::Demo(
            StaticAuthenticator::new(),
        ))));
        let key = App::visibility_key("team-00", None);
        action(
            &mut app,
            TeamListAction::Show {
                team: "team-00".into(),
                channel: None,
                shown: false,
            },
        );
        assert!(!app.team_shown(&app.teams[0]));
        let _ = app.update_team_list(TeamListMsg::ShownSaved(
            0,
            key.clone(),
            None,
            Err("offline".into()),
        ));
        assert!(app.team_shown(&app.teams[0]));
        let _ = app.handle(Msg::SignOut);
        let _ = app.update_team_list(TeamListMsg::ShownSaved(0, key, None, Err("late".into())));
        assert!(app.error.is_none());
        assert!(app.team_list.shown.is_empty());
    }
}
