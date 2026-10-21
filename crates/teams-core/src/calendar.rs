//! Calendar state and native view-models. Request identities prevent stale UI updates.
use super::*;
use chrono::{Datelike, NaiveDate};
use teams_calendar::recurrence::{week_index, weekday_number};
use teams_calendar::{CalendarClient, Event, Pattern, Recurrence, Response};
use teams_i18n::t;

/// The repeat menu of the event form, as the web client lists it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum Repeat {
    #[default]
    Never,
    /// Monday to Friday.
    Weekdays,
    Daily,
    /// On the start's weekday.
    Weekly,
    /// On the start's Nth weekday ("last" in the month's last seven days).
    Monthly,
    /// On the start's day and month.
    Yearly,
    /// Every `interval` units, on the chosen weekdays or month day.
    Custom,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum RepeatUnit {
    #[default]
    Day,
    Week,
    Month,
    Year,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct RecurrenceDraft {
    pub repeat: Repeat,
    /// For `Custom`.
    pub unit: RepeatUnit,
    pub interval: u32,
    /// 0 (Sunday) to 6, for a custom weekly repeat.
    pub weekdays: Vec<u32>,
    /// Months and years: the Nth weekday rather than the day number.
    pub by_weekday: bool,
    /// `YYYY-MM-DD` of the last occurrence; empty never ends.
    pub end_date: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct CalendarPerson {
    pub address: String,
    pub name: String,
    pub optional: bool,
    /// The service's answer (`Accepted`, `TentativelyAccepted`, `Declined`, `None`…); empty in a draft.
    #[uniffi(default = "")]
    pub response: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct CalendarDraft {
    pub subject: String,
    pub start: String,
    pub end: String,
    pub time_zone: String,
    pub all_day: bool,
    pub location: String,
    pub description: String,
    pub online: bool,
    pub attendees: Vec<CalendarPerson>,
    pub recurrence: RecurrenceDraft,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct CalendarEventView {
    pub id: String,
    pub form: CalendarDraft,
    pub organizer: String,
    pub response: String,
    pub cancelled: bool,
    pub recurring: bool,
    /// One date of a series: edits and answers can go to it alone or to the series.
    pub occurrence: bool,
    /// "Every week on Monday, until 21/12/2026"; empty for a single event.
    pub recurrence_text: String,
    pub can_edit: bool,
    pub can_cancel: bool,
    pub can_respond: bool,
    pub can_delete: bool,
    pub join_url: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct CalendarView {
    pub start: String,
    pub end: String,
    pub events: Vec<CalendarEventView>,
    pub selected: Option<CalendarEventView>,
    pub draft: Option<CalendarDraft>,
    pub editing: bool,
    /// The form edits one date of a series: its repeat cannot change.
    pub editing_occurrence: bool,
    pub loading: bool,
    pub loading_detail: bool,
    pub busy: bool,
    pub uncertain: bool,
    pub error: Option<String>,
    pub people: Vec<CalendarPerson>,
    pub searching: bool,
}
#[derive(Debug, Clone, uniffi::Enum)]
pub enum CalendarAction {
    Range {
        start: String,
        end: String,
    },
    Refresh,
    Select {
        id: String,
    },
    CloseDetail,
    New {
        start: String,
        end: String,
        time_zone: String,
    },
    /// `series`: the whole series of an occurrence (its master is loaded first).
    Edit {
        series: bool,
    },
    Draft {
        value: CalendarDraft,
    },
    CloseEditor,
    Save,
    Delete {
        series: bool,
    },
    Cancel {
        series: bool,
    },
    Respond {
        response: String,
        series: bool,
    },
    SearchPeople {
        query: String,
    },
}
#[derive(Debug, Clone)]
pub(crate) enum CalendarMsg {
    Action(CalendarAction),
    Listed(u64, u64, Result<Vec<Event>, String>),
    Detail(u64, u64, Result<Box<Event>, String>),
    /// The series master, to edit the whole series.
    Series(u64, u64, Result<Box<Event>, String>),
    Mutated(u64, Result<(), (String, bool)>),
    People(u64, u64, Result<Vec<Person>, String>),
}
#[derive(Default)]
pub(crate) struct CalendarState {
    client: Option<CalendarClient<AppAuthenticator>>,
    pub view: CalendarView,
    events: Vec<Event>,
    selected: Option<Event>,
    editing: Option<Event>,
    list_request: u64,
    detail_request: u64,
    people_request: u64,
    pub(crate) refreshed: Option<std::time::Instant>,
}
impl CalendarState {
    /// The language changed: event views are built again from the loaded events.
    pub(crate) fn relocalize(&mut self) {
        self.view.events = self.events.iter().map(event_view).collect();
        if let (Some(view), Some(event)) = (&mut self.view.selected, &self.selected) {
            *view = event_view(event);
        }
    }
    /// "3 nov., 09:00 - 17:00" for a loaded event, as on an activity row.
    pub(crate) fn event_range(&self, id: &str) -> Option<String> {
        let event = self.events.iter().find(|e| e.object_id == id)?;
        meeting_range(&event.start_time, &event.end_time, event.is_all_day_event)
    }
    /// The user's answer to the loaded meeting whose chat is `thread`, and whether they organize it.
    pub(crate) fn meeting_response(&self, thread: &str) -> Option<(String, bool)> {
        self.events.iter().find_map(|e| {
            match teams_calling::MeetingLink::parse(join_url(e))? {
                teams_calling::MeetingLink::Coordinates(m) if m.thread_id == thread => {
                    Some((e.my_response_type.clone(), e.is_organizer))
                }
                _ => None,
            }
        })
    }
    /// Finds and selects the meeting of a notification that names no event, by time range then subject.
    pub(crate) fn find_meeting(
        &mut self,
        ctx: &Ctx,
        start: String,
        end: String,
        subject: String,
        map: impl FnOnce(Result<Option<Event>, String>) -> Msg + Send + 'static,
    ) -> Task<Msg> {
        let Some(client) = self.client(ctx) else {
            return Task::none();
        };
        self.detail_request += 1;
        self.selected = None;
        self.view.selected = None;
        self.view.loading_detail = true;
        Task::perform(
            async move {
                let events = client.list(&start, &end).await.map_err(|e| e.to_string())?;
                Ok(pick_meeting(events, &start, &subject))
            },
            map,
        )
    }
    pub(crate) fn show_found_meeting(&mut self, found: Result<Option<Event>, String>) {
        self.view.loading_detail = false;
        match found {
            Ok(Some(event)) => {
                self.view.selected = Some(event_view(&event));
                self.selected = Some(event);
            }
            Ok(None) => {
                self.view.error = Some(t("core.calendar.meeting_not_found_in_your_calendar").into())
            }
            Err(error) => self.view.error = Some(error),
        }
    }
}
/// Same subject (ignoring case and spaces) wins, then the same start time.
pub(crate) fn pick_meeting(events: Vec<Event>, start: &str, subject: &str) -> Option<Event> {
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let subject = norm(subject);
    let start = local_time(start);
    let same_start = |e: &Event| start.is_some() && local_time(&e.start_time) == start;
    let mut events: Vec<Event> = events
        .into_iter()
        .filter(|e| !e.object_id.is_empty())
        .collect();
    if let Some(i) = events
        .iter()
        .position(|e| !subject.is_empty() && norm(&e.subject) == subject)
    {
        return Some(events.swap_remove(i));
    }
    let i = events.iter().position(same_start)?;
    Some(events.swap_remove(i))
}
/// Local time of a calendar timestamp: RFC 3339, or UTC without an offset.
fn local_time(value: &str) -> Option<chrono::DateTime<Local>> {
    DateTime::parse_from_rfc3339(value)
        .map(|t| t.with_timezone(&Local))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(
                value.trim_end_matches('Z'),
                "%Y-%m-%dT%H:%M:%S%.f",
            )
            .ok()
            .map(|t| t.and_utc().with_timezone(&Local))
        })
}
/// "3 nov., 09:00 - 17:00", "3 nov. - 4 nov., 09:00 - 17:00" over several days, the day alone if all-day.
pub(crate) fn meeting_range(start: &str, end: &str, all_day: bool) -> Option<String> {
    let start = local_time(start)?;
    let day = |time: &chrono::DateTime<Local>| {
        let month = t(&format!("core.calendar.month.{}", time.month()));
        tf!("core.calendar.day_month", day = time.day(), month = month)
    };
    if all_day {
        return Some(day(&start));
    }
    let Some(end) = local_time(end) else {
        return Some(format!("{}, {}", day(&start), start.format("%H:%M")));
    };
    let days = if start.date_naive() == end.date_naive() {
        day(&start)
    } else {
        format!("{} - {}", day(&start), day(&end))
    };
    Some(format!(
        "{days}, {} - {}",
        start.format("%H:%M"),
        end.format("%H:%M")
    ))
}
/// The form's repeat choice for a series' recurrence: a preset when it is one.
fn recurrence_draft(r: &Recurrence, start: Option<NaiveDate>) -> RecurrenceDraft {
    let custom = |unit, interval, weekdays: Vec<u8>, by_weekday| RecurrenceDraft {
        repeat: Repeat::Custom,
        unit,
        interval,
        weekdays: weekdays.into_iter().map(u32::from).collect(),
        by_weekday,
        end_date: String::new(),
    };
    let mut draft = match &r.pattern {
        Pattern::Daily { interval: 1 } => RecurrenceDraft {
            repeat: Repeat::Daily,
            interval: 1,
            ..Default::default()
        },
        Pattern::Daily { interval } => custom(RepeatUnit::Day, *interval, vec![], false),
        Pattern::Weekly { interval: 1, days } if days == &[1, 2, 3, 4, 5] => RecurrenceDraft {
            repeat: Repeat::Weekdays,
            unit: RepeatUnit::Week,
            interval: 1,
            weekdays: days.iter().map(|d| u32::from(*d)).collect(),
            ..Default::default()
        },
        Pattern::Weekly { interval: 1, days }
            if start.is_some_and(|s| days == &[weekday_number(s)]) =>
        {
            RecurrenceDraft {
                repeat: Repeat::Weekly,
                unit: RepeatUnit::Week,
                interval: 1,
                weekdays: days.iter().map(|d| u32::from(*d)).collect(),
                ..Default::default()
            }
        }
        Pattern::Weekly { interval, days } => custom(RepeatUnit::Week, *interval, days.clone(), false),
        Pattern::RelativeMonthly { interval: 1, .. } => RecurrenceDraft {
            repeat: Repeat::Monthly,
            unit: RepeatUnit::Month,
            interval: 1,
            by_weekday: true,
            ..Default::default()
        },
        Pattern::RelativeMonthly { interval, .. } => custom(RepeatUnit::Month, *interval, vec![], true),
        Pattern::Monthly { interval, .. } => custom(RepeatUnit::Month, *interval, vec![], false),
        Pattern::Yearly { .. } => RecurrenceDraft {
            repeat: Repeat::Yearly,
            unit: RepeatUnit::Year,
            interval: 1,
            ..Default::default()
        },
        Pattern::RelativeYearly { .. } => custom(RepeatUnit::Year, 1, vec![], true),
    };
    draft.end_date = r.end.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
    draft
}
/// The recurrence the form describes, from the event's start (local date).
pub(crate) fn recurrence(d: &RecurrenceDraft, start: &str) -> Result<Option<Recurrence>, String> {
    let Some(start) = local_time(start).map(|t| t.date_naive()) else {
        return Ok(None);
    };
    let weekday = weekday_number(start);
    let index = week_index(start);
    let interval = d.interval.clamp(1, 99);
    let pattern = match (d.repeat, d.unit) {
        (Repeat::Never, _) => return Ok(None),
        (Repeat::Weekdays, _) => Pattern::Weekly {
            interval: 1,
            days: vec![1, 2, 3, 4, 5],
        },
        (Repeat::Daily, _) => Pattern::Daily { interval: 1 },
        (Repeat::Weekly, _) => Pattern::Weekly {
            interval: 1,
            days: vec![weekday],
        },
        (Repeat::Monthly, _) => Pattern::RelativeMonthly {
            interval: 1,
            weekday,
            index,
        },
        (Repeat::Yearly, _) => Pattern::Yearly {
            month: start.month(),
            day: start.day(),
        },
        (Repeat::Custom, RepeatUnit::Day) => Pattern::Daily { interval },
        (Repeat::Custom, RepeatUnit::Week) => {
            let mut days: Vec<u8> = d
                .weekdays
                .iter()
                .filter(|d| **d < 7)
                .map(|d| *d as u8)
                .collect();
            days.sort_unstable();
            days.dedup();
            if days.is_empty() {
                days.push(weekday);
            }
            Pattern::Weekly { interval, days }
        }
        (Repeat::Custom, RepeatUnit::Month) if d.by_weekday => Pattern::RelativeMonthly {
            interval,
            weekday,
            index,
        },
        (Repeat::Custom, RepeatUnit::Month) => Pattern::Monthly {
            interval,
            day: start.day(),
        },
        (Repeat::Custom, RepeatUnit::Year) if d.by_weekday => Pattern::RelativeYearly {
            month: start.month(),
            weekday,
            index,
        },
        (Repeat::Custom, RepeatUnit::Year) => Pattern::Yearly {
            month: start.month(),
            day: start.day(),
        },
    };
    let end = if d.end_date.trim().is_empty() {
        None
    } else {
        let end = NaiveDate::parse_from_str(d.end_date.trim(), "%Y-%m-%d")
            .map_err(|_| t("core.calendar.invalid_end_of_series").to_owned())?;
        if end < start {
            return Err(t("core.calendar.series_ends_before_it_starts").into());
        }
        Some(end)
    };
    Ok(Some(Recurrence { pattern, start, end }))
}
fn weekday_name(day: u8) -> &'static str {
    t(&format!("core.calendar.weekday.{day}"))
}
/// A sentence for the series, as the event details show it.
fn recurrence_text(r: &Recurrence) -> String {
    let index = |i: u8| t(&format!("core.calendar.week_index.{i}"));
    let month = |m: u32| t(&format!("core.calendar.month.{m}"));
    let every = |unit_one: &str, unit_many: &str, n: u32| {
        if n <= 1 {
            t(unit_one).to_owned()
        } else {
            teams_i18n::fill(t(unit_many), &[("count", n.to_string())])
        }
    };
    let text = match &r.pattern {
        Pattern::Daily { interval } => every("core.calendar.every_day", "core.calendar.every_n_days", *interval),
        Pattern::Weekly { interval, days } => {
            let names: Vec<_> = days.iter().map(|d| weekday_name(*d)).collect();
            tf!(
                "core.calendar.weekly_on",
                every = every("core.calendar.every_week", "core.calendar.every_n_weeks", *interval),
                days = names.join(", ")
            )
        }
        Pattern::Monthly { interval, day } => tf!(
            "core.calendar.monthly_on_day",
            every = every("core.calendar.every_month", "core.calendar.every_n_months", *interval),
            day = day
        ),
        Pattern::RelativeMonthly {
            interval,
            weekday,
            index: i,
        } => tf!(
            "core.calendar.monthly_on_weekday",
            every = every("core.calendar.every_month", "core.calendar.every_n_months", *interval),
            index = index(*i),
            weekday = weekday_name(*weekday)
        ),
        Pattern::Yearly { month: m, day } => {
            tf!("core.calendar.yearly_on", day = day, month = month(*m))
        }
        Pattern::RelativeYearly {
            month: m,
            weekday,
            index: i,
        } => tf!(
            "core.calendar.yearly_on_weekday",
            index = index(*i),
            weekday = weekday_name(*weekday),
            month = month(*m)
        ),
    };
    match r.end {
        Some(end) => tf!(
            "core.calendar.until",
            text = text,
            date = end.format(t("core.me.date_format")).to_string()
        ),
        None => text,
    }
}
fn draft(e: &Event) -> CalendarDraft {
    let description = if e.body_content_type.eq_ignore_ascii_case("HTML") {
        html::to_text(&e.body_content)
    } else {
        e.body_content.clone()
    };
    CalendarDraft {
        subject: e.subject.clone(),
        start: e.start_time.clone(),
        end: e.end_time.clone(),
        time_zone: e.event_time_zone.clone(),
        all_day: e.is_all_day_event,
        location: e.location.clone(),
        description,
        online: e.is_online_meeting,
        attendees: e
            .attendees
            .iter()
            .filter(|a| matches!(a.kind.as_str(), "Required" | "Optional"))
            .map(|a| CalendarPerson {
                address: a.address.clone(),
                name: a.name.clone(),
                optional: a.kind == "Optional",
                response: a
                    .extra
                    .get("status")
                    .and_then(|s| s.get("response"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
            .collect(),
        recurrence: Recurrence::of(e)
            .map(|r| recurrence_draft(&r, local_time(&e.start_time).map(|t| t.date_naive())))
            .unwrap_or_default(),
    }
}
fn join_url(e: &Event) -> &str {
    if e.join_online_meeting_url.is_empty() {
        e.extra
            .get("skypeTeamsMeetingUrl")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
    } else {
        &e.join_online_meeting_url
    }
}
fn event_view(e: &Event) -> CalendarEventView {
    CalendarEventView {
        id: e.object_id.clone(),
        form: draft(e),
        organizer: if e.organizer_name.is_empty() {
            e.organizer_address.clone()
        } else {
            e.organizer_name.clone()
        },
        response: e.my_response_type.clone(),
        cancelled: e.is_cancelled,
        recurring: e.recurring(),
        occurrence: e.is_occurrence() && e.series_id().is_some(),
        recurrence_text: Recurrence::of(e).map(|r| recurrence_text(&r)).unwrap_or_default(),
        can_edit: e.is_organizer && !e.is_cancelled,
        can_respond: !e.is_organizer && !e.is_cancelled && e.is_response_requested,
        can_cancel: e.is_organizer
            && !e.is_cancelled
            && (e.is_online_meeting || !e.attendees.is_empty()),
        can_delete: !e.is_organizer
            || e.is_cancelled
            || (!e.is_online_meeting && e.attendees.is_empty()),
        join_url: join_url(e).to_owned(),
    }
}
impl CalendarState {
    fn client(&mut self, ctx: &Ctx) -> Option<CalendarClient<AppAuthenticator>> {
        if self.client.is_none() {
            self.client = ctx.client.map(|c| CalendarClient::new(c.authenticator()));
        }
        self.client.clone()
    }
    pub(crate) fn load(&mut self, ctx: &Ctx, force: bool) -> Task<Msg> {
        if ctx.nav != Nav::Calendar
            || self.view.start.is_empty()
            || (!force
                && (self.view.loading
                    || self
                        .refreshed
                        .is_some_and(|t| t.elapsed() < Duration::from_secs(90))))
        {
            return Task::none();
        }
        let Some(client) = self.client(ctx) else {
            return Task::none();
        };
        self.list_request += 1;
        let request = self.list_request;
        let g = ctx.generation;
        let start = self.view.start.clone();
        let end = self.view.end.clone();
        self.view.loading = true;
        Task::perform(
            async move { client.list(&start, &end).await.map_err(|e| e.to_string()) },
            move |r| Msg::Calendar(CalendarMsg::Listed(g, request, r)),
        )
    }
    pub(crate) fn update(&mut self, ctx: &Ctx, msg: CalendarMsg) -> Task<Msg> {
        match msg {
            CalendarMsg::Listed(g, r, result) => {
                if g != ctx.generation || r != self.list_request {
                    return Task::none();
                }
                self.view.loading = false;
                self.refreshed = Some(std::time::Instant::now());
                match result {
                    Ok(events) => {
                        self.view.events = events.iter().map(event_view).collect();
                        self.events = events;
                    }
                    Err(e) => {
                        if !self.view.uncertain {
                            self.view.error = Some(e);
                        }
                    }
                }
            }
            CalendarMsg::Detail(g, r, result) => {
                if g != ctx.generation || r != self.detail_request {
                    return Task::none();
                }
                self.view.loading_detail = false;
                match result {
                    Ok(e) => {
                        self.view.selected = Some(event_view(&e));
                        self.selected = Some(*e);
                    }
                    Err(e) => self.view.error = Some(e),
                }
            }
            CalendarMsg::Series(g, r, result) => {
                if g != ctx.generation || r != self.detail_request {
                    return Task::none();
                }
                self.view.loading_detail = false;
                match result {
                    Ok(master) if master.is_organizer && !master.is_cancelled => {
                        self.view.draft = Some(draft(&master));
                        self.editing = Some(*master);
                        self.view.editing = true;
                        self.view.editing_occurrence = false;
                        self.view.error = None;
                    }
                    Ok(_) => {
                        self.view.error = Some(t("core.calendar.only_the_organizer_edits_the_series").into())
                    }
                    Err(e) => self.view.error = Some(e),
                }
            }
            CalendarMsg::Mutated(g, result) => {
                if g != ctx.generation {
                    return Task::none();
                }
                self.view.busy = false;
                match result {
                    Ok(()) => {
                        self.view.draft = None;
                        self.editing = None;
                        self.view.editing = false;
                        self.selected = None;
                        self.view.selected = None;
                        self.view.error = None;
                    }
                    Err((e, uncertain)) => {
                        self.view.error = Some(e);
                        self.view.uncertain = uncertain;
                    }
                }
                return self.load(ctx, true);
            }
            CalendarMsg::People(g, r, result) => {
                if g != ctx.generation || r != self.people_request {
                    return Task::none();
                }
                self.view.searching = false;
                match result {
                    Ok(people) => {
                        self.view.people = people
                            .into_iter()
                            .filter(|p| !p.email.is_empty())
                            .map(|p| CalendarPerson {
                                address: p.email.clone(),
                                name: p.name().to_owned(),
                                optional: false,
                                response: String::new(),
                            })
                            .collect()
                    }
                    Err(e) => self.view.error = Some(e),
                }
            }
            CalendarMsg::Action(action) => {
                if ctx.client.is_none() {
                    return Task::none();
                }
                match action {
                    CalendarAction::Range { start, end } => {
                        let valid = DateTime::parse_from_rfc3339(&start)
                            .ok()
                            .zip(DateTime::parse_from_rfc3339(&end).ok())
                            .is_some_and(|(s, e)| e > s && e - s <= chrono::Duration::days(62));
                        if !valid {
                            self.view.error =
                                Some(t("core.calendar.invalid_calendar_period").into());
                            return Task::none();
                        }
                        if self.view.start != start || self.view.end != end {
                            self.view.start = start;
                            self.view.end = end;
                            self.view.events.clear();
                            self.events.clear();
                            return self.load(ctx, true);
                        }
                        return self.load(ctx, false);
                    }
                    CalendarAction::Refresh => {
                        if !self.view.uncertain {
                            self.view.error = None;
                        }
                        return self.load(ctx, true);
                    }
                    CalendarAction::Select { id } => {
                        if self.view.busy || self.view.draft.is_some() {
                            return Task::none();
                        }
                        let Some(client) = self.client(ctx) else {
                            return Task::none();
                        };
                        self.detail_request += 1;
                        let r = self.detail_request;
                        let g = ctx.generation;
                        self.selected = None;
                        self.view.selected = None;
                        self.view.loading_detail = true;
                        return Task::perform(
                            async move {
                                client
                                    .get(&id)
                                    .await
                                    .map(Box::new)
                                    .map_err(|e| e.to_string())
                            },
                            move |v| Msg::Calendar(CalendarMsg::Detail(g, r, v)),
                        );
                    }
                    CalendarAction::CloseDetail => {
                        if !self.view.busy {
                            self.detail_request += 1;
                            self.selected = None;
                            self.view.selected = None;
                            self.view.loading_detail = false;
                            self.view.uncertain = false;
                        }
                    }
                    CalendarAction::New {
                        start,
                        end,
                        time_zone,
                    } => {
                        if self.view.busy || self.view.draft.is_some() {
                            return Task::none();
                        }
                        self.detail_request += 1;
                        self.view.loading_detail = false;
                        self.editing = None;
                        self.view.editing = false;
                        self.view.editing_occurrence = false;
                        self.view.uncertain = false;
                        self.view.error = None;
                        self.view.draft = Some(CalendarDraft {
                            start,
                            end,
                            time_zone,
                            online: true,
                            ..Default::default()
                        });
                    }
                    CalendarAction::Edit { series } => {
                        if self.view.busy || self.view.uncertain {
                            return Task::none();
                        }
                        let master = self
                            .selected
                            .as_ref()
                            .filter(|e| series && e.is_occurrence())
                            .and_then(|e| e.series_id())
                            .map(str::to_owned);
                        if let Some(id) = master {
                            let Some(client) = self.client(ctx) else {
                                return Task::none();
                            };
                            self.detail_request += 1;
                            let r = self.detail_request;
                            let g = ctx.generation;
                            self.view.loading_detail = true;
                            return Task::perform(
                                async move {
                                    client.get(&id).await.map(Box::new).map_err(|e| e.to_string())
                                },
                                move |v| Msg::Calendar(CalendarMsg::Series(g, r, v)),
                            );
                        }
                        if let Some(e) = self.selected.as_ref().filter(|e| event_view(e).can_edit) {
                            self.editing = Some(e.clone());
                            self.view.draft = Some(draft(e));
                            self.view.editing = true;
                            self.view.editing_occurrence = e.is_occurrence();
                            self.view.error = None;
                        }
                    }
                    CalendarAction::Draft { value } => {
                        if !self.view.busy {
                            self.view.draft = Some(value);
                        }
                    }
                    CalendarAction::CloseEditor => {
                        if !self.view.busy {
                            self.view.draft = None;
                            self.editing = None;
                            self.view.editing = false;
                            self.view.people.clear();
                            self.people_request += 1;
                            self.view.searching = false;
                            self.view.uncertain = false;
                        }
                    }
                    CalendarAction::SearchPeople { query } => {
                        self.people_request += 1;
                        let r = self.people_request;
                        self.view.people.clear();
                        self.view.searching = false;
                        if query.trim().len() < 2 || self.view.draft.is_none() {
                            return Task::none();
                        }
                        let client = ctx.client.cloned().unwrap();
                        let g = ctx.generation;
                        self.view.searching = true;
                        return Task::perform(
                            async move {
                                client
                                    .search_people(&query)
                                    .await
                                    .map_err(|e| e.to_string())
                            },
                            move |v| Msg::Calendar(CalendarMsg::People(g, r, v)),
                        );
                    }
                    mutation @ (CalendarAction::Save
                    | CalendarAction::Delete { .. }
                    | CalendarAction::Cancel { .. }
                    | CalendarAction::Respond { .. }) => {
                        if self.view.busy || self.view.uncertain {
                            return Task::none();
                        }
                        let Some(client) = self.client(ctx) else {
                            return Task::none();
                        };
                        let mut event = self.selected.clone().unwrap_or_default();
                        if matches!(mutation, CalendarAction::Save) {
                            let Some(d) = self.view.draft.clone() else {
                                return Task::none();
                            };
                            event = self.editing.clone().unwrap_or_default();
                            if !event.object_id.is_empty() && !event_view(&event).can_edit {
                                return Task::none();
                            }
                            // Existing online meetings retain their scheduling metadata and online status.
                            if event.object_id.is_empty() {
                                event.is_online_meeting = d.online;
                                event.is_organizer = true;
                                event.is_response_requested = true;
                                event.event_type = "Single".into();
                                event.my_response_type = "Organizer".into();
                                event.organizer_name = ctx.display_name.to_owned();
                                event.organizer_address = ctx.account_email.to_owned();
                            }
                            event.subject = d.subject;
                            event.start_time = d.start;
                            event.end_time = d.end;
                            event.event_time_zone = d.time_zone;
                            event.is_all_day_event = d.all_day;
                            event.location = d.location;
                            // Preserve original HTML when the description was not changed.
                            if self
                                .editing
                                .as_ref()
                                .is_none_or(|old| draft(old).description != d.description)
                            {
                                event.body_content = d.description;
                                event.body_content_type = "Text".into();
                            }
                            let retained: Vec<_> = event
                                .attendees
                                .iter()
                                .filter(|a| !matches!(a.kind.as_str(), "Required" | "Optional"))
                                .cloned()
                                .collect();
                            event.attendees = d
                                .attendees
                                .into_iter()
                                .map(|a| {
                                    let mut wire = event
                                        .attendees
                                        .iter()
                                        .find(|old| old.address.eq_ignore_ascii_case(&a.address))
                                        .cloned()
                                        .unwrap_or_default();
                                    wire.address = a.address;
                                    wire.name = a.name;
                                    wire.kind =
                                        if a.optional { "Optional" } else { "Required" }.into();
                                    wire
                                })
                                .chain(retained)
                                .collect();
                            // An occurrence keeps the series' pattern; a new event or a series master takes the form's.
                            if !event.is_occurrence() {
                                match recurrence(&d.recurrence, &event.start_time) {
                                    Ok(r) => event.set_recurrence(r.as_ref()),
                                    Err(e) => {
                                        self.view.error = Some(e);
                                        return Task::none();
                                    }
                                }
                            }
                            if let Err(e) = event.validate() {
                                self.view.error = Some(e.to_string());
                                return Task::none();
                            }
                        } else {
                            if event.object_id.is_empty() {
                                return Task::none();
                            }
                            match &mutation {
                                CalendarAction::Cancel { .. } if !event_view(&event).can_cancel => {
                                    return Task::none();
                                }
                                CalendarAction::Delete { .. } if !event_view(&event).can_delete => {
                                    return Task::none();
                                }
                                CalendarAction::Respond { .. }
                                    if !event_view(&event).can_respond =>
                                {
                                    return Task::none();
                                }
                                _ => {}
                            }
                        }
                        // The series is addressed by its master's id.
                        let series = match &mutation {
                            CalendarAction::Delete { series }
                            | CalendarAction::Cancel { series }
                            | CalendarAction::Respond { series, .. } => *series,
                            _ => false,
                        };
                        let target = series
                            .then(|| event.series_id().map(str::to_owned))
                            .flatten()
                            .unwrap_or_else(|| event.object_id.clone());
                        let exception = event.event_type == "Exception";
                        let response = if let CalendarAction::Respond { response, .. } = &mutation {
                            match response.as_str() {
                                "Accept" => Some(Response::Accept),
                                "TentativelyAccept" => Some(Response::Tentative),
                                "Decline" => Some(Response::Decline),
                                _ => return Task::none(),
                            }
                        } else {
                            None
                        };
                        let g = ctx.generation;
                        self.view.busy = true;
                        self.view.error = None;
                        return Task::perform(
                            async move {
                                let result = match mutation {
                                    CalendarAction::Save if event.object_id.is_empty() => {
                                        client.create(&event).await.map(drop)
                                    }
                                    CalendarAction::Save => client.update(&event).await,
                                    CalendarAction::Delete { .. } => client.delete(&target).await,
                                    CalendarAction::Cancel { .. } => {
                                        let mut event = event;
                                        event.object_id = target;
                                        client.cancel(&event).await
                                    }
                                    _ => {
                                        let response = response.unwrap();
                                        // An exception is answered silently first, then the series, unless it is declined.
                                        if series
                                            && exception
                                            && !matches!(response, Response::Decline)
                                            && target != event.object_id
                                        {
                                            client
                                                .respond_with(&event.object_id, response, false)
                                                .await
                                                .map_err(|e| (e.to_string(), e.uncertain()))?;
                                        }
                                        client.respond(&target, response).await
                                    }
                                };
                                result.map_err(|e| (e.to_string(), e.uncertain()))
                            },
                            move |v| Msg::Calendar(CalendarMsg::Mutated(g, v)),
                        );
                    }
                }
            }
        }
        Task::none()
    }
}

impl App {
    /// The calendar, and the context it reads from the rest of the app.
    fn calendar(&mut self) -> (&mut CalendarState, Ctx<'_>) {
        (&mut self.calendar, ctx!(self))
    }
    pub(crate) fn load_calendar(&mut self, force: bool) -> Task<Msg> {
        let (calendar, ctx) = self.calendar();
        calendar.load(&ctx, force)
    }
    pub(crate) fn update_calendar(&mut self, msg: CalendarMsg) -> Task<Msg> {
        let (calendar, ctx) = self.calendar();
        calendar.update(&ctx, msg)
    }
    pub(crate) fn find_meeting(
        &mut self,
        start: String,
        end: String,
        subject: String,
        map: impl FnOnce(Result<Option<Event>, String>) -> Msg + Send + 'static,
    ) -> Task<Msg> {
        let (calendar, ctx) = self.calendar();
        calendar.find_meeting(&ctx, start, end, subject, map)
    }
}

#[cfg(test)]
mod recurrence_tests {
    use super::*;
    #[test]
    fn presets_follow_the_start_and_read_back() {
        let start = "2026-10-26T09:00:00+01:00"; // the last Monday of October
        let monthly = recurrence(
            &RecurrenceDraft {
                repeat: Repeat::Monthly,
                ..Default::default()
            },
            start,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            monthly.pattern,
            Pattern::RelativeMonthly {
                interval: 1,
                weekday: 1,
                index: 4
            }
        );
        let custom = RecurrenceDraft {
            repeat: Repeat::Custom,
            unit: RepeatUnit::Week,
            interval: 2,
            weekdays: vec![3, 1, 3],
            end_date: "2026-12-21".into(),
            ..Default::default()
        };
        let r = recurrence(&custom, start).unwrap().unwrap();
        assert_eq!(
            r.pattern,
            Pattern::Weekly {
                interval: 2,
                days: vec![1, 3]
            }
        );
        let back = recurrence_draft(&r, NaiveDate::from_ymd_opt(2026, 10, 26));
        assert_eq!(back.repeat, Repeat::Custom);
        assert_eq!(back.end_date, "2026-12-21");
        let weekly = recurrence(
            &RecurrenceDraft {
                repeat: Repeat::Weekly,
                ..Default::default()
            },
            start,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            recurrence_draft(&weekly, NaiveDate::from_ymd_opt(2026, 10, 26)).repeat,
            Repeat::Weekly
        );
        assert!(recurrence(
            &RecurrenceDraft {
                repeat: Repeat::Daily,
                end_date: "2026-01-01".into(),
                ..Default::default()
            },
            start
        )
        .is_err());
        assert!(!recurrence_text(&r).is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::Job;
    fn app() -> App {
        let (mut app, _) = App::boot(true);
        *app.nav = Nav::Calendar;
        app.calendar.view.start = "2026-10-01T00:00:00Z".into();
        app.calendar.view.end = "2026-11-01T00:00:00Z".into();
        app
    }
    fn event(organizer: bool, recurring: bool) -> Event {
        Event {
            object_id: "demo".into(),
            subject: "Test".into(),
            start_time: "2026-10-15T09:00:00Z".into(),
            end_time: "2026-10-15T10:00:00Z".into(),
            is_organizer: organizer,
            is_response_requested: true,
            event_type: if recurring { "Occurrence" } else { "Single" }.into(),
            ..Default::default()
        }
    }
    async fn run(app: &mut App, task: Task<Msg>) {
        let mut jobs = task.0;
        while let Some(job) = jobs.pop() {
            if let Job::Future(future) = job {
                jobs.extend(app.update(future.await).0);
            }
        }
    }
    #[tokio::test]
    async fn period_and_detail_responses_are_ignored_when_obsolete() {
        let mut app = app();
        app.calendar.list_request = 2;
        let _ = app.update_calendar(CalendarMsg::Listed(
            app.generation,
            1,
            Ok(vec![event(true, false)]),
        ));
        assert!(app.calendar.view.events.is_empty());
        app.calendar.detail_request = 3;
        let _ = app.update_calendar(CalendarMsg::Detail(
            app.generation,
            2,
            Ok(Box::new(event(true, false))),
        ));
        assert!(app.calendar.view.selected.is_none());
        let _ = app.update_calendar(CalendarMsg::Listed(
            app.generation,
            2,
            Ok(vec![event(true, false)]),
        ));
        assert_eq!(app.calendar.view.events.len(), 1);
    }
    #[tokio::test]
    async fn sign_out_discards_drafts_and_late_mutations() {
        let mut app = app();
        let g = app.generation;
        app.calendar.view.draft = Some(CalendarDraft::default());
        let _ = app.update(Msg::SignOut);
        let _ = app.update_calendar(CalendarMsg::Mutated(g, Err(("late".into(), true))));
        let _ = app.update_calendar(CalendarMsg::Listed(g, 0, Ok(vec![event(true, false)])));
        assert!(app.calendar.view.draft.is_none());
        assert!(app.calendar.view.error.is_none());
        assert!(app.calendar.view.events.is_empty());
    }
    #[tokio::test]
    async fn failed_mutation_preserves_draft_and_uncertainty_blocks_resubmit() {
        let mut app = app();
        let d = draft(&event(true, false));
        app.calendar.view.draft = Some(d.clone());
        app.calendar.view.busy = true;
        let _ = app.update_calendar(CalendarMsg::Mutated(
            app.generation,
            Err(("liaison incertaine".into(), true)),
        ));
        assert_eq!(app.calendar.view.draft, Some(d));
        assert!(!app.calendar.view.busy);
        assert!(app.calendar.view.uncertain);
        assert!(
            app.update_calendar(CalendarMsg::Action(CalendarAction::Save))
                .0
                .is_empty()
        );
    }
    #[tokio::test]
    async fn rights_and_double_submissions_are_enforced_in_core() {
        let mut app = app();
        app.calendar.selected = Some(event(false, false));
        assert!(
            app.update_calendar(CalendarMsg::Action(CalendarAction::Cancel { series: false }))
                .0
                .is_empty()
        );
        let _ = app.update_calendar(CalendarMsg::Action(CalendarAction::Edit { series: false }));
        assert!(app.calendar.view.draft.is_none());
        // The organizer edits one occurrence of a series alone.
        app.calendar.selected = Some(event(true, true));
        let _ = app.update_calendar(CalendarMsg::Action(CalendarAction::Edit { series: false }));
        assert!(app.calendar.view.draft.is_some());
        let _ = app.update_calendar(CalendarMsg::Action(CalendarAction::CloseEditor));
        app.calendar.view.busy = true;
        assert!(
            app.update_calendar(CalendarMsg::Action(CalendarAction::Respond {
                response: "Accept".into(),
                series: false,
            }))
            .0
            .is_empty()
        );
    }
    #[tokio::test]
    async fn demo_exercises_create_read_update_cancel_delete_and_rsvp() {
        let mut app = app();
        let task = app.load_calendar(true);
        run(&mut app, task).await;
        assert!(!app.calendar.view.events.is_empty());
        let d = CalendarDraft {
            subject: "Test complet".into(),
            start: "2026-10-15T12:00:00Z".into(),
            end: "2026-10-15T13:00:00Z".into(),
            time_zone: "Europe/Paris".into(),
            online: true,
            ..Default::default()
        };
        app.calendar.view.draft = Some(d);
        let task = app.update_calendar(CalendarMsg::Action(CalendarAction::Save));
        assert!(app.calendar.view.busy);
        assert!(
            app.update_calendar(CalendarMsg::Action(CalendarAction::Save))
                .0
                .is_empty()
        );
        run(&mut app, task).await;
        assert!(
            app.calendar.view.error.is_none(),
            "{:?}",
            app.calendar.view.error
        );
        let id = app
            .calendar
            .events
            .iter()
            .find(|e| e.subject == "Test complet")
            .unwrap()
            .object_id
            .clone();
        let task = app.update_calendar(CalendarMsg::Action(CalendarAction::Select {
            id: id.clone(),
        }));
        run(&mut app, task).await;
        let _ = app.update_calendar(CalendarMsg::Action(CalendarAction::Edit { series: false }));
        app.calendar.view.draft.as_mut().unwrap().subject = "Modifié".into();
        let task = app.update_calendar(CalendarMsg::Action(CalendarAction::Save));
        run(&mut app, task).await;
        assert!(app.calendar.events.iter().any(|e| e.subject == "Modifié"));
        let task = app.update_calendar(CalendarMsg::Action(CalendarAction::Select {
            id: id.clone(),
        }));
        run(&mut app, task).await;
        let task = app.update_calendar(CalendarMsg::Action(CalendarAction::Cancel { series: false }));
        run(&mut app, task).await;
        assert!(
            app.calendar
                .events
                .iter()
                .find(|e| e.object_id == id)
                .unwrap()
                .is_cancelled
        );
        let task = app.update_calendar(CalendarMsg::Action(CalendarAction::Select {
            id: "demo-review".into(),
        }));
        run(&mut app, task).await;
        let task = app.update_calendar(CalendarMsg::Action(CalendarAction::Respond {
            response: "Accept".into(),
            series: false,
        }));
        run(&mut app, task).await;
        assert_eq!(
            app.calendar
                .events
                .iter()
                .find(|e| e.object_id == "demo-review")
                .unwrap()
                .my_response_type,
            "Accepted"
        );
        let task = app.update_calendar(CalendarMsg::Action(CalendarAction::Select {
            id: "demo-review".into(),
        }));
        run(&mut app, task).await;
        let task = app.update_calendar(CalendarMsg::Action(CalendarAction::Delete { series: false }));
        run(&mut app, task).await;
        assert!(
            !app.calendar
                .events
                .iter()
                .any(|e| e.object_id == "demo-review")
        );
    }
    #[test]
    fn html_description_and_unknown_metadata_survive_roundtrip() {
        let mut e = event(true, false);
        e.body_content_type = "HTML".into();
        e.body_content = "<p>Bonjour <b>Alice</b></p>".into();
        assert_eq!(draft(&e).description, "Bonjour Alice");
        assert!(event_view(&e).can_delete);
        assert!(!event_view(&e).can_cancel);
    }
}
