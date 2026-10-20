//! Repeating events: how often an event repeats, in Havre's own words.
//!
//! [`Repeat`] is what the event editor sends ("every 2 weeks on Monday and
//! Thursday, until 18 Dec"). Havre turns it into what each provider
//! understands — the iCalendar `RRULE` line (CalDAV, Google, Havre's own
//! calendar) or Microsoft's `recurrence` object — and reads those back into
//! a `Repeat` to show and edit it. A rule made elsewhere that a `Repeat`
//! can't describe ("the 2nd and 4th weekday") reads back as `None`: Havre
//! shows it but doesn't rewrite it.
//!
//! The browser never sends rule text: every rule is built here from checked
//! fields, so nothing unexpected reaches a provider.
//!
//! **Time zones.** A weekly 10:00 must stay at 10:00 after the clocks
//! change, so a timed rule is kept in a time zone (`Europe/Paris`), not in
//! UTC. All-day events have no time zone: their days are UTC dates, as
//! everywhere in Havre.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Error, Result};

/// At most this many occurrences when a series ends "after N times".
pub const MAX_COUNT: u32 = 730;
/// "Every 99 weeks" at most.
pub const MAX_INTERVAL: u32 = 99;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// A day of the week, as iCalendar writes it (`MO`, `TU`…), lower case in JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Day {
    Mo,
    Tu,
    We,
    Th,
    Fr,
    Sa,
    Su,
}

const DAYS: [Day; 7] = [Day::Mo, Day::Tu, Day::We, Day::Th, Day::Fr, Day::Sa, Day::Su];
const CODES: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];
const GRAPH_DAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
const GRAPH_INDEX: [(i8, &str); 5] = [(1, "first"), (2, "second"), (3, "third"), (4, "fourth"), (-1, "last")];

impl Day {
    /// Its place in the week, Monday = 0 (the `as` cast reads an enum's number).
    fn index(self) -> usize {
        self as usize
    }

    fn code(self) -> &'static str {
        CODES[self.index()]
    }

    fn from_code(code: &str) -> Option<Day> {
        CODES.iter().position(|c| *c == code).map(|i| DAYS[i])
    }

    /// The weekday of a date.
    pub fn of(date: NaiveDate) -> Day {
        DAYS[date.weekday().num_days_from_monday() as usize]
    }
}

/// "The third Monday" (`nth` 1–4) or "the last Friday" (`nth` −1) of a month.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NthDay {
    pub nth: i8,
    pub day: Day,
}

impl NthDay {
    /// Which one `date` is: the 19 Oct 2026 is the third Monday.
    pub fn of(date: NaiveDate, last: bool) -> NthDay {
        let nth = if last { -1 } else { ((date.day() - 1) / 7 + 1) as i8 };
        NthDay { nth, day: Day::of(date) }
    }

    fn matches(self, date: NaiveDate) -> bool {
        if self.day != Day::of(date) {
            return false;
        }
        if self.nth == -1 {
            // The last one: a week later is next month.
            return (date + Duration::days(7)).month() != date.month();
        }
        NthDay::of(date, false).nth == self.nth
    }
}

/// How an event repeats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Repeat {
    pub freq: Freq,
    /// Every `interval` days / weeks / months / years.
    #[serde(default = "one")]
    pub interval: u32,
    /// Weekly: on these days (empty: the day of the first event).
    #[serde(default)]
    pub days: Vec<Day>,
    /// Monthly: "on the third Monday" (`None`: on the same day number).
    #[serde(default)]
    pub on: Option<NthDay>,
    /// The last day it can happen, as a day (midnight UTC of that date).
    #[serde(default)]
    pub until: Option<i64>,
    /// Or: how many times it happens in all.
    #[serde(default)]
    pub count: Option<u32>,
}

fn one() -> u32 {
    1
}

/// The time zone called `name` (IANA: `Europe/Paris`); UTC when none is given.
pub fn zone(name: Option<&str>) -> Result<Tz> {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        None => Ok(Tz::UTC),
        Some(name) => {
            name.parse::<Tz>().map_err(|_| Error::Config(format!("“{name}” isn't a time zone Havre knows.")))
        }
    }
}

/// The date an event happens on: its UTC date when all-day, else in `tz`.
pub fn local_date(ts: i64, all_day: bool, tz: Tz) -> NaiveDate {
    let utc = utc(ts);
    if all_day { utc.date_naive() } else { utc.with_timezone(&tz).date_naive() }
}

/// The wall-clock time `ts` shows in `tz`.
pub fn local_time(ts: i64, tz: Tz) -> NaiveDateTime {
    utc(ts).with_timezone(&tz).naive_local()
}

/// A wall-clock time in `tz` as unix seconds. In the hour skipped when the
/// clocks go forward, the time just after it.
pub fn from_local(local: NaiveDateTime, tz: Tz) -> i64 {
    match tz.from_local_datetime(&local).earliest() {
        Some(t) => t.timestamp(),
        None => from_local(local + Duration::hours(1), tz),
    }
}

/// Midnight UTC of a date (how Havre writes days).
pub fn day_ts(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0).map(|d| d.and_utc().timestamp()).unwrap_or_default()
}

fn utc(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0).single().unwrap_or_default()
}

/// Where an occurrence lands when the whole series moves: the first one
/// (`first_start`) shifts by as many days as the edited occurrence moved, and
/// takes its new time of day. The clock time is kept in `tz`, not seconds:
/// 10:00 → 11:00 is "an hour later" in winter and in summer alike.
pub fn shift(first_start: i64, original_start: i64, new_start: i64, all_day: bool, tz: Tz) -> i64 {
    let days = (local_date(new_start, all_day, tz) - local_date(original_start, all_day, tz)).num_days();
    let date = local_date(first_start, all_day, tz) + Duration::days(days);
    if all_day {
        return day_ts(date);
    }
    from_local(date.and_time(local_time(new_start, tz).time()), tz)
}

impl Repeat {
    /// Drops the fields that don't apply to this frequency, and sorts the days.
    pub fn normalized(mut self) -> Repeat {
        if self.freq != Freq::Weekly {
            self.days.clear();
        }
        if self.freq != Freq::Monthly {
            self.on = None;
        }
        self.days.sort();
        self.days.dedup();
        self
    }

    /// Checks the rule against the event's first occurrence.
    pub fn validate(&self, start: i64, all_day: bool, tz: Tz) -> Result<()> {
        let fail = |msg: &str| Err(Error::Config(msg.into()));
        if !(1..=MAX_INTERVAL).contains(&self.interval) {
            return fail("Repeat every 1 to 99 days, weeks, months or years.");
        }
        if self.until.is_some() && self.count.is_some() {
            return fail("Choose an end date or a number of times, not both.");
        }
        if let Some(count) = self.count
            && !(1..=MAX_COUNT).contains(&count)
        {
            return fail("A repeating event can happen 1 to 730 times.");
        }
        let first = local_date(start, all_day, tz);
        if let Some(until) = self.until {
            if until % 86_400 != 0 {
                return fail("The end date is a day.");
            }
            if utc(until).date_naive() < first {
                return fail("The repeat must end after the event starts.");
            }
        }
        if self.freq == Freq::Weekly && !self.days.is_empty() && !self.days.contains(&Day::of(first)) {
            return fail("The first event must be on one of the chosen days.");
        }
        if let Some(on) = self.on
            && (!matches!(on.nth, 1..=4 | -1) || !on.matches(first))
        {
            return fail("The first event must be on the chosen day of the month.");
        }
        Ok(())
    }

    /// The iCalendar rule, without the `RRULE:` name:
    /// `FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH;UNTIL=20261218T225959Z`.
    pub fn rrule(&self, all_day: bool, tz: Tz) -> String {
        let freq = match self.freq {
            Freq::Daily => "DAILY",
            Freq::Weekly => "WEEKLY",
            Freq::Monthly => "MONTHLY",
            Freq::Yearly => "YEARLY",
        };
        let mut parts = vec![format!("FREQ={freq}")];
        if self.interval > 1 {
            parts.push(format!("INTERVAL={}", self.interval));
        }
        if self.freq == Freq::Weekly && !self.days.is_empty() {
            let codes: Vec<&str> = self.days.iter().map(|d| d.code()).collect();
            parts.push(format!("BYDAY={}", codes.join(",")));
        }
        if let (Freq::Monthly, Some(on)) = (self.freq, self.on) {
            parts.push(format!("BYDAY={}{}", on.nth, on.day.code()));
        }
        if let Some(count) = self.count {
            parts.push(format!("COUNT={count}"));
        }
        if let Some(until) = self.until {
            parts.push(format!("UNTIL={}", until_value(utc(until).date_naive(), all_day, tz)));
        }
        parts.join(";")
    }

    /// Reads an `RRULE` back. `None`: a rule a `Repeat` can't describe
    /// (hourly, "the 2nd and 4th Monday", `BYSETPOS`…).
    pub fn parse(rule: &str, all_day: bool, tz: Tz) -> Option<Repeat> {
        let rule = rule.trim().trim_start_matches("RRULE:");
        let mut repeat =
            Repeat { freq: Freq::Daily, interval: 1, days: vec![], on: None, until: None, count: None };
        let mut by_day: Option<&str> = None;
        let mut has_freq = false;
        for part in rule.split(';').filter(|p| !p.is_empty()) {
            let (key, value) = part.split_once('=')?;
            match key.to_ascii_uppercase().as_str() {
                "FREQ" => {
                    has_freq = true;
                    repeat.freq = match value.to_ascii_uppercase().as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        _ => return None,
                    }
                }
                "INTERVAL" => {
                    repeat.interval = value.parse().ok().filter(|n| (1..=MAX_INTERVAL).contains(n))?
                }
                "COUNT" => repeat.count = Some(value.parse().ok()?),
                "UNTIL" => repeat.until = Some(day_ts(read_until(value, all_day, tz)?)),
                "BYDAY" => by_day = Some(value),
                // The week starts on…: doesn't change a rule a Repeat can describe.
                "WKST" => {}
                // "On the 19th" / "in October": the same as the first event, as
                // other apps write it. (A list would be a different rule.)
                "BYMONTHDAY" | "BYMONTH" if !value.contains(',') => {}
                _ => return None,
            }
        }
        if !has_freq {
            return None;
        }
        if let Some(by_day) = by_day {
            let by_day = by_day.to_ascii_uppercase();
            match repeat.freq {
                Freq::Weekly => {
                    for code in by_day.split(',') {
                        repeat.days.push(Day::from_code(code)?);
                    }
                }
                Freq::Monthly if !by_day.contains(',') => {
                    let (nth, code) = by_day.split_at(by_day.len().checked_sub(2)?);
                    let nth: i8 = nth.trim_start_matches('+').parse().ok()?;
                    if !matches!(nth, 1..=4 | -1) {
                        return None;
                    }
                    repeat.on = Some(NthDay { nth, day: Day::from_code(code)? });
                }
                _ => return None,
            }
        }
        Some(repeat.normalized())
    }

    /// Microsoft's `recurrence` object. `start`: the first occurrence;
    /// `zone_name`: the time zone Microsoft gets (it repeats in it).
    pub fn graph(&self, start: i64, all_day: bool, tz: Tz, zone_name: &str) -> Value {
        let first = local_date(start, all_day, tz);
        let index = |nth: i8| GRAPH_INDEX.iter().find(|(n, _)| *n == nth).map_or("first", |(_, name)| *name);
        let pattern = match self.freq {
            Freq::Daily => json!({ "type": "daily", "interval": self.interval }),
            Freq::Weekly => {
                let days = if self.days.is_empty() { vec![Day::of(first)] } else { self.days.clone() };
                json!({
                    "type": "weekly",
                    "interval": self.interval,
                    "daysOfWeek": days.iter().map(|d| GRAPH_DAYS[d.index()]).collect::<Vec<_>>(),
                    "firstDayOfWeek": "monday",
                })
            }
            Freq::Monthly => match self.on {
                Some(on) => json!({
                    "type": "relativeMonthly",
                    "interval": self.interval,
                    "daysOfWeek": [GRAPH_DAYS[on.day.index()]],
                    "index": index(on.nth),
                }),
                None => {
                    json!({ "type": "absoluteMonthly", "interval": self.interval, "dayOfMonth": first.day() })
                }
            },
            Freq::Yearly => json!({
                "type": "absoluteYearly",
                "interval": self.interval,
                "dayOfMonth": first.day(),
                "month": first.month(),
            }),
        };
        let mut range =
            json!({ "startDate": first.format("%Y-%m-%d").to_string(), "recurrenceTimeZone": zone_name });
        match (self.until, self.count) {
            (Some(until), _) => {
                range["type"] = json!("endDate");
                range["endDate"] = json!(utc(until).date_naive().format("%Y-%m-%d").to_string());
            }
            (None, Some(count)) => {
                range["type"] = json!("numbered");
                range["numberOfOccurrences"] = json!(count);
            }
            (None, None) => range["type"] = json!("noEnd"),
        }
        json!({ "pattern": pattern, "range": range })
    }

    /// Reads Microsoft's `recurrence` back.
    pub fn from_graph(recurrence: &Value) -> Option<Repeat> {
        let pattern = &recurrence["pattern"];
        let range = &recurrence["range"];
        let interval = pattern["interval"].as_u64().unwrap_or(1) as u32;
        if !(1..=MAX_INTERVAL).contains(&interval) {
            return None;
        }
        let days: Vec<Day> = pattern["daysOfWeek"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|d| GRAPH_DAYS.iter().position(|g| Some(*g) == d.as_str()).map(|i| DAYS[i]))
                    .collect()
            })
            .unwrap_or_default();
        let mut repeat =
            Repeat { freq: Freq::Daily, interval, days: vec![], on: None, until: None, count: None };
        match pattern["type"].as_str()? {
            "daily" => {}
            "weekly" => {
                repeat.freq = Freq::Weekly;
                repeat.days = days;
            }
            "absoluteMonthly" => repeat.freq = Freq::Monthly,
            "relativeMonthly" if days.len() == 1 => {
                let nth = GRAPH_INDEX.iter().find(|(_, name)| Some(*name) == pattern["index"].as_str())?.0;
                repeat.freq = Freq::Monthly;
                repeat.on = Some(NthDay { nth, day: days[0] });
            }
            "absoluteYearly" => repeat.freq = Freq::Yearly,
            _ => return None,
        }
        match range["type"].as_str() {
            Some("endDate") => {
                let end = NaiveDate::parse_from_str(range["endDate"].as_str()?, "%Y-%m-%d").ok()?;
                repeat.until = Some(day_ts(end));
            }
            Some("numbered") => repeat.count = Some(range["numberOfOccurrences"].as_u64()? as u32),
            _ => {}
        }
        Some(repeat.normalized())
    }

    /// How many occurrences come before `before`, for a series whose first
    /// occurrence is `first_start` (to continue "after 10 times" in a new
    /// series without adding to it).
    pub fn occurrences_before(&self, first_start: i64, before: i64, all_day: bool, tz: Tz) -> u32 {
        // The rrule crate (through icalendar) does the counting.
        let rule = self.rrule(false, if all_day { Tz::UTC } else { tz });
        let start = if all_day {
            format!("DTSTART:{}", utc(first_start).format("%Y%m%dT%H%M%SZ"))
        } else {
            format!("DTSTART;TZID={}:{}", tz.name(), local_time(first_start, tz).format("%Y%m%dT%H%M%S"))
        };
        let Ok(set) = format!("{start}\nRRULE:{rule}").parse::<icalendar::rrule::RRuleSet>() else {
            return 0;
        };
        let before = utc(before - 1).with_timezone(&icalendar::Tz::UTC);
        set.before(before).all(MAX_COUNT as u16 + 1).dates.len() as u32
    }

    /// The same rule, ending the day before `original_start` happens (the
    /// first part of a series split at "this and the following ones").
    pub fn ended_before(&self, original_start: i64, all_day: bool, tz: Tz) -> Repeat {
        let last = local_date(original_start, all_day, tz) - Duration::days(1);
        let mut cut = self.clone();
        cut.count = None;
        cut.until = Some(day_ts(match self.until.map(|u| utc(u).date_naive()) {
            Some(until) if until < last => until,
            _ => last,
        }));
        cut
    }
}

/// `UNTIL`: the last day, written as iCalendar wants it — a date for all-day
/// events, else the end of that day in `tz`, in UTC.
fn until_value(last: NaiveDate, all_day: bool, tz: Tz) -> String {
    if all_day {
        return last.format("%Y%m%d").to_string();
    }
    let end = from_local(last.and_hms_opt(23, 59, 59).unwrap_or_default(), tz);
    utc(end).format("%Y%m%dT%H%M%SZ").to_string()
}

/// The day an `UNTIL` value falls on (in `tz` when it's a moment).
fn read_until(value: &str, all_day: bool, tz: Tz) -> Option<NaiveDate> {
    let value = value.trim();
    if value.len() == 8 {
        return NaiveDate::parse_from_str(value, "%Y%m%d").ok();
    }
    let moment = NaiveDateTime::parse_from_str(value.trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok()?;
    let ts = if value.ends_with('Z') { moment.and_utc().timestamp() } else { from_local(moment, tz) };
    Some(local_date(ts, all_day, tz))
}

/// Changes an `RRULE` (any rule, even one a `Repeat` can't describe) so it
/// stops before `original_start`, `COUNT` dropped. `UNTIL` is the end of the
/// day before in `tz` (a date for all-day series), so the rule reads back as
/// "until the day before"; a rule repeating within a day ends one second
/// before instead.
pub fn cut_rule(rule: &str, original_start: i64, all_day: bool, tz: Tz) -> String {
    let rule = rule.trim().trim_start_matches("RRULE:");
    let within_a_day = rule
        .to_ascii_uppercase()
        .split(';')
        .any(|p| matches!(p, "FREQ=HOURLY" | "FREQ=MINUTELY" | "FREQ=SECONDLY"));
    let mut parts: Vec<String> = rule
        .split(';')
        .filter(|p| !p.is_empty())
        .filter(|p| {
            let key = p.split('=').next().unwrap_or_default().to_ascii_uppercase();
            key != "COUNT" && key != "UNTIL"
        })
        .map(str::to_owned)
        .collect();
    let until = if within_a_day && !all_day {
        utc(original_start - 1).format("%Y%m%dT%H%M%SZ").to_string()
    } else {
        until_value(local_date(original_start, all_day, tz) - Duration::days(1), all_day, tz)
    };
    parts.push(format!("UNTIL={until}"));
    parts.join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARIS: Tz = chrono_tz::Europe::Paris;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Monday 19 Oct 2026, 10:00 in Paris (summer time: 08:00 UTC).
    fn monday_10h() -> i64 {
        from_local(date(2026, 10, 19).and_hms_opt(10, 0, 0).unwrap(), PARIS)
    }

    fn weekly(days: Vec<Day>) -> Repeat {
        Repeat { freq: Freq::Weekly, interval: 2, days, on: None, until: None, count: None }
    }

    #[test]
    fn writes_and_reads_rules() {
        let mut r = weekly(vec![Day::Mo, Day::Th]);
        r.until = Some(day_ts(date(2026, 12, 18)));
        // The end of 18 Dec in Paris (winter time) is 22:59:59 UTC.
        assert_eq!(r.rrule(false, PARIS), "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH;UNTIL=20261218T225959Z");
        assert_eq!(Repeat::parse(&r.rrule(false, PARIS), false, PARIS), Some(r.clone()));
        assert_eq!(r.rrule(true, PARIS), "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH;UNTIL=20261218");
        assert_eq!(Repeat::parse(&r.rrule(true, PARIS), true, PARIS), Some(r));

        let third_monday = Repeat {
            freq: Freq::Monthly,
            interval: 1,
            days: vec![],
            on: Some(NthDay { nth: 3, day: Day::Mo }),
            until: None,
            count: Some(10),
        };
        assert_eq!(third_monday.rrule(false, PARIS), "FREQ=MONTHLY;BYDAY=3MO;COUNT=10");
        assert_eq!(Repeat::parse("RRULE:FREQ=MONTHLY;BYDAY=3MO;COUNT=10", false, PARIS), Some(third_monday));
        let last_friday = Repeat::parse("FREQ=MONTHLY;BYDAY=-1FR", false, PARIS).unwrap();
        assert_eq!(last_friday.on, Some(NthDay { nth: -1, day: Day::Fr }));
        // How Google writes "every month on the 19th".
        assert_eq!(Repeat::parse("FREQ=MONTHLY;BYMONTHDAY=19", false, PARIS).unwrap().on, None);
    }

    #[test]
    fn rules_it_cant_describe() {
        for rule in [
            "FREQ=HOURLY",
            "FREQ=MONTHLY;BYDAY=2MO,4MO",
            "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
            "FREQ=DAILY;BYDAY=MO,TU",
            "FREQ=YEARLY;BYMONTH=1,7",
            "INTERVAL=2",
        ] {
            assert_eq!(Repeat::parse(rule, false, PARIS), None, "{rule}");
        }
    }

    #[test]
    fn checks_rules() {
        let start = monday_10h();
        assert!(weekly(vec![Day::Mo, Day::Th]).validate(start, false, PARIS).is_ok());
        assert!(weekly(vec![]).validate(start, false, PARIS).is_ok());
        // The first event (a Monday) isn't on the chosen days.
        assert!(weekly(vec![Day::Tu]).validate(start, false, PARIS).is_err());
        let mut r = weekly(vec![Day::Mo]);
        r.interval = 0;
        assert!(r.validate(start, false, PARIS).is_err());
        r.interval = 1;
        r.count = Some(731);
        assert!(r.validate(start, false, PARIS).is_err());
        r.count = Some(3);
        r.until = Some(day_ts(date(2026, 12, 1)));
        assert!(r.validate(start, false, PARIS).is_err(), "both an end date and a count");
        r.count = None;
        r.until = Some(day_ts(date(2026, 10, 18)));
        assert!(r.validate(start, false, PARIS).is_err(), "ends before it starts");
        let monthly = |nth| Repeat {
            freq: Freq::Monthly,
            interval: 1,
            days: vec![],
            on: Some(NthDay { nth, day: Day::Mo }),
            until: None,
            count: None,
        };
        assert!(monthly(3).validate(start, false, PARIS).is_ok(), "19 Oct is the third Monday");
        assert!(monthly(2).validate(start, false, PARIS).is_err());
        assert!(monthly(-1).validate(start, false, PARIS).is_err(), "and not the last one");
        assert!(zone(Some("Europe/Paris")).is_ok());
        assert!(zone(Some("Mars/Olympus")).is_err());
        assert_eq!(zone(None).unwrap(), Tz::UTC);
    }

    #[test]
    fn keeps_the_clock_time_across_daylight_saving() {
        // A weekly 10:00 from Monday 19 Oct: the clocks go back on 25 Oct,
        // so the 26 Oct one is at 09:00 UTC, an hour later than 19 Oct's.
        let start = monday_10h();
        let r = weekly(vec![Day::Mo]);
        let text = format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART;TZID=Europe/Paris:20261019T100000\r\n\
             DURATION:PT30M\r\nRRULE:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            Repeat { interval: 1, ..r }.rrule(false, PARIS)
        );
        let set = text.parse::<icalendar::Calendar>().unwrap();
        let event = set.calendar_events().next().unwrap();
        let dates = event.get_recurrence().unwrap().all(2).dates;
        assert_eq!(dates[0].timestamp(), start);
        assert_eq!(dates[1].timestamp(), start + 7 * 86_400 + 3_600);
        assert_eq!(dates[1].with_timezone(&PARIS).format("%H:%M").to_string(), "10:00");

        // Moving one occurrence from 10:00 to 11:00 a week later moves the
        // whole series to 11:00, including its first, summer-time event.
        let moved = shift(start, start + 7 * 86_400 + 3_600, start + 7 * 86_400 + 7_200, false, PARIS);
        assert_eq!(local_time(moved, PARIS), date(2026, 10, 19).and_hms_opt(11, 0, 0).unwrap());
    }

    #[test]
    fn microsoft_patterns() {
        let start = monday_10h();
        let mut r = weekly(vec![Day::Mo, Day::Th]);
        r.count = Some(6);
        let graph = r.graph(start, false, PARIS, "Europe/Paris");
        assert_eq!(graph["pattern"]["daysOfWeek"], json!(["monday", "thursday"]));
        assert_eq!(graph["range"]["type"], "numbered");
        assert_eq!(graph["range"]["startDate"], "2026-10-19");
        assert_eq!(Repeat::from_graph(&graph), Some(r));

        let third = Repeat {
            freq: Freq::Monthly,
            interval: 1,
            days: vec![],
            on: Some(NthDay { nth: 3, day: Day::Mo }),
            until: Some(day_ts(date(2027, 6, 30))),
            count: None,
        };
        let graph = third.graph(start, false, PARIS, "Europe/Paris");
        assert_eq!(graph["pattern"]["type"], "relativeMonthly");
        assert_eq!(graph["pattern"]["index"], "third");
        assert_eq!(graph["range"]["endDate"], "2027-06-30");
        assert_eq!(Repeat::from_graph(&graph), Some(third));
        assert_eq!(
            Repeat::from_graph(&json!({ "pattern": { "type": "relativeYearly" }, "range": {} })),
            None
        );
    }

    #[test]
    fn splitting_a_series() {
        let start = monday_10h();
        let mut r = weekly(vec![Day::Mo]);
        r.interval = 1;
        r.count = Some(10);
        // The fourth occurrence: three come before it.
        let fourth = start + 3 * 7 * 86_400 + 3_600;
        assert_eq!(r.occurrences_before(start, fourth, false, PARIS), 3);
        let cut = r.ended_before(fourth, false, PARIS);
        assert_eq!((cut.count, cut.until), (None, Some(day_ts(date(2026, 11, 8)))));

        assert_eq!(
            cut_rule("FREQ=WEEKLY;COUNT=10;BYDAY=MO", fourth, false, PARIS),
            "FREQ=WEEKLY;BYDAY=MO;UNTIL=20261108T225959Z"
        );
        // Read back: until the day before (8 Nov), as `ended_before` says.
        let back =
            Repeat::parse(&cut_rule("FREQ=WEEKLY;COUNT=10;BYDAY=MO", fourth, false, PARIS), false, PARIS);
        assert_eq!(back.and_then(|r| r.until), cut.until);
        assert_eq!(
            cut_rule("FREQ=HOURLY", 1_791_158_400, false, PARIS),
            "FREQ=HOURLY;UNTIL=20261004T235959Z"
        );
        assert_eq!(
            cut_rule("RRULE:FREQ=DAILY;UNTIL=20270101", 1_791_158_400, true, PARIS),
            "FREQ=DAILY;UNTIL=20261004"
        );
    }
}
