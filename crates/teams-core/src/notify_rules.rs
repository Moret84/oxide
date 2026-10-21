//! Pure notification rules: the `userPersonalSettings` document, the UI choice converter and the banner filter.
use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use teams_i18n::t;

pub(crate) const TOAST: &str = "toastSettings";
pub(crate) const ACTIVITY: &str = "activityGenerationSettings";
pub(crate) const EMAIL: &str = "emailSettings";
pub(crate) const SOUNDS: &str = "soundsSettings";
pub(crate) const CHAT: &str = "chatSettings";
/// User property holding the serialized document.
pub(crate) const PROPERTY: &str = "userPersonalSettings";
/// Banners older than this are dropped.
pub(crate) const MAX_AGE_MS: i64 = 5 * 60 * 1000;

/// One line of the settings screen. Aggregated lines write several stored keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum NotificationRow {
    Chat,
    PersonalMentions,
    EveryoneMentions,
    Reactions,
    Replies,
    TeamMentions,
    TagMentions,
    ChannelPosts,
    FollowedThreads,
    MeetingStarted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum NotificationSection {
    Chats,
    TeamsAndChannels,
    Meetings,
}

impl NotificationRow {
    pub(crate) const ALL: [Self; 10] = [
        Self::Chat,
        Self::PersonalMentions,
        Self::EveryoneMentions,
        Self::Reactions,
        Self::Replies,
        Self::TeamMentions,
        Self::TagMentions,
        Self::ChannelPosts,
        Self::FollowedThreads,
        Self::MeetingStarted,
    ];
    /// Stored keys, written together; the first one is read back.
    pub(crate) fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Chat => &["Chat"],
            Self::PersonalMentions => &["MentionInChat_Person_v2", "Mention_Person"],
            Self::EveryoneMentions => &["MentionInChat_Everyone"],
            Self::Reactions => &["LikeInChat_v2", "Like"],
            Self::Replies => &["Reply", "ReplyToReply"],
            Self::TeamMentions => &["Mention_Team", "Visible_ChannelMention_v2"],
            Self::TagMentions => &["Mention_Tag"],
            Self::ChannelPosts => &[
                "Visible_ChannelNewMessage_v2",
                "Visible_ChannelReplyMessage_v2",
            ],
            Self::FollowedThreads => &["RepliesFollowedThreads"],
            Self::MeetingStarted => &["MeetingWentLiveNotification"],
        }
    }
    /// Choice when nothing is stored yet.
    pub(crate) fn default_choice(self) -> NotificationChoice {
        match self {
            Self::ChannelPosts => NotificationChoice::OnlyFeed,
            _ => NotificationChoice::BannerAndFeed,
        }
    }
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Chat => t("core.notify_rules.chat_messages"),
            Self::PersonalMentions => t("core.notify_rules.personal_mentions"),
            Self::EveryoneMentions => t("core.notify_rules.everyone_mentions"),
            Self::Reactions => t("core.notify_rules.likes_and_reactions"),
            Self::Replies => t("core.notify_rules.replies_to_my_messages_and_threads"),
            Self::TeamMentions => t("core.notify_rules.team_and_channel_mentions"),
            Self::TagMentions => t("core.notify_rules.tag_mentions"),
            Self::ChannelPosts => t("core.notify_rules.posts_in_shown_channels"),
            Self::FollowedThreads => t("core.notify_rules.replies_in_followed_threads"),
            Self::MeetingStarted => t("core.notify_rules.meeting_started"),
        }
    }
    pub(crate) fn section(self) -> NotificationSection {
        match self {
            Self::Chat | Self::PersonalMentions | Self::EveryoneMentions | Self::Reactions => {
                NotificationSection::Chats
            }
            Self::MeetingStarted => NotificationSection::Meetings,
            _ => NotificationSection::TeamsAndChannels,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum NotificationChoice {
    BannerAndFeed,
    OnlyFeed,
    Off,
}

impl NotificationChoice {
    /// (toast, email, activity) of the global converter.
    pub(crate) fn outputs(self) -> (bool, bool, bool) {
        match self {
            Self::Off => (false, false, false),
            Self::OnlyFeed => (false, false, true),
            Self::BannerAndFeed => (true, true, true),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum NotificationPreset {
    AllActivity,
    MyAtMentions,
    Custom,
}

impl NotificationPreset {
    /// Business enum stored in the document: `AllActivity=1, MyAtMentions=2, Custom=3`.
    fn code(self) -> i64 {
        match self {
            Self::AllActivity => 1,
            Self::MyAtMentions => 2,
            Self::Custom => 3,
        }
    }
    fn parse(value: &Value) -> Option<Self> {
        match value {
            Value::Number(n) => match n.as_i64()? {
                1 => Some(Self::AllActivity),
                2 => Some(Self::MyAtMentions),
                3 => Some(Self::Custom),
                _ => None,
            },
            Value::String(s) => match s.as_str() {
                "AllActivity" | "1" => Some(Self::AllActivity),
                "MyAtMentions" | "2" => Some(Self::MyAtMentions),
                "Custom" | "3" => Some(Self::Custom),
                _ => None,
            },
            _ => None,
        }
    }
}

/// On/Off settings, with where they are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum NotificationToggle {
    /// `toastSettings.ShowPreviewInToasts`.
    ShowPreview,
    /// `toastSettings.MuteNotificationsDuringMeetings`.
    MuteDuringMeetings,
    /// `soundsSettings.alertSound` (`All` / `Off`).
    Sounds,
    /// `soundsSettings.incomingCallsAndMeetingStartSound`.
    CallSounds,
    /// `toastSettings.PlayNotificationSoundsWhenBusyOrInMeetings`.
    SoundsWhenBusy,
    /// `toastSettings.PlayNotificationSoundsForUrgentAndPriority`.
    SoundsForUrgent,
    /// `toastSettings.muteGenericNotifications`: only urgent and priority banners.
    OnlyUrgent,
}

impl NotificationToggle {
    fn place(self) -> (&'static str, &'static str, bool) {
        match self {
            Self::ShowPreview => (TOAST, "ShowPreviewInToasts", true),
            Self::MuteDuringMeetings => (TOAST, "MuteNotificationsDuringMeetings", false),
            Self::Sounds => (SOUNDS, "alertSound", true),
            Self::CallSounds => (SOUNDS, "incomingCallsAndMeetingStartSound", true),
            Self::SoundsWhenBusy => (TOAST, "PlayNotificationSoundsWhenBusyOrInMeetings", false),
            Self::SoundsForUrgent => (TOAST, "PlayNotificationSoundsForUrgentAndPriority", true),
            Self::OnlyUrgent => (TOAST, "muteGenericNotifications", false),
        }
    }
    fn on_value(self) -> &'static str {
        match self {
            Self::Sounds | Self::CallSounds => "All",
            _ => "On",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum NotificationSound {
    Default,
    Vibe,
    Nudge,
    Tap,
    Flick,
    Pluck,
    Summon,
    Ping,
    DefaultUrgent,
    Prioritize,
    Escalate,
    Alarm,
}

impl NotificationSound {
    pub(crate) const STANDARD: [Self; 8] = [
        Self::Default,
        Self::Vibe,
        Self::Nudge,
        Self::Tap,
        Self::Flick,
        Self::Pluck,
        Self::Summon,
        Self::Ping,
    ];
    pub(crate) const URGENT: [Self; 4] = [
        Self::DefaultUrgent,
        Self::Prioritize,
        Self::Escalate,
        Self::Alarm,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Vibe => "Vibe",
            Self::Nudge => "Nudge",
            Self::Tap => "Tap",
            Self::Flick => "Flick",
            Self::Pluck => "Pluck",
            Self::Summon => "Summon",
            Self::Ping => "Ping",
            Self::DefaultUrgent => "DefaultUrgent",
            Self::Prioritize => "Prioritize",
            Self::Escalate => "Escalate",
            Self::Alarm => "Alarm",
        }
    }
    fn parse(name: &str) -> Option<Self> {
        Self::STANDARD
            .into_iter()
            .chain(Self::URGENT)
            .find(|s| s.name().eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum EmailDelay {
    Immediately,
    Every10Mins,
    Hourly,
    EveryEightHours,
    Daily,
    Never,
}

impl EmailDelay {
    pub(crate) const ALL: [Self; 6] = [
        Self::Immediately,
        Self::Every10Mins,
        Self::Hourly,
        Self::EveryEightHours,
        Self::Daily,
        Self::Never,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::Immediately => "Immediately",
            Self::Every10Mins => "Every10Mins",
            Self::Hourly => "Hourly",
            Self::EveryEightHours => "EveryEightHours",
            Self::Daily => "Daily",
            Self::Never => "Never",
        }
    }
}

/// Meeting chats without an explicit `alerts`. Stored as `all` / `participated` / `none`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum MeetingChats {
    /// UI `Unmute`.
    All,
    /// UI `MuteUntilJoin`.
    Participated,
    /// UI `MuteChat`.
    None,
}

impl MeetingChats {
    fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Participated => "participated",
            Self::None => "none",
        }
    }
    fn parse(value: &str) -> Option<Self> {
        match value {
            "all" | "Unmute" | "MeetingsUserIsInvitedTo" => Some(Self::All),
            "participated" | "MuteUntilJoin" | "MeetingsUserParticipatesIn" => {
                Some(Self::Participated)
            }
            "none" | "MuteChat" | "MeetingsManuallyUnmuted" => Some(Self::None),
            _ => None,
        }
    }
}

/// The user's answer to the meeting invitation, from the calendar (`myResponseType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rsvp {
    Organizer,
    Accepted,
    Tentative,
    NoResponse,
    Declined,
    /// "Follow" a meeting without attending.
    Follow,
}

impl Rsvp {
    pub(crate) fn from_response(response: &str, organizer: bool) -> Self {
        if organizer {
            return Self::Organizer;
        }
        match response {
            "Organizer" => Self::Organizer,
            "Accepted" => Self::Accepted,
            "TentativelyAccepted" | "Tentative" => Self::Tentative,
            "Declined" => Self::Declined,
            "Follow" | "Following" => Self::Follow,
            _ => Self::NoResponse,
        }
    }
    /// `meetingChatNotificationRSVPv2…` key of this answer.
    fn v2_key(self) -> Option<&'static str> {
        Some(match self {
            Self::Organizer => return None,
            Self::Accepted => "meetingChatNotificationRSVPv2Accepted",
            Self::Tentative => "meetingChatNotificationRSVPv2Tentative",
            Self::NoResponse => "meetingChatNotificationRSVPv2NoResponse",
            Self::Declined => "meetingChatNotificationRSVPv2Declined",
            Self::Follow => "meetingChatNotificationRSVPv2Follow",
        })
    }
}

const RSVP_V2: [Rsvp; 5] = [
    Rsvp::Accepted,
    Rsvp::Tentative,
    Rsvp::NoResponse,
    Rsvp::Declined,
    Rsvp::Follow,
];

/// The `userPersonalSettings` document; unknown categories and keys are kept so a write never drops another client's.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Prefs {
    pub(crate) raw: Map<String, Value>,
}

/// `On`, `All`, `true` and booleans are on; `Off`, `false` are off.
pub(crate) fn flag(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.as_str() {
            "On" | "on" | "All" | "true" | "True" | "BannerAndFeed" | "Banner" => Some(true),
            "Off" | "off" | "false" | "False" | "None" | "Mute" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

impl Prefs {
    /// Reads the property from `GET v1/users/ME/properties`: a JSON string, or an object.
    pub(crate) fn from_properties(properties: &Value) -> Option<Self> {
        let value = properties.get(PROPERTY)?;
        let raw = match value {
            Value::String(s) if s.trim().is_empty() => Map::new(),
            Value::String(s) => serde_json::from_str::<Value>(s).ok()?.as_object()?.clone(),
            Value::Object(map) => map.clone(),
            _ => return None,
        };
        Some(Self { raw })
    }
    /// Body value of `PUT …/properties?name=userPersonalSettings`.
    pub(crate) fn serialized(&self) -> Value {
        Value::String(Value::Object(self.raw.clone()).to_string())
    }
    pub(crate) fn get(&self, category: &str, key: &str) -> Option<&Value> {
        self.raw.get(category)?.get(key)
    }
    fn set(&mut self, category: &str, key: &str, value: Value) {
        let entry = self
            .raw
            .entry(category.to_owned())
            .or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        entry
            .as_object_mut()
            .expect("category is an object")
            .insert(key.to_owned(), value);
    }
    fn remove(&mut self, category: &str, key: &str) {
        if let Some(map) = self.raw.get_mut(category).and_then(Value::as_object_mut) {
            map.remove(key);
        }
    }

    pub(crate) fn choice(&self, row: NotificationRow) -> NotificationChoice {
        let key = row.keys()[0];
        let toast = self.get(TOAST, key).and_then(flag);
        let activity = self.get(ACTIVITY, key).and_then(flag);
        match (toast, activity) {
            (None, None) => row.default_choice(),
            (Some(true), _) => NotificationChoice::BannerAndFeed,
            (_, Some(true)) => NotificationChoice::OnlyFeed,
            (None, Some(false)) => {
                if row.default_choice() == NotificationChoice::BannerAndFeed {
                    NotificationChoice::BannerAndFeed
                } else {
                    NotificationChoice::Off
                }
            }
            (Some(false), _) => NotificationChoice::Off,
        }
    }
    /// Whether a banner of this kind passes the preference check.
    pub(crate) fn banner(&self, row: NotificationRow) -> bool {
        self.choice(row) == NotificationChoice::BannerAndFeed
    }
    /// Writes the three outputs of every key of the row and marks the preset `Custom`.
    pub(crate) fn set_choice(&mut self, row: NotificationRow, choice: NotificationChoice) {
        self.write_choice(row, choice);
        self.raw.insert(
            "selectedPreset".into(),
            json!(NotificationPreset::Custom.code()),
        );
    }
    fn write_choice(&mut self, row: NotificationRow, choice: NotificationChoice) {
        let (toast, email, activity) = choice.outputs();
        for key in row.keys() {
            self.set(TOAST, key, json!(toast));
            self.set(EMAIL, key, json!(email));
            self.set(ACTIVITY, key, json!(activity));
        }
    }
    pub(crate) fn preset(&self) -> NotificationPreset {
        self.raw
            .get("selectedPreset")
            .and_then(NotificationPreset::parse)
            .unwrap_or(NotificationPreset::Custom)
    }
    /// Rewrites every line: all banners for `AllActivity`, mention banners for `MyAtMentions`; `Custom` keeps them.
    pub(crate) fn apply_preset(&mut self, preset: NotificationPreset) {
        self.raw
            .insert("selectedPreset".into(), json!(preset.code()));
        match preset {
            NotificationPreset::Custom => {}
            NotificationPreset::AllActivity => {
                for row in NotificationRow::ALL {
                    self.write_choice(row, NotificationChoice::BannerAndFeed);
                }
            }
            NotificationPreset::MyAtMentions => {
                for row in NotificationRow::ALL {
                    let mention = matches!(
                        row,
                        NotificationRow::PersonalMentions
                            | NotificationRow::Chat
                            | NotificationRow::MeetingStarted
                    );
                    self.write_choice(
                        row,
                        if mention {
                            NotificationChoice::BannerAndFeed
                        } else {
                            NotificationChoice::OnlyFeed
                        },
                    );
                }
            }
        }
    }

    pub(crate) fn toggle(&self, toggle: NotificationToggle) -> bool {
        let (category, key, default) = toggle.place();
        self.get(category, key).and_then(flag).unwrap_or(default)
    }
    pub(crate) fn set_toggle(&mut self, toggle: NotificationToggle, on: bool) {
        let (category, key, _) = toggle.place();
        let value = if on { toggle.on_value() } else { "Off" };
        self.set(category, key, json!(value));
        // The mapper of `muteGenericNotifications` also allows urgent banners.
        if toggle == NotificationToggle::OnlyUrgent {
            self.set(TOAST, "allowUrgentNotifications", json!("On"));
        }
    }
    pub(crate) fn allow_urgent(&self) -> bool {
        self.get(TOAST, "allowUrgentNotifications")
            .and_then(flag)
            .unwrap_or(true)
    }

    pub(crate) fn sound(&self, urgent: bool) -> NotificationSound {
        self.get(TOAST, sound_key(urgent))
            .and_then(Value::as_str)
            .and_then(NotificationSound::parse)
            .unwrap_or(if urgent {
                NotificationSound::DefaultUrgent
            } else {
                NotificationSound::Default
            })
    }
    pub(crate) fn set_sound(&mut self, urgent: bool, sound: NotificationSound) {
        self.set(TOAST, sound_key(urgent), json!(sound.name()));
    }

    pub(crate) fn email_delay(&self) -> EmailDelay {
        self.get(EMAIL, "EmailNotificationDelay")
            .and_then(Value::as_str)
            .and_then(|s| EmailDelay::ALL.into_iter().find(|d| d.name() == s))
            // `defaultMissedActivityEmailDelayOption`.
            .unwrap_or(EmailDelay::Hourly)
    }
    pub(crate) fn set_email_delay(&mut self, delay: EmailDelay) {
        self.set(EMAIL, "EmailNotificationDelay", json!(delay.name()));
    }

    pub(crate) fn meeting_chats(&self) -> MeetingChats {
        self.get(CHAT, "meetingChatNotification")
            .and_then(Value::as_str)
            .and_then(MeetingChats::parse)
            .unwrap_or(MeetingChats::Participated)
    }
    /// Meeting chat without `alerts`: never for the organizer, else RSVP v2, per-answer, general; joining/writing ends "until I join".
    pub(crate) fn meeting_chat_muted(&self, rsvp: Option<Rsvp>, participated: bool) -> bool {
        if rsvp == Some(Rsvp::Organizer) {
            return false;
        }
        if let Some(on) = rsvp
            .and_then(Rsvp::v2_key)
            .and_then(|key| self.get(CHAT, key))
            .and_then(flag)
        {
            return !on;
        }
        let stored = |key: &str, default: MeetingChats| {
            self.get(CHAT, key)
                .and_then(Value::as_str)
                .and_then(MeetingChats::parse)
                .unwrap_or(default)
        };
        let choice = match rsvp {
            Some(Rsvp::Accepted) => stored(
                "meetingChatNotificationForAcceptedMeetings",
                MeetingChats::All,
            ),
            Some(Rsvp::Tentative | Rsvp::NoResponse) => stored(
                "meetingChatNotificationForTentativeMeetings",
                MeetingChats::Participated,
            ),
            Some(Rsvp::Declined | Rsvp::Follow) => MeetingChats::None,
            Some(Rsvp::Organizer) | None => self.meeting_chats(),
        };
        match choice {
            MeetingChats::All => false,
            MeetingChats::None => true,
            MeetingChats::Participated => !participated,
        }
    }

    /// The general setting, keeping RSVP variants in step like the web; "until I join" removes the v2 booleans.
    pub(crate) fn set_meeting_chats(&mut self, choice: MeetingChats) {
        let v2 = RSVP_V2
            .into_iter()
            .filter(|r| *r != Rsvp::Declined)
            .filter_map(Rsvp::v2_key);
        for key in v2 {
            if self.get(CHAT, key).is_none() {
                continue;
            }
            match choice {
                MeetingChats::All => self.set(CHAT, key, json!(true)),
                MeetingChats::None => self.set(CHAT, key, json!(false)),
                MeetingChats::Participated => self.remove(CHAT, key),
            }
        }
        for key in [
            "meetingChatNotification",
            "meetingChatNotificationForAcceptedMeetings",
            "meetingChatNotificationForTentativeMeetings",
        ] {
            self.set(CHAT, key, json!(choice.name()));
        }
    }

    pub(crate) fn paused_until(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let value = self.get(TOAST, "temporarilyPauseUntil")?.as_str()?;
        let until = DateTime::parse_from_rfc3339(value)
            .ok()?
            .with_timezone(&Utc);
        (until > now).then_some(until)
    }
    pub(crate) fn set_paused_until(&mut self, until: Option<DateTime<Utc>>) {
        match until {
            Some(until) => self.set(
                TOAST,
                "temporarilyPauseUntil",
                json!(until.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
            ),
            None => self.remove(TOAST, "temporarilyPauseUntil"),
        }
    }
}

fn sound_key(urgent: bool) -> &'static str {
    if urgent {
        "alertUrgentSoundType"
    } else {
        "alertSoundType"
    }
}

/// What an incoming event would notify about, before the filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Chat,
    PersonalMention,
    EveryoneMention,
    TeamMention,
    TagMention,
    Reaction,
    Reply,
    ChannelPost,
    ChannelReply,
    FollowedThreadReply,
    MeetingStarted,
    /// Anything else of the activity feed (apps, tasks, membership…).
    Other,
}

impl Kind {
    pub(crate) fn row(self) -> Option<NotificationRow> {
        Some(match self {
            Self::Chat => NotificationRow::Chat,
            Self::PersonalMention => NotificationRow::PersonalMentions,
            Self::EveryoneMention => NotificationRow::EveryoneMentions,
            Self::TeamMention => NotificationRow::TeamMentions,
            Self::TagMention => NotificationRow::TagMentions,
            Self::Reaction => NotificationRow::Reactions,
            Self::Reply => NotificationRow::Replies,
            Self::ChannelPost | Self::ChannelReply => NotificationRow::ChannelPosts,
            Self::FollowedThreadReply => NotificationRow::FollowedThreads,
            Self::MeetingStarted => NotificationRow::MeetingStarted,
            Self::Other => return None,
        })
    }
    /// Mentions get through a muted conversation (`ConversationMuted` exception).
    pub(crate) fn is_mention(self) -> bool {
        matches!(
            self,
            Self::PersonalMention | Self::EveryoneMention | Self::TeamMention | Self::TagMention
        )
    }
}

/// A notification candidate built from a pushed message or activity.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Candidate {
    pub(crate) kind: Kind,
    pub(crate) conversation_id: String,
    pub(crate) message_id: String,
    pub(crate) reply_chain_id: Option<String>,
    pub(crate) from_me: bool,
    pub(crate) title: String,
    pub(crate) subtitle: Option<String>,
    pub(crate) body: String,
    pub(crate) time_ms: i64,
    pub(crate) urgent: bool,
    pub(crate) deleted: bool,
    /// `displayBanner` / `displayBannerV2`: `"true"`, `"false"` or `"GNS"`.
    pub(crate) display_banner: Option<String>,
    /// The message itself is the activity's source: an activity-feed copy.
    pub(crate) from_activity: bool,
}

/// What the filter knows about the conversation of a candidate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ConversationRules {
    /// Explicit `alerts`, or the meeting-chat fallback.
    pub(crate) muted: bool,
    /// `dskNotif`: `Some(true)` all banners, `Some(false)` mentions only, `None` the global settings.
    pub(crate) desktop: Option<bool>,
    /// Per-channel answer for posts, replies and channel mentions (NSS).
    pub(crate) channel_banner: Option<bool>,
}

pub(crate) struct Context<'a> {
    pub(crate) prefs: &'a Prefs,
    pub(crate) now: DateTime<Utc>,
    pub(crate) conversation: ConversationRules,
    /// The window is in front and shows this conversation.
    pub(crate) visible: bool,
    pub(crate) in_call: bool,
    /// "Mute notifications during this meeting", local to the call.
    pub(crate) muted_in_call: bool,
    pub(crate) duplicate: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rejection {
    OwnMessage,
    NotEnabledByDisplayBanner,
    NotEnabledByPreferences,
    ConversationMuted,
    TooOld,
    Duplicate,
    Paused,
    DuringMeeting,
    ConversationVisible,
}

/// The banner checks, in order.
pub(crate) fn decide(c: &Candidate, ctx: &Context) -> Result<(), Rejection> {
    if c.from_me {
        return Err(Rejection::OwnMessage);
    }
    let server = c.display_banner.as_deref();
    if server == Some("false") {
        return Err(Rejection::NotEnabledByDisplayBanner);
    }
    if server != Some("true") {
        let enabled = match (ctx.conversation.desktop, ctx.conversation.channel_banner) {
            (Some(own), _) => own || c.kind.is_mention(),
            (None, Some(channel))
                if matches!(
                    c.kind,
                    Kind::ChannelPost | Kind::ChannelReply | Kind::TeamMention
                ) =>
            {
                channel
            }
            _ => match c.kind.row() {
                Some(row) => ctx.prefs.banner(row),
                None => !ctx.prefs.toggle(NotificationToggle::OnlyUrgent),
            },
        };
        let urgent_only = ctx.prefs.toggle(NotificationToggle::OnlyUrgent)
            && !(c.urgent && ctx.prefs.allow_urgent());
        if !enabled || urgent_only {
            return Err(Rejection::NotEnabledByPreferences);
        }
    }
    // The meeting start is announced even in a meeting chat muted until joining.
    if ctx.conversation.muted && !c.kind.is_mention() && !c.urgent && c.kind != Kind::MeetingStarted
    {
        return Err(Rejection::ConversationMuted);
    }
    if !c.deleted && (ctx.now.timestamp_millis() - c.time_ms) > MAX_AGE_MS {
        return Err(Rejection::TooOld);
    }
    if ctx.duplicate && !c.deleted {
        return Err(Rejection::Duplicate);
    }
    if ctx.prefs.paused_until(ctx.now).is_some() {
        return Err(Rejection::Paused);
    }
    if ctx.in_call
        && (ctx.muted_in_call || ctx.prefs.toggle(NotificationToggle::MuteDuringMeetings))
        && !(c.urgent && ctx.prefs.allow_urgent())
    {
        return Err(Rejection::DuringMeeting);
    }
    if ctx.visible {
        return Err(Rejection::ConversationVisible);
    }
    Ok(())
}

/// Sound of a banner that passed the filter, or `None` for a silent one.
pub(crate) fn sound(prefs: &Prefs, urgent: bool, in_call: bool) -> Option<NotificationSound> {
    if !prefs.toggle(NotificationToggle::Sounds) {
        return None;
    }
    let urgent_sound = urgent && prefs.toggle(NotificationToggle::SoundsForUrgent);
    if in_call && !prefs.toggle(NotificationToggle::SoundsWhenBusy) && !urgent_sound {
        return None;
    }
    Some(prefs.sound(urgent))
}

/// `followAllRc`: up to 15 `;`-separated `start[:end]` segments; on opens one at `at`, off closes the last open one.
pub(crate) fn follow_all_rc(existing: &str, on: bool, at: i64) -> String {
    let mut segments: Vec<String> = existing
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let open = segments.last().is_some_and(|s| !s.contains(':'));
    match (on, open) {
        (true, false) => segments.push(at.to_string()),
        (false, true) => {
            let last = segments.pop().expect("open segment");
            segments.push(format!("{last}:{at}"));
        }
        _ => {}
    }
    let skip = segments.len().saturating_sub(15);
    segments[skip..].join(";")
}
pub(crate) fn follows_all(follow_all_rc: &str) -> bool {
    follow_all_rc
        .split(';')
        .map(str::trim)
        .rfind(|s| !s.is_empty())
        .is_some_and(|s| !s.contains(':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs(value: Value) -> Prefs {
        Prefs {
            raw: value.as_object().unwrap().clone(),
        }
    }
    fn candidate(kind: Kind) -> Candidate {
        Candidate {
            kind,
            conversation_id: "19:a@thread.v2".into(),
            message_id: "1".into(),
            reply_chain_id: None,
            from_me: false,
            title: "Alice".into(),
            subtitle: None,
            body: "Salut".into(),
            time_ms: 1_000_000,
            urgent: false,
            deleted: false,
            display_banner: None,
            from_activity: false,
        }
    }
    fn context(prefs: &Prefs) -> Context<'_> {
        Context {
            prefs,
            now: DateTime::from_timestamp_millis(1_000_000 + 1000).unwrap(),
            conversation: ConversationRules::default(),
            visible: false,
            in_call: false,
            muted_in_call: false,
            duplicate: false,
        }
    }

    #[test]
    fn reads_the_serialized_property_and_writes_it_back_as_a_string() {
        let prefs = Prefs::from_properties(&json!({
            "userPersonalSettings": "{\"toastSettings\":{\"Chat\":false},\"other\":{\"x\":1}}"
        }))
        .unwrap();
        assert_eq!(prefs.choice(NotificationRow::Chat), NotificationChoice::Off);
        let mut prefs = prefs;
        prefs.set_choice(NotificationRow::Chat, NotificationChoice::OnlyFeed);
        let body = prefs.serialized();
        let back: Value = serde_json::from_str(body.as_str().unwrap()).unwrap();
        assert_eq!(back["toastSettings"]["Chat"], false);
        assert_eq!(back["activityGenerationSettings"]["Chat"], true);
        assert_eq!(back["emailSettings"]["Chat"], false);
        assert_eq!(back["other"]["x"], 1, "unknown categories survive");
        assert_eq!(back["selectedPreset"], 3);
    }

    #[test]
    fn aggregated_rows_write_every_key() {
        let mut prefs = Prefs::default();
        prefs.set_choice(NotificationRow::PersonalMentions, NotificationChoice::Off);
        prefs.set_choice(NotificationRow::Replies, NotificationChoice::BannerAndFeed);
        for key in ["MentionInChat_Person_v2", "Mention_Person"] {
            assert_eq!(prefs.get(TOAST, key), Some(&json!(false)));
            assert_eq!(prefs.get(ACTIVITY, key), Some(&json!(false)));
        }
        for key in ["Reply", "ReplyToReply"] {
            assert_eq!(prefs.get(EMAIL, key), Some(&json!(true)));
        }
    }

    #[test]
    fn presets_rewrite_lines_and_store_the_business_code() {
        let mut prefs = Prefs::default();
        prefs.apply_preset(NotificationPreset::MyAtMentions);
        assert_eq!(prefs.raw["selectedPreset"], 2);
        assert_eq!(prefs.preset(), NotificationPreset::MyAtMentions);
        assert!(prefs.banner(NotificationRow::PersonalMentions));
        assert_eq!(
            prefs.choice(NotificationRow::Reactions),
            NotificationChoice::OnlyFeed
        );
        assert_eq!(
            self::prefs(json!({"selectedPreset": "AllActivity"})).preset(),
            NotificationPreset::AllActivity
        );
    }

    #[test]
    fn generic_mute_also_allows_urgent_banners() {
        let mut prefs = Prefs::default();
        prefs.set_toggle(NotificationToggle::OnlyUrgent, true);
        assert_eq!(
            prefs.get(TOAST, "muteGenericNotifications"),
            Some(&json!("On"))
        );
        assert_eq!(
            prefs.get(TOAST, "allowUrgentNotifications"),
            Some(&json!("On"))
        );
        let ctx = context(&prefs);
        assert_eq!(
            decide(&candidate(Kind::Chat), &ctx),
            Err(Rejection::NotEnabledByPreferences)
        );
        let mut urgent = candidate(Kind::Chat);
        urgent.urgent = true;
        assert_eq!(decide(&urgent, &ctx), Ok(()));
    }

    #[test]
    fn muted_conversations_let_mentions_and_urgent_messages_through() {
        let prefs = Prefs::default();
        let mut ctx = context(&prefs);
        ctx.conversation.muted = true;
        assert_eq!(
            decide(&candidate(Kind::Chat), &ctx),
            Err(Rejection::ConversationMuted)
        );
        assert_eq!(decide(&candidate(Kind::PersonalMention), &ctx), Ok(()));
        let mut urgent = candidate(Kind::Chat);
        urgent.urgent = true;
        assert_eq!(decide(&urgent, &ctx), Ok(()));
    }

    #[test]
    fn server_hints_are_strings_not_booleans() {
        let prefs = prefs(json!({"toastSettings": {"Chat": false}}));
        let ctx = context(&prefs);
        let mut c = candidate(Kind::Chat);
        c.display_banner = Some("false".into());
        assert_eq!(decide(&c, &ctx), Err(Rejection::NotEnabledByDisplayBanner));
        c.display_banner = Some("true".into());
        assert_eq!(decide(&c, &ctx), Ok(()), "the server overrides preferences");
        c.display_banner = Some("GNS".into());
        assert_eq!(decide(&c, &ctx), Err(Rejection::NotEnabledByPreferences));
    }

    #[test]
    fn conversation_desktop_setting_overrides_the_global_one() {
        let prefs = prefs(json!({"toastSettings": {"Chat": false}}));
        let mut ctx = context(&prefs);
        ctx.conversation.desktop = Some(true);
        assert_eq!(decide(&candidate(Kind::Chat), &ctx), Ok(()));
        ctx.conversation.desktop = Some(false);
        assert_eq!(
            decide(&candidate(Kind::Chat), &ctx),
            Err(Rejection::NotEnabledByPreferences)
        );
        assert_eq!(decide(&candidate(Kind::PersonalMention), &ctx), Ok(()));
    }

    #[test]
    fn channel_settings_decide_posts() {
        let prefs = Prefs::default();
        let mut ctx = context(&prefs);
        assert_eq!(
            decide(&candidate(Kind::ChannelPost), &ctx),
            Err(Rejection::NotEnabledByPreferences),
            "posts are feed-only by default"
        );
        ctx.conversation.channel_banner = Some(true);
        assert_eq!(decide(&candidate(Kind::ChannelPost), &ctx), Ok(()));
    }

    #[test]
    fn age_duplicates_pause_meetings_and_visibility_reject_in_order() {
        let mut prefs = Prefs::default();
        let mut ctx = context(&prefs);
        let mut old = candidate(Kind::Chat);
        old.time_ms -= MAX_AGE_MS + 1;
        assert_eq!(decide(&old, &ctx), Err(Rejection::TooOld));
        ctx.duplicate = true;
        assert_eq!(
            decide(&candidate(Kind::Chat), &ctx),
            Err(Rejection::Duplicate)
        );
        ctx.duplicate = false;
        ctx.in_call = true;
        ctx.muted_in_call = true;
        assert_eq!(
            decide(&candidate(Kind::Chat), &ctx),
            Err(Rejection::DuringMeeting)
        );
        ctx.in_call = false;
        ctx.visible = true;
        assert_eq!(
            decide(&candidate(Kind::Chat), &ctx),
            Err(Rejection::ConversationVisible)
        );
        let now = ctx.now;
        prefs.set_paused_until(Some(now + chrono::Duration::minutes(10)));
        let ctx = Context {
            visible: false,
            ..context(&prefs)
        };
        assert_eq!(decide(&candidate(Kind::Chat), &ctx), Err(Rejection::Paused));
    }

    #[test]
    fn sounds_follow_busy_and_urgent_settings() {
        let mut prefs = Prefs::default();
        assert_eq!(
            sound(&prefs, false, false),
            Some(NotificationSound::Default)
        );
        assert_eq!(sound(&prefs, false, true), None, "silent while busy");
        assert_eq!(
            sound(&prefs, true, true),
            Some(NotificationSound::DefaultUrgent)
        );
        prefs.set_sound(false, NotificationSound::Ping);
        assert_eq!(sound(&prefs, false, false), Some(NotificationSound::Ping));
        prefs.set_toggle(NotificationToggle::Sounds, false);
        assert_eq!(prefs.get(SOUNDS, "alertSound"), Some(&json!("Off")));
        assert_eq!(sound(&prefs, true, false), None);
    }

    #[test]
    fn follow_all_segments_open_close_and_cap_at_fifteen() {
        assert_eq!(follow_all_rc("", true, 10), "10");
        assert_eq!(follow_all_rc("10", true, 20), "10", "already open");
        assert_eq!(follow_all_rc("10", false, 20), "10:20");
        assert!(!follows_all("10:20"));
        assert!(follows_all("10:20;30"));
        let many: Vec<String> = (0..15).map(|i| format!("{i}:{i}")).collect();
        let next = follow_all_rc(&many.join(";"), true, 99);
        assert_eq!(next.split(';').count(), 15);
        assert!(next.ends_with(";99"));
        assert!(next.starts_with("1:1"));
    }

    #[test]
    fn pause_round_trips_as_iso_and_expires() {
        let mut prefs = Prefs::default();
        let now = Utc::now();
        prefs.set_paused_until(Some(now + chrono::Duration::hours(1)));
        assert!(prefs.paused_until(now).is_some());
        assert!(
            prefs
                .paused_until(now + chrono::Duration::hours(2))
                .is_none()
        );
        prefs.set_paused_until(None);
        assert!(prefs.get(TOAST, "temporarilyPauseUntil").is_none());
    }

    #[test]
    fn meeting_chat_mute_follows_rsvp_settings() {
        let prefs = Prefs::default();
        assert!(!prefs.meeting_chat_muted(Some(Rsvp::Organizer), false));
        assert!(
            !prefs.meeting_chat_muted(Some(Rsvp::Accepted), false),
            "accepted: all"
        );
        assert!(
            prefs.meeting_chat_muted(Some(Rsvp::Tentative), false),
            "until I join"
        );
        assert!(!prefs.meeting_chat_muted(Some(Rsvp::NoResponse), true));
        assert!(prefs.meeting_chat_muted(Some(Rsvp::Declined), true));
        assert!(
            prefs.meeting_chat_muted(None, false),
            "general default: participated"
        );

        let v2 = self::prefs(json!({"chatSettings": {
            "meetingChatNotificationRSVPv2Declined": true,
            "meetingChatNotificationRSVPv2Accepted": "Off",
        }}));
        assert!(!v2.meeting_chat_muted(Some(Rsvp::Declined), false));
        assert!(v2.meeting_chat_muted(Some(Rsvp::Accepted), true));
        assert!(
            v2.meeting_chat_muted(Some(Rsvp::Tentative), false),
            "falls back"
        );
        assert_eq!(
            Rsvp::from_response("TentativelyAccepted", false),
            Rsvp::Tentative
        );
        assert_eq!(Rsvp::from_response("Accepted", true), Rsvp::Organizer);
    }

    #[test]
    fn general_meeting_choice_keeps_stored_rsvp_v2_values_in_step() {
        let mut prefs = prefs(json!({"chatSettings": {
            "meetingChatNotificationRSVPv2Accepted": false,
            "meetingChatNotificationRSVPv2Declined": false,
        }}));
        prefs.set_meeting_chats(MeetingChats::All);
        assert_eq!(
            prefs.get(CHAT, "meetingChatNotificationRSVPv2Accepted"),
            Some(&json!(true))
        );
        assert_eq!(
            prefs.get(CHAT, "meetingChatNotificationRSVPv2Declined"),
            Some(&json!(false)),
            "declined meetings stay muted"
        );
        assert!(
            prefs
                .get(CHAT, "meetingChatNotificationRSVPv2Tentative")
                .is_none()
        );
        prefs.set_meeting_chats(MeetingChats::Participated);
        assert!(
            prefs
                .get(CHAT, "meetingChatNotificationRSVPv2Accepted")
                .is_none()
        );
        assert!(prefs.meeting_chat_muted(Some(Rsvp::Accepted), false));
    }

    #[test]
    fn meeting_chats_accept_storage_and_ui_values() {
        let prefs = prefs(json!({"chatSettings": {"meetingChatNotification": "MuteChat"}}));
        assert_eq!(prefs.meeting_chats(), MeetingChats::None);
        let mut prefs = Prefs::default();
        prefs.set_meeting_chats(MeetingChats::All);
        assert_eq!(
            prefs.get(CHAT, "meetingChatNotificationForTentativeMeetings"),
            Some(&json!("all"))
        );
    }
}
