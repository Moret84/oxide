//! The search results page: server-side message search with the web client's filters, and opening a result.
use super::view::PersonView;
use super::*;
use chrono::{Datelike, NaiveDate, TimeZone};
use teams_chat::{MessageHit, SearchFilters, SearchIn};
use teams_i18n::{t, tf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum SearchDate {
    #[default]
    Any,
    Today,
    Yesterday,
    ThisWeek,
    LastWeek,
    ThisMonth,
    LastMonth,
    ThisYear,
    LastYear,
    /// Between `SearchFiltersView.start` and `end` (`YYYY-MM-DD`, both included).
    Custom,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum SearchKind {
    #[default]
    Everywhere,
    Chats,
    Channels,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct SearchFiltersView {
    pub from: Option<PersonView>,
    pub date: SearchDate,
    pub start: String,
    pub end: String,
    pub kind: SearchKind,
    pub has_attachment: bool,
    pub mentions_me: bool,
    pub hide_bots: bool,
    /// Only the open chat or channel.
    pub here: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct SearchHitView {
    pub message_id: String,
    pub sender: String,
    pub sender_id: String,
    pub preview: String,
    pub time: String,
    /// "Team › Channel", or the chat's name.
    pub place: String,
    pub is_channel: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, uniffi::Record)]
pub struct SearchView {
    pub open: bool,
    pub query: String,
    pub filters: SearchFiltersView,
    /// Where "only here" searches: the open chat or channel, if any.
    pub here_name: Option<String>,
    pub hits: Vec<SearchHitView>,
    pub terms: Vec<String>,
    pub total: u64,
    pub loading: bool,
    pub has_more: bool,
    pub searched: bool,
    pub error: Option<String>,
    /// People found for the "From" filter.
    pub people: Vec<PersonView>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum SearchAction {
    /// Opens the results page and searches.
    Run {
        query: String,
        filters: SearchFiltersView,
    },
    More,
    Close,
    OpenHit {
        message_id: String,
    },
    FindPeople {
        query: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum SearchMsg {
    Action(SearchAction),
    Found(u64, u64, bool, Result<teams_chat::SearchPage, String>),
    People(u64, String, Result<Vec<Person>, String>),
}

#[derive(Default)]
pub(crate) struct SearchState {
    pub view: SearchView,
    hits: Vec<MessageHit>,
    /// The thread "only here" was asked in, kept while the results are browsed.
    here: Option<Selection>,
    page: u32,
    request: u64,
    cvid: String,
    /// The last "From" lookup: earlier answers are dropped.
    people_query: String,
}

/// Local midnight of `date`, in UTC ISO 8601 with milliseconds.
fn midnight(date: NaiveDate) -> Option<String> {
    let local = Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .earliest()?;
    Some(
        local
            .with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    )
}
/// `[start, end)` of a date choice, as the web client's ranges (weeks start on Sunday).
pub(crate) fn date_range(
    choice: SearchDate,
    start: &str,
    end: &str,
    today: NaiveDate,
) -> Option<(String, Option<String>)> {
    let days = |n: i64| today + chrono::Duration::days(n);
    let sunday = today - chrono::Duration::days(today.weekday().num_days_from_sunday().into());
    let first = today.with_day(1)?;
    let next_month = first.checked_add_months(chrono::Months::new(1))?;
    let previous_month = first.checked_sub_months(chrono::Months::new(1))?;
    let year = NaiveDate::from_ymd_opt(today.year(), 1, 1)?;
    let (from, to) = match choice {
        SearchDate::Any => return None,
        SearchDate::Today => (today, Some(days(1))),
        SearchDate::Yesterday => (days(-1), Some(today)),
        SearchDate::ThisWeek => (sunday, Some(sunday + chrono::Duration::days(7))),
        SearchDate::LastWeek => (sunday - chrono::Duration::days(7), Some(sunday)),
        SearchDate::ThisMonth => (first, Some(next_month)),
        SearchDate::LastMonth => (previous_month, Some(first)),
        SearchDate::ThisYear => (year, None),
        SearchDate::LastYear => (NaiveDate::from_ymd_opt(today.year() - 1, 1, 1)?, Some(year)),
        SearchDate::Custom => {
            let parse = |s: &str| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok();
            let (from, to) = (parse(start)?, parse(end)?);
            (from.min(to), Some(from.max(to) + chrono::Duration::days(1)))
        }
    };
    Some((midnight(from)?, to.and_then(midnight)))
}

impl App {
    pub(crate) fn update_search(&mut self, msg: SearchMsg) -> Task<Msg> {
        let task = self.search_msg(msg);
        self.refresh_search_view();
        task
    }
    fn search_msg(&mut self, msg: SearchMsg) -> Task<Msg> {
        let generation = self.generation;
        match msg {
            SearchMsg::Found(g, ..) | SearchMsg::People(g, ..) if g != generation => Task::none(),
            SearchMsg::Action(SearchAction::Run { query, filters }) => {
                let search = &mut *self.search_state;
                if search.cvid.is_empty() {
                    search.cvid = uuid::Uuid::new_v4().to_string();
                }
                if !search.view.open || filters.here && search.here.is_none() {
                    search.here = self.selected.clone();
                }
                search.view.open = true;
                search.view.query = query;
                search.view.filters = filters;
                search.page = 0;
                search.hits.clear();
                self.search_page(false)
            }
            SearchMsg::Action(SearchAction::More) => {
                if self.search_state.view.loading || !self.search_state.view.has_more {
                    return Task::none();
                }
                self.search_state.page += 1;
                self.search_page(true)
            }
            SearchMsg::Action(SearchAction::Close) => {
                let search = &mut *self.search_state;
                search.view.open = false;
                search.request += 1;
                search.here = None;
                Task::none()
            }
            SearchMsg::Action(SearchAction::FindPeople { query }) => {
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                let query = query.trim().to_owned();
                self.search_state.people_query = query.clone();
                if query.chars().count() < 2 {
                    self.search_state.view.people.clear();
                    return Task::none();
                }
                Task::perform(
                    async move {
                        let r = client
                            .search_people(&query)
                            .await
                            .map_err(|e| e.to_string());
                        (query, r)
                    },
                    move |(q, r)| Msg::FullSearch(SearchMsg::People(generation, q, r)),
                )
            }
            SearchMsg::People(_, query, result) => {
                if query != self.search_state.people_query {
                    return Task::none();
                }
                if let Ok(people) = result {
                    self.search_state.view.people = people
                        .iter()
                        .filter(|p| !p.mri.is_empty())
                        .map(|p| PersonView {
                            mri: p.mri.clone(),
                            name: p.name().to_owned(),
                            email: p.email.clone(),
                        })
                        .collect();
                }
                Task::none()
            }
            SearchMsg::Action(SearchAction::OpenHit { message_id }) => {
                let Some(hit) = self
                    .search_state
                    .hits
                    .iter()
                    .find(|h| h.message_id == message_id)
                    .cloned()
                else {
                    return Task::none();
                };
                self.search_state.view.open = false;
                if hit.is_channel {
                    let team = self
                        .teams
                        .iter()
                        .find(|t| {
                            t.id == hit.thread_id
                                || t.channels.iter().any(|c| c.id == hit.thread_id)
                        })
                        .map(|t| t.id.clone());
                    let Some(team) = team else {
                        *self.notice = Some(t("core.search.channel_not_in_your_teams").into());
                        return Task::none();
                    };
                    let select = Task::done(Msg::Select(Selection::Channel {
                        team,
                        channel: hit.thread_id.clone(),
                    }));
                    // A reply opens its thread; a root post shows in the channel.
                    match hit.reply_chain_id.filter(|root| *root != hit.message_id) {
                        Some(root) => Task::batch([
                            select,
                            Task::done(ConversationMsg::OpenReply(root).into()),
                        ]),
                        None => select,
                    }
                } else {
                    Task::done(Msg::Select(Selection::Chat(hit.thread_id)))
                }
            }
            SearchMsg::Found(_, request, append, result) => {
                let search = &mut *self.search_state;
                if request != search.request {
                    return Task::none();
                }
                search.view.loading = false;
                search.view.searched = true;
                match result {
                    Ok(page) => {
                        if !append {
                            search.hits.clear();
                        }
                        for hit in page.hits {
                            if !search.hits.iter().any(|h| h.message_id == hit.message_id) {
                                search.hits.push(hit);
                            }
                        }
                        search.view.total = page.total;
                        search.view.has_more = page.more;
                        search.view.terms = page.terms;
                        search.view.error = None;
                    }
                    Err(e) => search.view.error = Some(tf!("core.search.failed", error = e)),
                }
                Task::none()
            }
        }
    }
    fn search_page(&mut self, append: bool) -> Task<Msg> {
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        let me = self.me.clone();
        let search = &mut *self.search_state;
        let f = &search.view.filters;
        let filters = SearchFilters {
            from: f.from.as_ref().map(|p| p.mri.clone()),
            thread: if f.here {
                search
                    .here
                    .as_ref()
                    .map(|s| (s.id().to_owned(), !s.is_channel()))
            } else {
                None
            },
            sent: date_range(f.date, &f.start, &f.end, Local::now().date_naive()),
            has_attachment: f.has_attachment,
            mentions: f
                .mentions_me
                .then(|| me.rsplit(':').next().unwrap_or_default().to_owned())
                .filter(|oid| !oid.is_empty()),
            kind: match f.kind {
                SearchKind::Everywhere => SearchIn::Everywhere,
                SearchKind::Chats => SearchIn::Chats,
                SearchKind::Channels => SearchIn::Channels,
            },
            hide_bots: f.hide_bots,
        };
        let empty = search.view.query.trim().is_empty() && filters == SearchFilters::default();
        if empty {
            search.view.loading = false;
            search.view.searched = false;
            search.hits.clear();
            return Task::none();
        }
        search.request += 1;
        search.view.loading = true;
        let request = search.request;
        let (query, page, cvid) = (search.view.query.clone(), search.page, search.cvid.clone());
        let generation = self.generation;
        let language = if teams_i18n::english() {
            "en-US"
        } else {
            "fr-FR"
        };
        let zone = iana_time_zone();
        Task::perform(
            async move {
                client
                    .search_messages(&query, &filters, page, &cvid, &zone, language)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |r| Msg::FullSearch(SearchMsg::Found(generation, request, append, r)),
        )
    }
    pub(crate) fn refresh_search_view(&mut self) {
        let here_name = self
            .search_state
            .here
            .clone()
            .or_else(|| self.selected.clone())
            .map(|s| self.place_name(&s));
        let hits: Vec<SearchHitView> = self
            .search_state
            .hits
            .iter()
            .map(|hit| SearchHitView {
                message_id: hit.message_id.clone(),
                sender: if hit.sender_name.is_empty() {
                    self.known_name(&hit.sender_mri).unwrap_or_default()
                } else {
                    hit.sender_name.clone()
                },
                sender_id: hit.sender_mri.clone(),
                preview: hit.preview.clone(),
                time: hit_time(&hit.sent),
                place: if hit.is_channel && !hit.team_name.is_empty() {
                    format!("{} › {}", hit.team_name, hit.channel_name)
                } else if hit.is_channel {
                    hit.channel_name.clone()
                } else {
                    self.chats
                        .iter()
                        .find(|c| c.id == hit.thread_id)
                        .map(|c| chat_title(c, &self.me))
                        .unwrap_or_else(|| {
                            if hit.topic.is_empty() {
                                t("core.app.chat").into()
                            } else {
                                hit.topic.clone()
                            }
                        })
                },
                is_channel: hit.is_channel,
            })
            .collect();
        let search = &mut *self.search_state;
        let mut view = search.view.clone();
        view.hits = hits;
        view.here_name = here_name;
        if search.view != view {
            search.view = view;
        }
    }
    fn place_name(&self, selection: &Selection) -> String {
        match selection {
            Selection::Chat(id) => self
                .chats
                .iter()
                .find(|c| &c.id == id)
                .map(|c| chat_title(c, &self.me))
                .unwrap_or_else(|| t("core.app.chat").into()),
            Selection::Channel { team, channel } => self
                .teams
                .iter()
                .find(|t| &t.id == team)
                .and_then(|t| t.channels.iter().find(|c| &c.id == channel))
                .map(|c| c.display_name.clone())
                .unwrap_or_else(|| t("core.app.channel").into()),
        }
    }
}
/// "14:05" today, else the date.
fn hit_time(iso: &str) -> String {
    let Ok(time) = DateTime::parse_from_rfc3339(iso) else {
        return String::new();
    };
    let local = time.with_timezone(&Local);
    if local.date_naive() == Local::now().date_naive() {
        local.format("%H:%M").to_string()
    } else {
        local.format(t("core.me.date_format")).to_string()
    }
}
/// The system's IANA time zone, as the web client sends `timezone`.
fn iana_time_zone() -> String {
    std::env::var("TZ")
        .ok()
        .filter(|z| z.contains('/'))
        .or_else(|| {
            std::fs::read_link("/etc/localtime").ok().and_then(|p| {
                let p = p.to_string_lossy().into_owned();
                p.split_once("zoneinfo/").map(|(_, zone)| zone.to_owned())
            })
        })
        .unwrap_or_else(|| "UTC".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn date_choices_follow_the_web_client_ranges() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap(); // a Wednesday
        let range = |c| date_range(c, "", "", today).unwrap();
        let local = |iso: &str| {
            DateTime::parse_from_rfc3339(iso)
                .unwrap()
                .with_timezone(&Local)
                .date_naive()
                .to_string()
        };
        let (from, to) = range(SearchDate::ThisWeek);
        assert_eq!(
            (local(&from), local(&to.unwrap())),
            ("2026-10-18".into(), "2026-10-25".into())
        );
        let (from, to) = range(SearchDate::LastMonth);
        assert_eq!(
            (local(&from), local(&to.unwrap())),
            ("2026-09-01".into(), "2026-10-01".into())
        );
        assert!(range(SearchDate::ThisYear).1.is_none());
        let (from, to) = date_range(SearchDate::Custom, "2026-10-10", "2026-10-05", today).unwrap();
        assert_eq!(
            (local(&from), local(&to.unwrap())),
            ("2026-10-05".into(), "2026-10-11".into())
        );
        assert!(date_range(SearchDate::Any, "", "", today).is_none());
    }
}
