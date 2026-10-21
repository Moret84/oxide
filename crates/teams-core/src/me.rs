//! The user's own presence: the hand-picked status (`forceavailability`) and the note (`publishnote`).
use super::view::{PresenceStatus, PresenceView};
use super::*;
use chrono::{Datelike, Local, TimeZone};
use teams_chat::{AutomaticReplies, ChosenAvailability, ExternalAudience, RepliesStatus};
use teams_i18n::{t, tf};

/// Wait before reading the status back: UPS applies a change a moment after the PUT.
const SETTLE: Duration = Duration::from_millis(1500);
type Saving =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), teams_chat::Error>> + Send>>;
/// `presencePostHeartbeatIntervalInMs`: the endpoint's heartbeat.
const HEARTBEAT: Duration = Duration::from_secs(120);
/// How long a status just chosen is shown while UPS still reports the previous one.
const PROPAGATION: Duration = Duration::from_secs(30);
/// "Never" for a note, as the web client writes it.
const NEVER: &str = "9999-12-31T00:00:00.000Z";

/// A status the user can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum MyStatus {
    Available,
    Busy,
    DoNotDisturb,
    BeRightBack,
    Away,
    /// "Appear offline".
    Offline,
}
impl MyStatus {
    fn chosen(self) -> ChosenAvailability {
        match self {
            Self::Available => ChosenAvailability::Available,
            Self::Busy => ChosenAvailability::Busy,
            Self::DoNotDisturb => ChosenAvailability::DoNotDisturb,
            Self::BeRightBack => ChosenAvailability::BeRightBack,
            Self::Away => ChosenAvailability::Away,
            Self::Offline => ChosenAvailability::Offline,
        }
    }
    fn presence(self) -> PresenceStatus {
        match self {
            Self::Available => PresenceStatus::Available,
            Self::Busy => PresenceStatus::Busy,
            Self::DoNotDisturb => PresenceStatus::DoNotDisturb,
            Self::BeRightBack => PresenceStatus::BeRightBack,
            Self::Away => PresenceStatus::Away,
            Self::Offline => PresenceStatus::Offline,
        }
    }
}
/// How long a status or a note lasts: the web client's expiration choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum StatusDuration {
    HalfHour,
    OneHour,
    TwoHours,
    FourHours,
    Today,
    ThisWeek,
    Never,
}
impl StatusDuration {
    /// The end in UTC, ISO 8601 with milliseconds; `None` for "never".
    fn until(self, now: DateTime<Local>) -> Option<String> {
        let end = match self {
            Self::HalfHour => now + chrono::Duration::minutes(30),
            Self::OneHour => now + chrono::Duration::hours(1),
            Self::TwoHours => now + chrono::Duration::hours(2),
            Self::FourHours => now + chrono::Duration::hours(4),
            Self::Today => end_of_day(now, 0)?,
            // The end of Sunday, the last day of the week where Monday starts it.
            Self::ThisWeek => end_of_day(now, 6 - now.weekday().num_days_from_monday())?,
            Self::Never => return None,
        };
        Some(
            end.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )
    }
}
fn end_of_day(now: DateTime<Local>, days: u32) -> Option<DateTime<Local>> {
    let date = now.date_naive() + chrono::Days::new(days.into());
    Local
        .from_local_datetime(&date.and_hms_milli_opt(23, 59, 59, 999)?)
        .earliest()
}

/// Who outside the organization gets the external reply.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum OutsideAudience {
    #[default]
    ContactsOnly,
    All,
}
/// Automatic replies as the settings page edits them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct OutOfOffice {
    pub enabled: bool,
    /// Only between `start` and `end`.
    pub scheduled: bool,
    /// Epoch ms.
    pub start_ms: i64,
    pub end_ms: i64,
    pub internal_message: String,
    pub reply_outside: bool,
    pub outside_audience: OutsideAudience,
    pub external_message: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct OutOfOfficeView {
    /// `None` until read.
    pub settings: Option<OutOfOffice>,
    pub loading: bool,
    pub saving: bool,
    /// Bumped by each save that went through.
    pub saved: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct MeView {
    /// What others see; `None` until UPS answered.
    pub presence: Option<PresenceView>,
    /// A status chosen by hand is in force: "Reset status" applies.
    pub forced: bool,
    /// "Until 18:30", "Until Friday"…, when the chosen status ends.
    pub forced_until: Option<String>,
    /// The status note, as plain text.
    pub note: String,
    /// When the note disappears; `None` when it never does.
    pub note_until: Option<String>,
    /// Automatic replies are on (`calendarData.isOutOfOffice`).
    pub out_of_office: bool,
    pub out_of_office_note: String,
    pub note_limit: u32,
    pub saving: bool,
    pub error: Option<String>,
    pub out_of_office_settings: OutOfOfficeView,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum MeAction {
    SetStatus {
        status: MyStatus,
        duration: StatusDuration,
    },
    ResetStatus,
    SetNote {
        text: String,
        duration: StatusDuration,
    },
    ClearNote,
    Refresh,
    DismissError,
    LoadOutOfOffice,
    SaveOutOfOffice {
        settings: OutOfOffice,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum MeMsg {
    Action(MeAction),
    Saved(u64, Result<(), String>),
    Loaded(u64, Result<Option<Presence>, String>),
    /// The endpoint was published (`true`) or its heartbeat sent (`false`).
    Endpoint(u64, bool, Result<(), (String, bool)>),
    RepliesLoaded(u64, Result<AutomaticReplies, String>),
    RepliesSaved(u64, OutOfOffice, Result<(), String>),
}

#[derive(Default)]
pub(crate) struct MeState {
    pub view: MeView,
    presence: Option<Presence>,
    /// The status just chosen, and when: shown until UPS reports it.
    chosen: Option<(MyStatus, std::time::Instant)>,
    saving: u32,
    replies: OutOfOfficeView,
    /// This app's presence endpoint: UPS keeps a chosen status only while one is alive.
    endpoint: String,
    published: bool,
    heartbeat: Option<std::time::Instant>,
}

impl App {
    pub(crate) fn update_me(&mut self, msg: MeMsg) -> Task<Msg> {
        let generation = self.generation;
        match msg {
            MeMsg::Saved(g, _)
            | MeMsg::Loaded(g, _)
            | MeMsg::Endpoint(g, ..)
            | MeMsg::RepliesLoaded(g, _)
            | MeMsg::RepliesSaved(g, ..)
                if g != generation =>
            {
                Task::none()
            }
            MeMsg::Action(MeAction::LoadOutOfOffice) => {
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                self.me_state.replies.loading = true;
                self.me_state.refresh_view();
                Task::perform(
                    async move { client.automatic_replies().await.map_err(|e| e.to_string()) },
                    move |r| Msg::Me(MeMsg::RepliesLoaded(generation, r)),
                )
            }
            MeMsg::Action(MeAction::SaveOutOfOffice { settings }) => {
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                if let Err(e) = validate_replies(&settings) {
                    self.me_state.view.error = Some(e);
                    self.me_state.refresh_view();
                    return Task::none();
                }
                self.me_state.replies.saving = true;
                self.me_state.view.error = None;
                self.me_state.refresh_view();
                let wire = to_wire(&settings);
                Task::perform(
                    async move {
                        client
                            .set_automatic_replies(&wire)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    move |r| Msg::Me(MeMsg::RepliesSaved(generation, settings, r)),
                )
            }
            MeMsg::RepliesLoaded(_, result) => {
                let me = &mut *self.me_state;
                me.replies.loading = false;
                match result {
                    Ok(replies) => me.replies.settings = Some(from_wire(&replies, Local::now())),
                    Err(e) => {
                        // The page still opens on the web client's defaults.
                        me.replies.settings =
                            Some(from_wire(&AutomaticReplies::default(), Local::now()));
                        me.view.error = Some(tf!("core.me.replies_not_loaded", error = e));
                    }
                }
                me.refresh_view();
                Task::none()
            }
            MeMsg::RepliesSaved(_, settings, result) => {
                let me = &mut *self.me_state;
                me.replies.saving = false;
                match result {
                    Ok(()) => {
                        me.replies.settings = Some(settings);
                        me.replies.saved += 1;
                    }
                    Err(e) => me.view.error = Some(tf!("core.me.replies_not_saved", error = e)),
                }
                me.refresh_view();
                // Presence shows automatic replies once Exchange told UPS.
                self.load_my_presence(Duration::from_secs(5))
            }
            MeMsg::Action(MeAction::DismissError) => {
                self.me_state.view.error = None;
                Task::none()
            }
            MeMsg::Action(MeAction::Refresh) => self.load_my_presence(Duration::ZERO),
            MeMsg::Action(action) => {
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                let now = Local::now();
                let me = &mut *self.me_state;
                // A status needs a live endpoint: the first change publishes it.
                let endpoint = me.endpoint().to_owned();
                let publish = !me.published;
                me.published = true;
                let setup = client.clone();
                let ready = {
                    let endpoint = endpoint.clone();
                    async move {
                        if publish {
                            let _ = setup.publish_endpoint(&endpoint).await;
                        }
                    }
                };
                me.saving += 1;
                me.view.error = None;
                let work = match action {
                    MeAction::SetStatus { status, duration } => {
                        me.chosen = Some((status, std::time::Instant::now()));
                        let until = duration.until(now);
                        Box::pin(async move {
                            ready.await;
                            client
                                .set_my_availability(
                                    status.chosen(),
                                    until.as_deref(),
                                    Some(&endpoint),
                                )
                                .await
                        }) as Saving
                    }
                    MeAction::ResetStatus => {
                        me.chosen = None;
                        Box::pin(async move {
                            ready.await;
                            client.reset_my_availability(Some(&endpoint)).await
                        })
                    }
                    MeAction::SetNote { text, duration } => {
                        let html = note_html(&text);
                        let until = duration.until(now).unwrap_or_else(|| NEVER.into());
                        Box::pin(async move {
                            ready.await;
                            client
                                .publish_note(&html, Some(&until), Some(&endpoint))
                                .await
                        })
                    }
                    MeAction::ClearNote => Box::pin(async move {
                        ready.await;
                        client.publish_note("", None, Some(&endpoint)).await
                    }),
                    MeAction::Refresh
                    | MeAction::DismissError
                    | MeAction::LoadOutOfOffice
                    | MeAction::SaveOutOfOffice { .. } => unreachable!(),
                };
                me.refresh_view();
                Task::perform(
                    async move { work.await.map_err(|e| e.to_string()) },
                    move |r| Msg::Me(MeMsg::Saved(generation, r)),
                )
            }
            MeMsg::Endpoint(_, published, result) => {
                if let Err((_, gone)) = result {
                    // A forgotten endpoint is published again on the next tick.
                    if gone || published {
                        self.me_state.published = false;
                    }
                }
                Task::none()
            }
            MeMsg::Saved(_, result) => {
                let me = &mut *self.me_state;
                me.saving = me.saving.saturating_sub(1);
                if let Err(e) = result {
                    me.chosen = None;
                    me.view.error = Some(tf!("core.me.status_not_saved", error = e));
                }
                me.refresh_view();
                self.load_my_presence(SETTLE)
            }
            MeMsg::Loaded(_, result) => {
                let me = &mut *self.me_state;
                match result {
                    Ok(Some(presence)) => {
                        // UPS reports a new status late: show the chosen one until it does, `PROPAGATION` at most.
                        let reported = PresenceView::from(&presence).status;
                        if me.chosen.is_some_and(|(status, at)| {
                            status.presence() == reported || at.elapsed() > PROPAGATION
                        }) {
                            me.chosen = None;
                        }
                        me.presence = Some(presence);
                    }
                    Ok(None) => {}
                    // The menu still works without it; the error would only be noise.
                    Err(_) => {}
                }
                me.refresh_view();
                Task::none()
            }
        }
    }
    /// Reads the user's own presence, `after` a pause for a change to settle.
    pub(crate) fn load_my_presence(&self, after: Duration) -> Task<Msg> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        let me = self.me.clone();
        let generation = self.generation;
        Task::perform(
            async move {
                tokio::time::sleep(after).await;
                let found = client
                    .presences(std::slice::from_ref(&me))
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(found.into_iter().find(|p| p.mri.eq_ignore_ascii_case(&me)))
            },
            move |r| Msg::Me(MeMsg::Loaded(generation, r)),
        )
    }
    /// Publishes this app's presence endpoint, at sign-in as the web client at startup.
    pub(crate) fn publish_presence_endpoint(&mut self) -> Task<Msg> {
        self.me_state.published = false;
        self.me_state.heartbeat = None;
        self.presence_heartbeat(true)
    }
    /// Keeps the endpoint alive (`activityReporting: Direct` heartbeat), republishing it when UPS forgot it.
    pub(crate) fn presence_heartbeat(&mut self, active: bool) -> Task<Msg> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        let generation = self.generation;
        let me = &mut *self.me_state;
        let endpoint = me.endpoint().to_owned();
        if me.heartbeat.is_some_and(|at| at.elapsed() < HEARTBEAT) {
            return Task::none();
        }
        me.heartbeat = Some(std::time::Instant::now());
        if !me.published {
            me.published = true;
            return Task::perform(
                async move {
                    client
                        .publish_endpoint(&endpoint)
                        .await
                        .map_err(|e| (e.to_string(), false))
                },
                move |r| Msg::Me(MeMsg::Endpoint(generation, true, r)),
            );
        }
        Task::perform(
            async move {
                client
                    .report_my_activity(&endpoint, active)
                    .await
                    .map_err(|e| {
                        let gone = matches!(&e, teams_chat::Error::Status { status, .. } if status.as_u16() == 404);
                        (e.to_string(), gone)
                    })
            },
            move |r| Msg::Me(MeMsg::Endpoint(generation, false, r)),
        )
    }
    /// The endpoint to remove when signing out, if one was published.
    pub(crate) fn presence_endpoint(&self) -> Option<String> {
        self.me_state
            .published
            .then(|| self.me_state.endpoint.clone())
    }
    pub(crate) fn relocalize_me(&mut self) {
        self.me_state.refresh_view();
    }
}
impl MeState {
    fn endpoint(&mut self) -> &str {
        if self.endpoint.is_empty() {
            self.endpoint = uuid::Uuid::new_v4().to_string();
        }
        &self.endpoint
    }
    pub(crate) fn refresh_view(&mut self) {
        let presence = self.presence.as_ref();
        let mut shown = presence.map(PresenceView::from);
        if let Some((chosen, _)) = self.chosen {
            shown = Some(PresenceView {
                status: chosen.presence(),
                out_of_office: shown.as_ref().is_some_and(|p| p.out_of_office),
            });
        }
        let note = presence
            .and_then(|p| p.note.as_deref())
            .map(html::to_text_cached)
            .unwrap_or_default();
        self.view = MeView {
            presence: shown,
            forced: self.chosen.is_some() || presence.is_some_and(|p| p.forced_expiry.is_some()),
            forced_until: presence
                .and_then(|p| p.forced_expiry.as_deref())
                .and_then(until_label),
            note_until: presence
                .and_then(|p| p.note_expiry.as_deref())
                .filter(|_| !note.is_empty())
                .and_then(until_label),
            note: note.trim().to_owned(),
            out_of_office: presence.is_some_and(|p| p.out_of_office),
            out_of_office_note: presence
                .and_then(|p| p.out_of_office_note.as_deref())
                .map(|n| html::to_text_cached(n).trim().to_owned())
                .unwrap_or_default(),
            note_limit: teams_chat::STATUS_NOTE_LIMIT as u32,
            saving: self.saving > 0,
            error: self.view.error.clone(),
            out_of_office_settings: self.replies.clone(),
        };
    }
}
fn iso_ms(iso: Option<&str>) -> Option<i64> {
    DateTime::parse_from_rfc3339(iso?)
        .ok()
        .map(|t| t.timestamp_millis())
}
fn ms_iso(ms: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}
/// The settings page's view of the answer: empty dates start now and end a day later.
fn from_wire(replies: &AutomaticReplies, now: DateTime<Local>) -> OutOfOffice {
    let start = iso_ms(replies.scheduled_start_date_time.as_deref())
        .unwrap_or_else(|| now.timestamp_millis());
    let end = iso_ms(replies.scheduled_end_date_time.as_deref()).unwrap_or(start + 86_400_000);
    OutOfOffice {
        enabled: replies.status != RepliesStatus::Disabled,
        scheduled: replies.status == RepliesStatus::Scheduled,
        start_ms: start,
        end_ms: end,
        internal_message: replies.internal_reply_message.trim().to_owned(),
        reply_outside: replies.external_audience != ExternalAudience::None,
        outside_audience: if replies.external_audience == ExternalAudience::All {
            OutsideAudience::All
        } else {
            OutsideAudience::ContactsOnly
        },
        external_message: replies.external_reply_message.trim().to_owned(),
    }
}
/// `status` from the two switches, `none` without outside replies, the period only when scheduled.
fn to_wire(settings: &OutOfOffice) -> AutomaticReplies {
    AutomaticReplies {
        status: match (settings.enabled, settings.scheduled) {
            (false, _) => RepliesStatus::Disabled,
            (true, true) => RepliesStatus::Scheduled,
            (true, false) => RepliesStatus::AlwaysEnabled,
        },
        internal_reply_message: settings.internal_message.trim().to_owned(),
        external_reply_message: if settings.reply_outside {
            settings.external_message.trim().to_owned()
        } else {
            String::new()
        },
        external_audience: match (settings.reply_outside, settings.outside_audience) {
            (false, _) => ExternalAudience::None,
            (true, OutsideAudience::ContactsOnly) => ExternalAudience::ContactsOnly,
            (true, OutsideAudience::All) => ExternalAudience::All,
        },
        scheduled_start_date_time: settings
            .scheduled
            .then(|| ms_iso(settings.start_ms))
            .flatten(),
        scheduled_end_date_time: settings
            .scheduled
            .then(|| ms_iso(settings.end_ms))
            .flatten(),
    }
}
fn validate_replies(settings: &OutOfOffice) -> Result<(), String> {
    if !settings.enabled {
        return Ok(());
    }
    if settings.internal_message.trim().is_empty() {
        return Err(t("core.me.replies_need_a_message").into());
    }
    if settings.scheduled && settings.start_ms >= settings.end_ms {
        return Err(t("core.me.replies_end_before_start").into());
    }
    Ok(())
}
/// A note as the web client publishes it: one paragraph of escaped text, lines kept.
fn note_html(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    format!("<p>{}</p>", escape(text))
}
/// "Until 18:30" today, "Until Fri 18:30" this week, "Until 24/10/2026" later; `None` for a far-off "never".
fn until_label(iso: &str) -> Option<String> {
    let end = DateTime::parse_from_rfc3339(iso)
        .ok()?
        .with_timezone(&Local);
    let now = Local::now();
    if end.year() > now.year() + 50 {
        return None;
    }
    let days = (end.date_naive() - now.date_naive()).num_days();
    let when = if days == 0 {
        end.format("%H:%M").to_string()
    } else if (1..7).contains(&days) {
        format!(
            "{} {}",
            t(WEEKDAYS[end.weekday().num_days_from_monday() as usize]),
            end.format("%H:%M")
        )
    } else {
        end.format(t("core.me.date_format")).to_string()
    };
    Some(tf!("core.me.until", when = when))
}
const WEEKDAYS: [&str; 7] = [
    "core.me.monday",
    "core.me.tuesday",
    "core.me.wednesday",
    "core.me.thursday",
    "core.me.friday",
    "core.me.saturday",
    "core.me.sunday",
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durations_end_where_the_web_client_ends_them() {
        let now = Local.with_ymd_and_hms(2026, 10, 21, 10, 0, 0).unwrap(); // a Wednesday
        let utc = |s: &str| {
            DateTime::parse_from_rfc3339(s)
                .unwrap()
                .with_timezone(&Local)
                .naive_local()
                .to_string()
        };
        assert_eq!(
            utc(&StatusDuration::OneHour.until(now).unwrap()),
            "2026-10-21 11:00:00"
        );
        assert_eq!(
            utc(&StatusDuration::Today.until(now).unwrap()),
            "2026-10-21 23:59:59.999"
        );
        assert_eq!(
            utc(&StatusDuration::ThisWeek.until(now).unwrap()),
            "2026-10-25 23:59:59.999"
        );
        assert_eq!(StatusDuration::Never.until(now), None);
    }
    #[test]
    fn replies_switches_map_to_the_status() {
        let now = Local.with_ymd_and_hms(2026, 10, 21, 10, 0, 0).unwrap();
        let mut settings = from_wire(&AutomaticReplies::default(), now);
        assert!(!settings.enabled);
        assert_eq!(settings.end_ms - settings.start_ms, 86_400_000);
        settings.enabled = true;
        settings.internal_message = " Absent ".into();
        let wire = to_wire(&settings);
        assert_eq!(wire.status, RepliesStatus::AlwaysEnabled);
        assert_eq!(wire.external_audience, ExternalAudience::None);
        assert_eq!(wire.internal_reply_message, "Absent");
        assert!(wire.scheduled_start_date_time.is_none());
        settings.scheduled = true;
        settings.reply_outside = true;
        let wire = to_wire(&settings);
        assert_eq!(wire.status, RepliesStatus::Scheduled);
        assert_eq!(wire.external_audience, ExternalAudience::ContactsOnly);
        assert!(wire.scheduled_end_date_time.unwrap().ends_with('Z'));
        settings.end_ms = settings.start_ms;
        assert!(validate_replies(&settings).is_err());
    }
    #[test]
    fn notes_are_escaped_paragraphs() {
        assert_eq!(note_html("  "), "");
        assert_eq!(note_html("En congés <3"), "<p>En congés &lt;3</p>");
    }
}
