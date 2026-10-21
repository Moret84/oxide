//! Creating teams and channels, joining teams (gallery, code) and leaving them.
use super::*;
use teams_chat::{GalleryPage, GalleryTeam, NewTeam, TeamVisibility};
use teams_i18n::{t, tf};

/// The web client searches the gallery past two characters only.
const MIN_QUERY: usize = 3;
const TEAM_NAME_MAX: usize = 256;
const CHANNEL_NAME_MAX: usize = 50;
const DESCRIPTION_MAX: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum NewTeamVisibility {
    Private,
    Public,
    OrgWide,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum GalleryJoin {
    None,
    Joining,
    Joined,
    /// A private team: its owners must accept.
    Requested,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct GalleryTeamView {
    pub group_id: String,
    pub name: String,
    pub description: String,
    pub private: bool,
    pub known_members: u32,
    pub join: GalleryJoin,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct GalleryView {
    pub query: String,
    pub teams: Vec<GalleryTeamView>,
    pub loading: bool,
    pub has_more: bool,
    pub joining_code: bool,
    /// The code sent last was accepted, but the team's owners must approve.
    pub code_pending: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct JoinCodeView {
    pub team_id: String,
    pub team_name: String,
    pub code: Option<String>,
    pub loading: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct TeamsAdminView {
    pub gallery: Option<GalleryView>,
    pub join_code: Option<JoinCodeView>,
    /// A team or channel is being created.
    pub creating: bool,
    /// The last creation went through: the dialog can close.
    pub created: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum TeamsAdminAction {
    OpenGallery,
    CloseGallery,
    GallerySearch {
        query: String,
    },
    GalleryMore,
    JoinTeam {
        group_id: String,
    },
    JoinWithCode {
        code: String,
    },
    CreateTeam {
        name: String,
        description: String,
        visibility: NewTeamVisibility,
    },
    CreateChannel {
        team_id: String,
        name: String,
        description: String,
        private: bool,
        show_for_everyone: bool,
    },
    LeaveTeam {
        team_id: String,
    },
    ShowJoinCode {
        team_id: String,
    },
    ResetJoinCode,
    CloseJoinCode,
    DismissError,
}

#[derive(Debug, Clone)]
pub(crate) enum TeamsAdminMsg {
    Action(TeamsAdminAction),
    Gallery(u64, u64, bool, Result<GalleryPage, String>),
    Joined(u64, String, Result<bool, String>),
    CodeJoined(u64, Result<Option<String>, String>),
    TeamCreated(u64, Result<String, String>),
    ChannelCreated(u64, String, Result<String, String>),
    Left(u64, String, Result<Option<String>, String>),
    JoinCode(u64, String, Result<String, String>),
}

#[derive(Default)]
pub(crate) struct TeamsAdminState {
    pub view: TeamsAdminView,
    gallery: Option<Gallery>,
    join_code: Option<JoinCodeView>,
    creating: bool,
    created: u32,
    error: Option<String>,
    /// Discards gallery answers to an earlier query.
    request: u64,
}
#[derive(Default)]
struct Gallery {
    query: String,
    teams: Vec<GalleryTeam>,
    joins: HashMap<String, GalleryJoin>,
    skip_token: Option<String>,
    loading: bool,
    joining_code: bool,
    code_pending: bool,
}

impl App {
    pub(crate) fn update_teams_admin(&mut self, msg: TeamsAdminMsg) -> Task<Msg> {
        let task = self.teams_admin_msg(msg);
        self.refresh_teams_admin_view();
        task
    }
    fn teams_admin_msg(&mut self, msg: TeamsAdminMsg) -> Task<Msg> {
        let generation = self.generation;
        let stale = match &msg {
            TeamsAdminMsg::Action(_) => false,
            TeamsAdminMsg::Gallery(g, ..)
            | TeamsAdminMsg::Joined(g, ..)
            | TeamsAdminMsg::CodeJoined(g, _)
            | TeamsAdminMsg::TeamCreated(g, _)
            | TeamsAdminMsg::ChannelCreated(g, ..)
            | TeamsAdminMsg::Left(g, ..)
            | TeamsAdminMsg::JoinCode(g, ..) => *g != generation,
        };
        if stale {
            return Task::none();
        }
        match msg {
            TeamsAdminMsg::Action(action) => self.teams_admin_action(action),
            TeamsAdminMsg::Gallery(_, request, append, result) => {
                let admin = &mut *self.teams_admin;
                let Some(gallery) = &mut admin.gallery else {
                    return Task::none();
                };
                if request != admin.request {
                    return Task::none();
                }
                gallery.loading = false;
                match result {
                    Ok(page) => {
                        let joined: HashSet<String> = self
                            .teams
                            .iter()
                            .filter_map(|t| t.group_id().map(str::to_owned))
                            .collect();
                        if !append {
                            gallery.teams.clear();
                        }
                        for team in page.teams {
                            if !joined.contains(&team.group_id)
                                && !gallery.teams.iter().any(|t| t.group_id == team.group_id)
                            {
                                gallery.teams.push(team);
                            }
                        }
                        gallery.skip_token = page.skip_token;
                    }
                    Err(e) => admin.error = Some(tf!("core.teams_admin.gallery_failed", error = e)),
                }
                Task::none()
            }
            TeamsAdminMsg::Joined(_, group_id, result) => {
                let admin = &mut *self.teams_admin;
                if let Some(gallery) = &mut admin.gallery {
                    match &result {
                        Ok(private) => {
                            let state = if *private {
                                GalleryJoin::Requested
                            } else {
                                GalleryJoin::Joined
                            };
                            gallery.joins.insert(group_id, state);
                        }
                        Err(_) => {
                            gallery.joins.remove(&group_id);
                        }
                    }
                }
                match result {
                    Ok(false) => self.reload_teams(),
                    Ok(true) => Task::none(),
                    Err(e) => {
                        self.teams_admin.error =
                            Some(tf!("core.teams_admin.join_failed", error = e));
                        Task::none()
                    }
                }
            }
            TeamsAdminMsg::CodeJoined(_, result) => {
                if let Some(gallery) = &mut self.teams_admin.gallery {
                    gallery.joining_code = false;
                }
                match result {
                    Ok(Some(team)) => {
                        self.teams_admin.gallery = None;
                        *self.notice = Some(t("core.teams_admin.joined").into());
                        self.after_team_change(team)
                    }
                    Ok(None) => {
                        if let Some(gallery) = &mut self.teams_admin.gallery {
                            gallery.code_pending = true;
                        }
                        Task::none()
                    }
                    Err(e) => {
                        self.teams_admin.error =
                            Some(tf!("core.teams_admin.code_failed", error = e));
                        Task::none()
                    }
                }
            }
            TeamsAdminMsg::TeamCreated(_, result) => {
                self.teams_admin.creating = false;
                match result {
                    Ok(team) => {
                        self.teams_admin.created += 1;
                        *self.notice = Some(t("core.teams_admin.team_created").into());
                        self.after_team_change(team)
                    }
                    Err(e) => {
                        self.teams_admin.error =
                            Some(tf!("core.teams_admin.create_team_failed", error = e));
                        Task::none()
                    }
                }
            }
            TeamsAdminMsg::ChannelCreated(_, team, result) => {
                self.teams_admin.creating = false;
                match result {
                    Ok(channel) => {
                        self.teams_admin.created += 1;
                        *self.notice = Some(t("core.teams_admin.channel_created").into());
                        self.channels_loaded.remove(&team);
                        self.expanded.insert(team.clone());
                        let select = Task::done(Msg::Select(Selection::Channel {
                            team: team.clone(),
                            channel,
                        }));
                        Task::batch([self.load_workspace(), select])
                    }
                    Err(e) => {
                        self.teams_admin.error =
                            Some(tf!("core.teams_admin.create_channel_failed", error = e));
                        Task::none()
                    }
                }
            }
            TeamsAdminMsg::Left(_, team, result) => match result {
                Ok(None) => {
                    self.teams.retain(|t| t.id != team);
                    *self.notice = Some(t("core.teams_admin.left").into());
                    if let Some(Selection::Channel { team: open, .. }) = &*self.selected
                        && *open == team
                    {
                        *self.selected = None;
                        if let Some((team, channel)) = self.first_shown_channel() {
                            return Task::batch([
                                Task::done(Msg::Select(Selection::Channel { team, channel })),
                                self.reload_teams(),
                            ]);
                        }
                    }
                    self.reload_teams()
                }
                Ok(Some(code)) => {
                    self.teams_admin.error = Some(match code.as_str() {
                        "OperationDeniedAsMemberIsLastAdminInTeam" => {
                            t("core.teams_admin.last_owner").into()
                        }
                        "RemoveFromTeamFailedAsUserIsLastOwnerOfPrivateChannel" => {
                            t("core.teams_admin.last_private_channel_owner").into()
                        }
                        other => tf!("core.teams_admin.leave_failed", error = other),
                    });
                    Task::none()
                }
                Err(e) => {
                    self.teams_admin.error = Some(tf!("core.teams_admin.leave_failed", error = e));
                    Task::none()
                }
            },
            TeamsAdminMsg::JoinCode(_, team, result) => {
                if let Some(view) = &mut self.teams_admin.join_code
                    && view.team_id == team
                {
                    view.loading = false;
                    match result {
                        Ok(code) => view.code = Some(code),
                        Err(e) => {
                            view.code = None;
                            self.teams_admin.error =
                                Some(tf!("core.teams_admin.join_code_failed", error = e));
                        }
                    }
                }
                Task::none()
            }
        }
    }
    fn teams_admin_action(&mut self, action: TeamsAdminAction) -> Task<Msg> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        let generation = self.generation;
        match action {
            TeamsAdminAction::DismissError => {
                self.teams_admin.error = None;
                Task::none()
            }
            TeamsAdminAction::OpenGallery => {
                self.teams_admin.gallery = Some(Gallery::default());
                self.gallery_load(client, false)
            }
            TeamsAdminAction::CloseGallery => {
                self.teams_admin.gallery = None;
                Task::none()
            }
            TeamsAdminAction::GallerySearch { query } => {
                let Some(gallery) = &mut self.teams_admin.gallery else {
                    return Task::none();
                };
                let query = query.trim().to_owned();
                if gallery.query == query {
                    return Task::none();
                }
                // Short queries show the suggestions again, as the web client does.
                gallery.query = if query.chars().count() < MIN_QUERY {
                    String::new()
                } else {
                    query
                };
                gallery.skip_token = None;
                self.gallery_load(client, false)
            }
            TeamsAdminAction::GalleryMore => {
                let more =
                    self.teams_admin.gallery.as_ref().is_some_and(|g| {
                        !g.loading && !g.query.is_empty() && g.skip_token.is_some()
                    });
                if more {
                    self.gallery_load(client, true)
                } else {
                    Task::none()
                }
            }
            TeamsAdminAction::JoinTeam { group_id } => {
                let Some(gallery) = &mut self.teams_admin.gallery else {
                    return Task::none();
                };
                let Some(team) = gallery.teams.iter().find(|t| t.group_id == group_id) else {
                    return Task::none();
                };
                if gallery.joins.contains_key(&group_id) {
                    return Task::none();
                }
                let private = team.private;
                gallery.joins.insert(group_id.clone(), GalleryJoin::Joining);
                let id = group_id.clone();
                Task::perform(
                    async move {
                        client
                            .join_team(&id)
                            .await
                            .map(|()| private)
                            .map_err(|e| e.to_string())
                    },
                    move |r| Msg::TeamsAdmin(TeamsAdminMsg::Joined(generation, group_id, r)),
                )
            }
            TeamsAdminAction::JoinWithCode { code } => {
                let code = code.trim().to_owned();
                let Some(gallery) = &mut self.teams_admin.gallery else {
                    return Task::none();
                };
                if code.is_empty() || gallery.joining_code {
                    return Task::none();
                }
                gallery.joining_code = true;
                gallery.code_pending = false;
                Task::perform(
                    async move {
                        client
                            .join_team_with_code(&code)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    move |r| Msg::TeamsAdmin(TeamsAdminMsg::CodeJoined(generation, r)),
                )
            }
            TeamsAdminAction::CreateTeam {
                name,
                description,
                visibility,
            } => {
                if self.teams_admin.creating {
                    return Task::none();
                }
                if let Err(e) = validate_team(&name, &description) {
                    self.teams_admin.error = Some(e);
                    return Task::none();
                }
                self.teams_admin.creating = true;
                self.teams_admin.error = None;
                let team = NewTeam {
                    name,
                    description,
                    visibility: match visibility {
                        NewTeamVisibility::Private => TeamVisibility::Private,
                        NewTeamVisibility::Public => TeamVisibility::Public,
                        NewTeamVisibility::OrgWide => TeamVisibility::OrgWide,
                    },
                };
                Task::perform(
                    async move { client.create_team(&team).await.map_err(|e| e.to_string()) },
                    move |r| Msg::TeamsAdmin(TeamsAdminMsg::TeamCreated(generation, r)),
                )
            }
            TeamsAdminAction::CreateChannel {
                team_id,
                name,
                description,
                private,
                show_for_everyone,
            } => {
                if self.teams_admin.creating {
                    return Task::none();
                }
                let Some(team) = self.teams.iter().find(|t| t.id == team_id) else {
                    return Task::none();
                };
                let existing: Vec<String> = team
                    .channels
                    .iter()
                    .map(|c| c.display_name.clone())
                    .collect();
                if let Err(e) = validate_channel(&name, &description, &existing) {
                    self.teams_admin.error = Some(e);
                    return Task::none();
                }
                let Some(group) = team.group_id().map(str::to_owned) else {
                    self.teams_admin.error = Some(t("core.teams_admin.no_group").into());
                    return Task::none();
                };
                self.teams_admin.creating = true;
                self.teams_admin.error = None;
                let team = team_id.clone();
                Task::perform(
                    async move {
                        let id = client
                            .create_channel(&team, &group, &name, &description, private)
                            .await
                            .map_err(|e| e.to_string())?;
                        // A separate call, made only for standard channels, as the web client.
                        if show_for_everyone && !private {
                            let _ = client.set_default_channel(&id, true).await;
                        }
                        Ok(id)
                    },
                    move |r| Msg::TeamsAdmin(TeamsAdminMsg::ChannelCreated(generation, team_id, r)),
                )
            }
            TeamsAdminAction::LeaveTeam { team_id } => {
                let Some(group) = self
                    .teams
                    .iter()
                    .find(|t| t.id == team_id)
                    .and_then(|t| t.group_id())
                    .map(str::to_owned)
                else {
                    self.teams_admin.error = Some(t("core.teams_admin.no_group").into());
                    return Task::none();
                };
                let (team, me) = (team_id.clone(), self.me.clone());
                Task::perform(
                    async move {
                        client
                            .leave_team(&team, &me, &group)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    move |r| Msg::TeamsAdmin(TeamsAdminMsg::Left(generation, team_id, r)),
                )
            }
            TeamsAdminAction::ShowJoinCode { team_id } => {
                let Some(team) = self.teams.iter().find(|t| t.id == team_id) else {
                    return Task::none();
                };
                self.teams_admin.join_code = Some(JoinCodeView {
                    team_id: team_id.clone(),
                    team_name: team.display_name.clone(),
                    code: None,
                    loading: true,
                });
                self.join_code_task(client, team_id, false)
            }
            TeamsAdminAction::ResetJoinCode => {
                let Some(view) = &mut self.teams_admin.join_code else {
                    return Task::none();
                };
                view.loading = true;
                let team = view.team_id.clone();
                self.join_code_task(client, team, true)
            }
            TeamsAdminAction::CloseJoinCode => {
                self.teams_admin.join_code = None;
                Task::none()
            }
        }
    }
    fn join_code_task(&self, client: Client, team: String, reset: bool) -> Task<Msg> {
        let generation = self.generation;
        Task::perform(
            async move {
                let r = client
                    .team_join_code(&team, reset)
                    .await
                    .map_err(|e| e.to_string());
                (team, r)
            },
            move |(team, r)| Msg::TeamsAdmin(TeamsAdminMsg::JoinCode(generation, team, r)),
        )
    }
    fn gallery_load(&mut self, client: Client, append: bool) -> Task<Msg> {
        let admin = &mut *self.teams_admin;
        let Some(gallery) = &mut admin.gallery else {
            return Task::none();
        };
        admin.request += 1;
        let request = admin.request;
        gallery.loading = true;
        let query = gallery.query.clone();
        let skip = if append {
            gallery.skip_token.clone()
        } else {
            None
        };
        let generation = self.generation;
        Task::perform(
            async move {
                if query.is_empty() {
                    client.suggested_teams().await
                } else {
                    client.find_teams(&query, skip.as_deref()).await
                }
                .map_err(|e| e.to_string())
            },
            move |r| Msg::TeamsAdmin(TeamsAdminMsg::Gallery(generation, request, append, r)),
        )
    }
    /// Reads the team list again; opens `team`'s General channel once it is there.
    fn after_team_change(&mut self, team: String) -> Task<Msg> {
        Task::batch([
            self.reload_teams(),
            Task::done(Msg::Select(Selection::Channel {
                team: team.clone(),
                channel: team,
            })),
        ])
    }
    fn reload_teams(&mut self) -> Task<Msg> {
        *self.loading_workspace = false;
        self.load_workspace()
    }
    pub(crate) fn refresh_teams_admin_view(&mut self) {
        let admin = &mut *self.teams_admin;
        let view = TeamsAdminView {
            gallery: admin.gallery.as_ref().map(|g| GalleryView {
                query: g.query.clone(),
                teams: g
                    .teams
                    .iter()
                    .map(|team| GalleryTeamView {
                        group_id: team.group_id.clone(),
                        name: team.name.clone(),
                        description: team.description.clone(),
                        private: team.private,
                        known_members: team.known_members,
                        join: g
                            .joins
                            .get(&team.group_id)
                            .copied()
                            .unwrap_or(GalleryJoin::None),
                    })
                    .collect(),
                loading: g.loading,
                has_more: !g.query.is_empty() && g.skip_token.is_some(),
                joining_code: g.joining_code,
                code_pending: g.code_pending,
            }),
            join_code: admin.join_code.clone(),
            creating: admin.creating,
            created: admin.created,
            error: admin.error.clone(),
        };
        if admin.view != view {
            admin.view = view;
        }
    }
}
fn validate_team(name: &str, description: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(t("core.teams_admin.name_required").into());
    }
    if name.chars().count() > TEAM_NAME_MAX {
        return Err(tf!("core.teams_admin.name_too_long", max = TEAM_NAME_MAX));
    }
    let lower = name.to_lowercase();
    if lower.starts_with("www.") || lower.starts_with("http://") || lower.starts_with("https://") {
        return Err(t("core.teams_admin.no_url_in_name").into());
    }
    if description.trim().len() > DESCRIPTION_MAX {
        return Err(tf!(
            "core.teams_admin.description_too_long",
            max = DESCRIPTION_MAX
        ));
    }
    Ok(())
}
/// The web client's channel name rules (`183841`), before the service checks the rest.
fn validate_channel(name: &str, description: &str, existing: &[String]) -> Result<(), String> {
    const FORBIDDEN: &str = "~#%&*{}+/\\:<>?|'\"";
    const RESERVED: [&str; 7] = [
        "forms",
        "con",
        "prn",
        "aux",
        "nul",
        "general",
        "desktop.ini",
    ];
    let name = name.trim();
    if name.is_empty() {
        return Err(t("core.teams_admin.name_required").into());
    }
    if name.chars().count() > CHANNEL_NAME_MAX {
        return Err(tf!(
            "core.teams_admin.name_too_long",
            max = CHANNEL_NAME_MAX
        ));
    }
    let lower = name.to_lowercase();
    let device = |prefix: &str| {
        lower
            .strip_prefix(prefix)
            .is_some_and(|n| n.len() == 1 && n.chars().all(|c| ('1'..='9').contains(&c)))
    };
    if name.chars().any(|c| FORBIDDEN.contains(c))
        || name.contains("..")
        || name.starts_with(['_', '.'])
        || name.ends_with('.')
    {
        return Err(t("core.teams_admin.channel_name_characters").into());
    }
    if RESERVED.contains(&lower.as_str())
        || device("com")
        || device("lpt")
        || lower.contains(".lock")
        || lower.contains("_vti_")
    {
        return Err(t("core.teams_admin.channel_name_reserved").into());
    }
    if existing.iter().any(|e| e.trim().eq_ignore_ascii_case(name)) {
        return Err(t("core.teams_admin.channel_name_taken").into());
    }
    if description.trim().len() > DESCRIPTION_MAX {
        return Err(tf!(
            "core.teams_admin.description_too_long",
            max = DESCRIPTION_MAX
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channel_names_follow_the_web_client_rules() {
        let taken = vec!["Design".to_owned()];
        assert!(validate_channel("Roadmap 2027", "", &taken).is_ok());
        for bad in [
            "", "a/b", "_x", "x.", "a..b", "con", "COM3", "General", "design",
        ] {
            assert!(validate_channel(bad, "", &taken).is_err(), "{bad}");
        }
        assert!(validate_channel(&"x".repeat(51), "", &taken).is_err());
    }
    #[test]
    fn team_names_reject_urls_and_empty_names() {
        assert!(validate_team("Produit", "").is_ok());
        assert!(validate_team("  ", "").is_err());
        assert!(validate_team("https://example.com", "").is_err());
    }
}
