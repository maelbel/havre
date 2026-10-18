//! Quick add: "Call Thomas fri 10:00 #work" → a title, a due day and time,
//! and a list.
//!
//! Words are read one by one. A word (or two) that says *when* — `today`,
//! `tomorrow`, `fri`, `16/10`, `16 oct`, `10:00`, `3pm`, `10h30`, also in
//! French (`demain`, `vendredi`, `16 octobre`) — becomes the due date or
//! time and leaves the title, with a little word before it like `on` or
//! `at`. `#name` picks a list. Everything else is the title.
//!
//! A small hand-written parser rather than a library: easy to read, and it
//! teaches matching on `&str` and `Option` chains.

use chrono::{Datelike, Duration, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday};
use serde::Serialize;

/// What quick add understood.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Parsed {
    pub title: String,
    /// The day: midnight UTC of that date.
    pub due: Option<i64>,
    /// With a time: that moment.
    pub due_at: Option<i64>,
    /// A list picked with `#name`.
    pub list: Option<i64>,
}

const WEEKDAYS: &[(&str, Weekday)] = &[
    ("monday", Weekday::Mon),
    ("mon", Weekday::Mon),
    ("lundi", Weekday::Mon),
    ("tuesday", Weekday::Tue),
    ("tues", Weekday::Tue),
    ("tue", Weekday::Tue),
    ("mardi", Weekday::Tue),
    ("wednesday", Weekday::Wed),
    ("wed", Weekday::Wed),
    ("mercredi", Weekday::Wed),
    ("thursday", Weekday::Thu),
    ("thurs", Weekday::Thu),
    ("thur", Weekday::Thu),
    ("thu", Weekday::Thu),
    ("jeudi", Weekday::Thu),
    ("friday", Weekday::Fri),
    ("fri", Weekday::Fri),
    ("vendredi", Weekday::Fri),
    ("saturday", Weekday::Sat),
    ("sat", Weekday::Sat),
    ("samedi", Weekday::Sat),
    ("sunday", Weekday::Sun),
    ("dimanche", Weekday::Sun),
];

const MONTHS: &[(&str, u32)] = &[
    ("january", 1),
    ("jan", 1),
    ("janvier", 1),
    ("janv", 1),
    ("february", 2),
    ("feb", 2),
    ("février", 2),
    ("fevrier", 2),
    ("fév", 2),
    ("fev", 2),
    ("march", 3),
    ("mars", 3),
    ("april", 4),
    ("apr", 4),
    ("avril", 4),
    ("avr", 4),
    ("may", 5),
    ("mai", 5),
    ("june", 6),
    ("jun", 6),
    ("juin", 6),
    ("july", 7),
    ("jul", 7),
    ("juillet", 7),
    ("juil", 7),
    ("august", 8),
    ("aug", 8),
    ("août", 8),
    ("aout", 8),
    ("september", 9),
    ("sept", 9),
    ("sep", 9),
    ("septembre", 9),
    ("october", 10),
    ("oct", 10),
    ("octobre", 10),
    ("november", 11),
    ("nov", 11),
    ("novembre", 11),
    ("december", 12),
    ("dec", 12),
    ("décembre", 12),
    ("decembre", 12),
    ("déc", 12),
];

/// Little words that go with a date or time: "on Friday", "at 10", "le 16".
const CONNECTORS: &[&str] = &["on", "at", "by", "due", "le", "à", "a", "pour", "avant", "before"];

/// `now`: unix seconds. `js_offset`: the browser's `getTimezoneOffset()`
/// (Paris in summer: -120). `lists`: `(id, name)` for `#name`.
pub fn parse(text: &str, now: i64, js_offset: i32, lists: &[(i64, String)]) -> Parsed {
    let offset = FixedOffset::east_opt(-js_offset.clamp(-14 * 60, 14 * 60) * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"));
    let local_now =
        Utc.timestamp_opt(now, 0).single().unwrap_or_default().with_timezone(&offset).naive_local();
    let today = local_now.date();

    let words: Vec<&str> = text.split_whitespace().collect();
    // `true` for each word that was understood (and leaves the title).
    let mut used = vec![false; words.len()];
    let mut date: Option<NaiveDate> = None;
    let mut time: Option<NaiveTime> = None;
    let mut list: Option<i64> = None;

    let mut i = 0;
    while i < words.len() {
        let word = clean(words[i]);
        let next = words.get(i + 1).map(|w| clean(w));
        let mut took = 0; // how many words this one used

        if let Some(name) = words[i].strip_prefix('#').filter(|_| list.is_none()) {
            if let Some(id) = find_list(name, lists) {
                list = Some(id);
                took = 1;
            }
        } else if date.is_none()
            && let Some((found, n)) =
                read_date(&word, next.as_deref(), words.get(i + 2).map(|w| clean(w)), today)
        {
            date = Some(found);
            took = n;
        }
        if took == 0
            && time.is_none()
            && let Some((found, n)) =
                read_time(&word, next.as_deref(), words.get(i.wrapping_sub(1)).map(|w| clean(w)))
        {
            time = Some(found);
            took = n;
        }

        if took > 0 {
            for u in used.iter_mut().skip(i).take(took) {
                *u = true;
            }
            // "on Friday", "at 10": the little word before goes too.
            if i > 0 && !used[i - 1] && CONNECTORS.contains(&clean(words[i - 1]).as_str()) {
                used[i - 1] = true;
            }
            i += took;
        } else {
            i += 1;
        }
    }

    // A time alone: today if it's still to come, else tomorrow.
    if let (None, Some(t)) = (date, time) {
        date = Some(if t > local_now.time() { today } else { today + Duration::days(1) });
    }

    let title: Vec<&str> = words.iter().zip(&used).filter(|(_, u)| !**u).map(|(w, _)| *w).collect();
    let title = if title.is_empty() { text.trim().to_owned() } else { title.join(" ") };
    let due = date.map(|d| d.and_time(NaiveTime::MIN).and_utc().timestamp());
    let due_at = match (date, time) {
        (Some(d), Some(t)) => {
            offset.from_local_datetime(&NaiveDateTime::new(d, t)).single().map(|x| x.timestamp())
        }
        _ => None,
    };
    Parsed { title, due, due_at, list }
}

/// Lowercase, without punctuation around (keeps `:`, `/`, `-` inside).
fn clean(word: &str) -> String {
    word.trim_matches(|c: char| matches!(c, ',' | '.' | ';' | '!' | '?' | '(' | ')' | '"')).to_lowercase()
}

fn find_list(name: &str, lists: &[(i64, String)]) -> Option<i64> {
    let squash = |s: &str| s.to_lowercase().replace([' ', '-', '_'], "");
    let wanted = squash(name);
    if wanted.is_empty() {
        return None;
    }
    lists
        .iter()
        .find(|(_, n)| squash(n) == wanted)
        .or_else(|| lists.iter().find(|(_, n)| squash(n).starts_with(&wanted)))
        .map(|(id, _)| *id)
}

fn weekday(word: &str) -> Option<Weekday> {
    WEEKDAYS.iter().find(|(name, _)| *name == word).map(|(_, d)| *d)
}

fn month(word: &str) -> Option<u32> {
    MONTHS.iter().find(|(name, _)| *name == word).map(|(_, m)| *m)
}

/// The next `day` after today (never today: "fri" on a Friday is next week).
fn next_weekday(today: NaiveDate, day: Weekday) -> NaiveDate {
    let ahead =
        (day.num_days_from_monday() as i64 - today.weekday().num_days_from_monday() as i64).rem_euclid(7);
    today + Duration::days(if ahead == 0 { 7 } else { ahead })
}

/// Day and month without a year: this year, or next year if it's past.
fn upcoming(today: NaiveDate, day: u32, month: u32) -> Option<NaiveDate> {
    let this_year = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    if this_year >= today { Some(this_year) } else { NaiveDate::from_ymd_opt(today.year() + 1, month, day) }
}

/// A date in `word` (and maybe the next two words). Returns it and how many
/// words it used.
fn read_date(
    word: &str,
    next: Option<&str>,
    after: Option<String>,
    today: NaiveDate,
) -> Option<(NaiveDate, usize)> {
    match word {
        "today" | "tod" | "aujourd'hui" | "aujourd’hui" | "auj" => return Some((today, 1)),
        "tomorrow" | "tmr" | "tmrw" | "demain" => return Some((today + Duration::days(1), 1)),
        "après-demain" | "apres-demain" => return Some((today + Duration::days(2), 1)),
        _ => {}
    }
    // "next week", "semaine prochaine": next Monday.
    if (word == "next" && next == Some("week")) || (word == "semaine" && next == Some("prochaine")) {
        return Some((next_weekday(today, Weekday::Mon), 2));
    }
    // "next friday", "friday", "vendredi (prochain)".
    if word == "next"
        && let Some(day) = next.and_then(weekday)
    {
        return Some((next_weekday(today, day), 2));
    }
    if let Some(day) = weekday(word) {
        let n = if matches!(next, Some("prochain" | "prochaine")) { 2 } else { 1 };
        return Some((next_weekday(today, day), n));
    }
    // "in 3 days", "dans 2 semaines".
    if matches!(word, "in" | "dans")
        && let Some(n) = next.and_then(|n| n.parse::<i64>().ok()).filter(|n| (1..=365).contains(n))
    {
        let unit = after.as_deref().unwrap_or_default();
        let days = match unit {
            "day" | "days" | "jour" | "jours" => n,
            "week" | "weeks" | "semaine" | "semaines" => n * 7,
            _ => return None,
        };
        return Some((today + Duration::days(days), 3));
    }
    // "2026-10-16".
    if let Ok(d) = NaiveDate::parse_from_str(word, "%Y-%m-%d") {
        return Some((d, 1));
    }
    // "16/10" or "16/10/2026" (day first, as in France and the UK).
    let parts: Vec<&str> = word.split('/').collect();
    if (2..=3).contains(&parts.len())
        && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    {
        let day: u32 = parts[0].parse().ok()?;
        let month: u32 = parts[1].parse().ok()?;
        let found = match parts.get(2) {
            Some(year) => {
                let year: i32 = year.parse().ok()?;
                let year = if year < 100 { 2000 + year } else { year };
                NaiveDate::from_ymd_opt(year, month, day)
            }
            None => upcoming(today, day, month),
        };
        return found.map(|d| (d, 1));
    }
    // "16 oct" / "16 octobre".
    if let Ok(day) = word.parse::<u32>()
        && let Some(m) = next.and_then(month)
    {
        return upcoming(today, day, m).map(|d| (d, 2));
    }
    // "oct 16".
    if let Some(m) = month(word)
        && let Some(day) =
            next.and_then(|n| n.trim_end_matches(|c: char| c.is_alphabetic()).parse::<u32>().ok())
    {
        return upcoming(today, day, m).map(|d| (d, 2));
    }
    None
}

/// A time: `10:00`, `10h`, `10h30`, `3pm`, `10:30am`, or `at 10` / `à 10`.
fn read_time(word: &str, next: Option<&str>, before: Option<String>) -> Option<(NaiveTime, usize)> {
    let at = |h: u32, m: u32| NaiveTime::from_hms_opt(h, m, 0);
    // am / pm, glued ("3pm") or not ("3 pm").
    let (core, meridiem, used) = if let Some(c) = word.strip_suffix("am") {
        (c, Some(false), 1)
    } else if let Some(c) = word.strip_suffix("pm") {
        (c, Some(true), 1)
    } else if matches!(next, Some("am" | "pm")) && word.chars().all(|c| c.is_ascii_digit() || c == ':') {
        (word, Some(next == Some("pm")), 2)
    } else {
        (word, None, 1)
    };
    let (h, m) = if let Some((h, m)) = core.split_once(':').or_else(|| core.split_once('h')) {
        let m = if m.is_empty() { 0 } else { m.parse().ok()? };
        (h.parse::<u32>().ok()?, m)
    } else if let Some(h) = core.strip_suffix('h') {
        (h.parse::<u32>().ok()?, 0)
    } else if meridiem.is_some() || matches!(before.as_deref(), Some("at" | "à" | "a")) {
        // A bare number is a time only after "at" or with am/pm.
        (core.parse::<u32>().ok()?, 0)
    } else {
        return None;
    };
    if m > 59 {
        return None;
    }
    let h = match meridiem {
        Some(_) if !(1..=12).contains(&h) => return None,
        Some(true) if h != 12 => h + 12,
        Some(false) if h == 12 => 0,
        _ => h,
    };
    at(h, m).map(|t| (t, used))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thursday 15 October 2026, 09:00 in Paris (summer time: UTC+2).
    fn now() -> i64 {
        Utc.with_ymd_and_hms(2026, 10, 15, 7, 0, 0).unwrap().timestamp()
    }
    const PARIS: i32 = -120;

    fn day(y: i32, m: u32, d: u32) -> Option<i64> {
        Some(NaiveDate::from_ymd_opt(y, m, d).unwrap().and_time(NaiveTime::MIN).and_utc().timestamp())
    }
    fn paris(y: i32, m: u32, d: u32, h: u32, min: u32) -> Option<i64> {
        Some(Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap().timestamp() - 2 * 3600)
    }
    fn p(text: &str) -> Parsed {
        let lists = [(1, "LM Control".to_owned()), (2, "Lyon project".to_owned()), (3, "Home".to_owned())];
        parse(text, now(), PARIS, &lists)
    }

    #[test]
    fn days_and_times() {
        let r = p("Call Thomas about the November visit fri 10:00");
        assert_eq!(r.title, "Call Thomas about the November visit");
        assert_eq!(r.due, day(2026, 10, 16));
        assert_eq!(r.due_at, paris(2026, 10, 16, 10, 0));

        assert_eq!(p("Book the train tomorrow").due, day(2026, 10, 16));
        assert_eq!(p("Book the train today").title, "Book the train");
        // Never today: "thu" on a Thursday is next week.
        assert_eq!(p("Weekly report thu").due, day(2026, 10, 22));
        assert_eq!(p("Plan next week").due, day(2026, 10, 19));
        assert_eq!(p("Pay in 3 days").due, day(2026, 10, 18));
        let r = p("Sign the quote on 21/10 at 3pm");
        assert_eq!(
            (r.title.as_str(), r.due, r.due_at),
            ("Sign the quote", day(2026, 10, 21), paris(2026, 10, 21, 15, 0))
        );
        assert_eq!(p("Renew oct 2").due, day(2027, 10, 2), "past dates mean next year");
        assert_eq!(p("Renew 2027-01-05").due, day(2027, 1, 5));
    }

    #[test]
    fn french_too() {
        let r = p("Appeler Thomas vendredi à 10h30");
        assert_eq!((r.title.as_str(), r.due_at), ("Appeler Thomas", paris(2026, 10, 16, 10, 30)));
        assert_eq!(p("Facture demain").due, day(2026, 10, 16));
        assert_eq!(p("Chaudière le 24 octobre").title, "Chaudière");
        assert_eq!(p("Chaudière le 24 octobre").due, day(2026, 10, 24));
        assert_eq!(p("Bilan semaine prochaine").due, day(2026, 10, 19));
    }

    #[test]
    fn a_time_alone_and_lists() {
        // 08:00 has passed (it's 09:00): tomorrow. 17:00 is still to come.
        assert_eq!(p("Standup 8:00").due, day(2026, 10, 16));
        assert_eq!(p("Confirm 17:00").due_at, paris(2026, 10, 15, 17, 0));
        let r = p("Slides #lyon fri");
        assert_eq!((r.title.as_str(), r.list), ("Slides", Some(2)));
        assert_eq!(p("Boiler #home").list, Some(3));
        assert_eq!(p("Read #nothing").title, "Read #nothing", "unknown lists stay in the title");
        // Numbers that aren't times stay.
        assert_eq!(p("Order 3 chairs").title, "Order 3 chairs");
        assert_eq!(p("Order 3 chairs").due, None);
        // Only the first date counts.
        let r = p("Move the fri meeting to mon");
        assert_eq!((r.title.as_str(), r.due), ("Move the meeting to mon", day(2026, 10, 16)));
        // Nothing left: the text is the title.
        assert_eq!(p("tomorrow").title, "tomorrow");
    }
}
