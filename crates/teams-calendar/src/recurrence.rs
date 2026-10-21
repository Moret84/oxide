//! Series recurrence as the MT event carries it: `eventRecurrencePattern` and `eventRecurrenceRange`.
use chrono::{Datelike, FixedOffset, NaiveDate, TimeZone};
use serde_json::{Value, json};

use crate::Event;

/// Weekdays are 0 (Sunday) to 6, week indexes 0 (first) to 4 (last), months 1 to 12.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Daily {
        interval: u32,
    },
    Weekly {
        interval: u32,
        days: Vec<u8>,
    },
    Monthly {
        interval: u32,
        day: u32,
    },
    RelativeMonthly {
        interval: u32,
        weekday: u8,
        index: u8,
    },
    Yearly {
        month: u32,
        day: u32,
    },
    RelativeYearly {
        month: u32,
        weekday: u8,
        index: u8,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recurrence {
    pub pattern: Pattern,
    pub start: NaiveDate,
    /// Date of the last occurrence; `None` never ends.
    pub end: Option<NaiveDate>,
}

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const INDEXES: [&str; 5] = ["First", "Second", "Third", "Fourth", "Last"];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
/// An enum given as its integer or its name (the scheduling service writes names).
fn number(value: Option<&Value>, names: &[&str], base: u64) -> Option<u64> {
    let value = value?;
    value.as_u64().or_else(|| {
        let name = value.as_str()?;
        name.parse().ok().or_else(|| {
            names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(name))
                .map(|i| i as u64 + base)
        })
    })
}
const PATTERN_TYPES: [&str; 9] = [
    "None",
    "Daily",
    "Weekly",
    "Monthly",
    "RelativeMonthly",
    "Yearly",
    "RelativeYearly",
    "AbsoluteMonthly",
    "AbsoluteYearly",
];
fn interval(v: &Value) -> u32 {
    v.get("interval")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as u32
}
fn weekday(v: &Value, key: &str) -> Option<u8> {
    number(v.get(key), &WEEKDAYS, 0)
        .filter(|d| *d < 7)
        .map(|d| d as u8)
}
fn index(v: &Value) -> u8 {
    number(v.get("dayOfTheWeekIndex"), &INDEXES, 0)
        .unwrap_or(0)
        .min(4) as u8
}
fn month(v: &Value) -> u32 {
    number(v.get("month"), &MONTHS, 1).unwrap_or(1).clamp(1, 12) as u32
}
fn date(value: Option<&Value>) -> Option<NaiveDate> {
    let s = value?.as_str()?;
    NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()
}
impl Recurrence {
    /// The series' recurrence, when the event carries it (a series master usually does).
    pub fn of(event: &Event) -> Option<Self> {
        let p = event
            .extra
            .get("eventRecurrencePattern")
            .filter(|v| v.is_object())?;
        let kind = number(p.get("patternType"), &PATTERN_TYPES, 0)?;
        let pattern = match kind {
            1 => Pattern::Daily {
                interval: interval(p.get("daily")?),
            },
            2 => {
                let w = p.get("weekly")?;
                let mut days: Vec<u8> = w
                    .get("daysOfTheWeek")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|d| number(Some(d), &WEEKDAYS, 0))
                    .filter(|d| *d < 7)
                    .map(|d| d as u8)
                    .collect();
                days.sort_unstable();
                days.dedup();
                Pattern::Weekly {
                    interval: interval(w),
                    days,
                }
            }
            3 | 7 => {
                let m = p.get("monthly").or_else(|| p.get("absoluteMonthly"))?;
                Pattern::Monthly {
                    interval: interval(m),
                    day: m.get("dayOfMonth").and_then(Value::as_u64).unwrap_or(1) as u32,
                }
            }
            4 => {
                let m = p.get("relativeMonthly")?;
                Pattern::RelativeMonthly {
                    interval: interval(m),
                    weekday: weekday(m, "dayOfTheWeek")?,
                    index: index(m),
                }
            }
            5 | 8 => {
                let y = p.get("yearly").or_else(|| p.get("absoluteYearly"))?;
                Pattern::Yearly {
                    month: month(y),
                    day: y
                        .get("dayOfTheMonth")
                        .or_else(|| y.get("dayOfMonth"))
                        .and_then(Value::as_u64)
                        .unwrap_or(1) as u32,
                }
            }
            6 => {
                let y = p.get("relativeYearly")?;
                Pattern::RelativeYearly {
                    month: month(y),
                    weekday: weekday(y, "dayOfTheWeek")?,
                    index: index(y),
                }
            }
            _ => return None,
        };
        let range = event.extra.get("eventRecurrenceRange");
        let start = date(range.and_then(|r| r.get("startDate")))
            .or_else(|| date(Some(&Value::String(event.start_time.clone()))))?;
        Some(Self {
            pattern,
            start,
            end: date(range.and_then(|r| r.get("endDate"))),
        })
    }
    /// `eventRecurrencePattern` as the web client's editor builds it.
    pub fn pattern_json(&self) -> Value {
        match &self.pattern {
            Pattern::Daily { interval } => {
                json!({"patternType": 1, "daily": {"interval": interval}})
            }
            Pattern::Weekly { interval, days } => {
                json!({"patternType": 2, "weekly": {"interval": interval, "daysOfTheWeek": days}})
            }
            Pattern::Monthly { interval, day } => {
                json!({"patternType": 3, "monthly": {"dayOfMonth": day, "interval": interval}})
            }
            Pattern::RelativeMonthly {
                interval,
                weekday,
                index,
            } => json!({"patternType": 4, "relativeMonthly": {
                "interval": interval, "dayOfTheWeek": weekday, "dayOfTheWeekIndex": index}}),
            Pattern::Yearly { month, day } => {
                json!({"patternType": 5, "yearly": {"dayOfTheMonth": day, "month": month}})
            }
            Pattern::RelativeYearly {
                month,
                weekday,
                index,
            } => json!({"patternType": 6, "relativeYearly": {
                "month": month, "dayOfTheWeek": weekday, "dayOfTheWeekIndex": index}}),
        }
    }
    /// `eventRecurrenceRange`: local midnights with `offset`, no `endDate` when endless.
    pub fn range_json(&self, offset: FixedOffset) -> Value {
        let midnight = |d: NaiveDate| {
            offset
                .from_local_datetime(&d.and_hms_opt(0, 0, 0).expect("midnight"))
                .single()
                .map(|t| t.format("%Y-%m-%dT%H:%M:%S%:z").to_string())
        };
        let mut range = json!({"startDate": midnight(self.start)});
        if let Some(end) = self.end.and_then(midnight) {
            range["endDate"] = json!(end);
        }
        range
    }
}
/// Weekday index (0 first … 4 last) as the "Monthly" preset picks it: last within the month's last seven days.
pub fn week_index(date: NaiveDate) -> u8 {
    let next_month = if date.month() == 12 {
        NaiveDate::from_ymd_opt(date.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(date.year(), date.month() + 1, 1)
    };
    let days_in_month = next_month
        .and_then(|n| n.pred_opt())
        .map_or(31, |d| d.day());
    if date.day() + 7 > days_in_month {
        4
    } else {
        ((date.day() - 1) / 7) as u8
    }
}
/// 0 (Sunday) to 6.
pub fn weekday_number(date: NaiveDate) -> u8 {
    date.weekday().num_days_from_sunday() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(pattern: Value, range: Value) -> Event {
        let mut e = Event {
            start_time: "2026-10-26T09:00:00+01:00".into(),
            ..Default::default()
        };
        e.extra.insert("eventRecurrencePattern".into(), pattern);
        e.extra.insert("eventRecurrenceRange".into(), range);
        e
    }
    #[test]
    fn patterns_read_integers_or_names_and_write_integers() {
        let e = event(
            json!({"patternType": "Weekly", "weekly": {"interval": 1, "daysOfTheWeek": ["Wednesday", "Monday"]}}),
            json!({"startDate": "2026-10-26T00:00:00+01:00", "endDate": "2026-12-21T00:00:00+01:00"}),
        );
        let r = Recurrence::of(&e).unwrap();
        assert_eq!(
            r.pattern,
            Pattern::Weekly {
                interval: 1,
                days: vec![1, 3]
            }
        );
        assert_eq!(r.end, NaiveDate::from_ymd_opt(2026, 12, 21));
        assert_eq!(
            r.pattern_json(),
            json!({"patternType": 2, "weekly": {"interval": 1, "daysOfTheWeek": [1, 3]}})
        );
        let offset = FixedOffset::east_opt(3600).unwrap();
        assert_eq!(
            r.range_json(offset),
            json!({"startDate": "2026-10-26T00:00:00+01:00", "endDate": "2026-12-21T00:00:00+01:00"})
        );
        let monthly = event(
            json!({"patternType": 4, "relativeMonthly": {"interval": 1, "dayOfTheWeek": 1, "dayOfTheWeekIndex": 4}}),
            json!({"startDate": "2026-10-26T00:00:00+01:00"}),
        );
        let r = Recurrence::of(&monthly).unwrap();
        assert_eq!(r.end, None);
        assert!(r.range_json(offset).get("endDate").is_none());
    }
    #[test]
    fn the_monthly_preset_uses_the_last_week_at_the_month_end() {
        assert_eq!(
            week_index(NaiveDate::from_ymd_opt(2026, 10, 26).unwrap()),
            4
        );
        assert_eq!(week_index(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()), 0);
        assert_eq!(
            week_index(NaiveDate::from_ymd_opt(2026, 10, 15).unwrap()),
            2
        );
        assert_eq!(
            weekday_number(NaiveDate::from_ymd_opt(2026, 10, 25).unwrap()),
            0
        );
    }
}
