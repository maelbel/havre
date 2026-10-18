//! iCalendar (`.ics`): reading events (CalDAV, invitations in mail) and
//! writing them (CalDAV).
//!
//! Repeating events are expanded here into one [`RemoteEvent`] per
//! occurrence in the period asked for, using the `RRULE` (with `EXDATE`s
//! and modified occurrences, `RECURRENCE-ID`).

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use icalendar::{
    Calendar, CalendarDateTime, Component, DatePerhapsTime, Event as IcsEvent, EventLike, EventStatus,
    Property,
};

use super::{Attendee, EventInput, RemoteEvent};
use crate::{Error, Result};

/// Outlook and Exchange write Windows time zone names; the iCalendar
/// libraries know the IANA ones.
const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("Romance Standard Time", "Europe/Paris"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("FLE Standard Time", "Europe/Kiev"),
    ("Eastern Standard Time", "America/New_York"),
    ("Central Standard Time", "America/Chicago"),
    ("Mountain Standard Time", "America/Denver"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("China Standard Time", "Asia/Shanghai"),
    ("India Standard Time", "Asia/Kolkata"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("UTC", "UTC"),
];

fn normalize_zones(text: &str) -> String {
    let mut text = text.replace("\r\n ", "").replace("\n ", ""); // unfold long lines
    for (windows, iana) in WINDOWS_ZONES {
        text = text.replace(&format!("TZID={windows}:"), &format!("TZID={iana}:"));
        text = text.replace(&format!("TZID=\"{windows}\":"), &format!("TZID={iana}:"));
    }
    text
}

pub(crate) fn parse(text: &str) -> Result<Calendar> {
    normalize_zones(text).parse::<Calendar>().map_err(|e| Error::Config(format!("invalid iCalendar: {e}")))
}

/// A date or date-time as unix seconds (+ "is it a date only").
/// Floating times (no time zone) are read as UTC.
pub(crate) fn timestamp(value: &DatePerhapsTime) -> Option<(i64, bool)> {
    match value {
        DatePerhapsTime::Date(date) => Some((date_ts(*date), true)),
        DatePerhapsTime::DateTime(CalendarDateTime::Floating(naive)) => {
            Some((naive.and_utc().timestamp(), false))
        }
        DatePerhapsTime::DateTime(dt) => dt.try_into_utc().map(|d| (d.timestamp(), false)),
    }
}

fn date_ts(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0).map(|d| d.and_utc().timestamp()).unwrap_or_default()
}

/// `DURATION`: `PT1H30M`, `P1D`, `P2W`… in seconds.
fn duration(value: &str) -> Option<i64> {
    let value = value.trim().trim_start_matches('+');
    let negative = value.starts_with('-');
    let mut rest = value.trim_start_matches('-').strip_prefix('P')?;
    let mut total = 0i64;
    let mut in_time = false;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('T') {
            in_time = true;
            rest = r;
            continue;
        }
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let n: i64 = rest[..digits].parse().ok()?;
        let unit = rest[digits..].chars().next()?;
        total += n * match (unit, in_time) {
            ('W', _) => 7 * 86_400,
            ('D', _) => 86_400,
            ('H', true) => 3_600,
            ('M', true) => 60,
            ('S', true) => 1,
            _ => return None,
        };
        rest = &rest[digits + 1..];
    }
    Some(if negative { -total } else { total })
}

fn attendees(event: &IcsEvent) -> Vec<Attendee> {
    event
        .multi_properties()
        .get("ATTENDEE")
        .map(|list| {
            list.iter()
                .filter_map(|p| {
                    let email = p.value().trim().trim_start_matches("mailto:").trim_start_matches("MAILTO:");
                    if !email.contains('@') {
                        return None;
                    }
                    let status = p
                        .get_param_as("PARTSTAT", |v| Some(v.to_lowercase().replace('-', "_")))
                        .unwrap_or_else(|| "needs_action".into());
                    Some(Attendee {
                        name: p.get_param_as("CN", |v| Some(v.trim_matches('"').to_owned())),
                        email: email.to_owned(),
                        status,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The address in `Claire <claire@x.org>` (or a bare address).
pub(crate) fn email_in(who: &str) -> Option<String> {
    let address = match (who.rfind('<'), who.rfind('>')) {
        (Some(a), Some(b)) if a < b => &who[a + 1..b],
        _ => who,
    };
    let address = address.trim();
    address.contains('@').then(|| address.to_lowercase())
}

fn organizer(event: &IcsEvent) -> Option<String> {
    let p = event.properties().get("ORGANIZER")?;
    let email = p.value().trim_start_matches("mailto:").trim_start_matches("MAILTO:").to_owned();
    Some(match p.get_param_as("CN", |v| Some(v.trim_matches('"').to_owned())) {
        Some(name) if !name.is_empty() => format!("{name} <{email}>"),
        _ => email,
    })
}

/// One event's fields, without its time.
fn base(event: &IcsEvent, href: Option<&str>, etag: Option<&str>, raw: Option<&str>) -> RemoteEvent {
    RemoteEvent {
        href: href.map(str::to_owned),
        etag: etag.map(str::to_owned),
        raw: raw.map(str::to_owned),
        title: event.get_summary().unwrap_or("(no title)").to_owned(),
        location: event.get_location().unwrap_or_default().to_owned(),
        description: event.get_description().unwrap_or_default().replace("\\n", "\n"),
        organizer: organizer(event),
        attendees: attendees(event),
        join_url: event.get_url().map(str::to_owned),
        uid: event.get_uid().map(str::to_owned),
        ..RemoteEvent::default()
    }
}

/// The start and length of an event.
fn span(event: &IcsEvent) -> Option<(i64, i64, bool)> {
    let (start, all_day) = timestamp(&event.get_start()?)?;
    let end = match event.get_end().and_then(|e| timestamp(&e)) {
        Some((end, _)) => end,
        None => match event.property_value("DURATION").and_then(duration) {
            Some(d) => start + d,
            None if all_day => start + 86_400,
            None => start,
        },
    };
    Some((start, (end - start).max(0), all_day))
}

/// Every event (and occurrence) of an iCalendar text overlapping
/// `[from, to)`. `recurring_read_only`: occurrences of repeating events
/// can't be edited (CalDAV, for now).
pub fn events_between(
    text: &str,
    href: Option<&str>,
    etag: Option<&str>,
    from: i64,
    to: i64,
    recurring_read_only: bool,
) -> Result<Vec<RemoteEvent>> {
    let calendar = parse(text)?;
    let mut out = Vec::new();
    let all: Vec<_> = calendar.calendar_events().collect();
    // Modified occurrences ("this one is at 3 pm instead"), by their UID +
    // original start.
    let overridden: Vec<(String, i64)> = all
        .iter()
        .filter_map(|e| {
            let id = e.get_recurrence_id()?;
            Some((e.get_uid()?.to_owned(), timestamp(&id)?.0))
        })
        .collect();

    for cal_event in &all {
        let event: &IcsEvent = cal_event;
        if event.get_status() == Some(EventStatus::Cancelled) {
            continue;
        }
        let Some((start, length, all_day)) = span(event) else { continue };
        let uid = event.get_uid().map(str::to_owned).or_else(|| href.map(str::to_owned)).unwrap_or_default();
        let mut base = base(event, href, etag, Some(text));
        base.all_day = all_day;

        if let Some(rid) = event.get_recurrence_id() {
            // A modified occurrence.
            let original = timestamp(&rid).map_or(start, |t| t.0);
            if start < to && start + length > from {
                out.push(RemoteEvent {
                    remote_id: format!("{uid}|{original}"),
                    start,
                    end: start + length,
                    recurring: true,
                    read_only: recurring_read_only,
                    ..base
                });
            }
            continue;
        }

        if event.property_value("RRULE").is_none() && event.property_value("RDATE").is_none() {
            if start < to && start + length > from {
                out.push(RemoteEvent { remote_id: uid, start, end: start + length, ..base });
            }
            continue;
        }

        // Repeating: expand the occurrences in the period.
        let Ok(set) = cal_event.get_recurrence() else {
            tracing::warn!(uid, "can't expand a repeating event");
            continue;
        };
        let after = utc(from - length - 1);
        let before = utc(to);
        let occurrences = set
            .after(after.with_timezone(&icalendar::Tz::UTC))
            .before(before.with_timezone(&icalendar::Tz::UTC))
            .all(1000);
        for occurrence in occurrences.dates {
            let start = if all_day { date_ts(occurrence.date_naive()) } else { occurrence.timestamp() };
            if overridden.iter().any(|(u, t)| *u == uid && *t == start) {
                continue;
            }
            if start < to && start + length > from {
                out.push(RemoteEvent {
                    remote_id: format!("{uid}|{start}"),
                    start,
                    end: start + length,
                    recurring: true,
                    read_only: recurring_read_only,
                    ..base.clone()
                });
            }
        }
    }
    Ok(out)
}

fn utc(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0).single().unwrap_or_default()
}

/// The events of an invitation (`.ics` attached to an email).
pub fn invitation(text: &str) -> Result<Vec<RemoteEvent>> {
    let mut events = events_between(text, None, None, i64::MIN / 4, i64::MAX / 4, false)?;
    events.truncate(50);
    Ok(events)
}

/// An invitation received by email (iTIP, RFC 5546).
#[derive(Debug, Clone)]
pub struct Invitation {
    /// `REQUEST` (an invitation or an update), `CANCEL`, `REPLY`…
    pub method: String,
    pub uid: Option<String>,
    pub organizer: Option<String>,
    /// The organizer's address, to answer.
    pub organizer_email: Option<String>,
    /// The occurrences in it (repeating events: the first ones).
    pub events: Vec<RemoteEvent>,
}

pub fn read_invitation(text: &str) -> Result<Invitation> {
    let calendar = parse(text)?;
    let method = calendar.property_value("METHOD").unwrap_or("PUBLISH").to_uppercase();
    let master =
        calendar.events().next().ok_or_else(|| Error::Config("This invitation has no event.".into()))?;
    let organizer_email = master
        .properties()
        .get("ORGANIZER")
        .map(|p| p.value().trim().trim_start_matches("mailto:").trim_start_matches("MAILTO:").to_owned())
        .filter(|e| e.contains('@'));
    let mut events = events_between(text, None, None, i64::MIN / 4, i64::MAX / 4, false)?;
    events.sort_by_key(|e| e.start);
    events.truncate(20);
    Ok(Invitation {
        method,
        uid: master.get_uid().map(str::to_owned),
        organizer: organizer(master),
        organizer_email,
        events,
    })
}

/// The answer to an invitation (`METHOD:REPLY`): your status for its UID,
/// sent to the organizer by email. `partstat`: ACCEPTED, TENTATIVE, DECLINED.
pub fn reply_ics(text: &str, me_email: &str, me_name: &str, partstat: &str) -> Result<String> {
    let calendar = parse(text)?;
    let master =
        calendar.events().next().ok_or_else(|| Error::Config("This invitation has no event.".into()))?;
    let mut reply = IcsEvent::new();
    for key in ["UID", "SEQUENCE", "ORGANIZER", "SUMMARY", "RECURRENCE-ID"] {
        if let Some(p) = master.properties().get(key) {
            reply.append_property(p.clone());
        }
    }
    reply.timestamp(Utc::now());
    let mut attendee = Property::new("ATTENDEE", format!("mailto:{me_email}"));
    attendee.add_parameter("PARTSTAT", partstat);
    if !me_name.trim().is_empty() {
        attendee.add_parameter("CN", me_name.trim());
    }
    reply.append_property(attendee.done());
    let mut out = Calendar::new();
    out.append_property(Property::new("METHOD", "REPLY"));
    out.push(reply.done());
    Ok(out.done().to_string())
}

/// Sets an event's fields from the editor.
fn apply(event: &mut IcsEvent, input: &EventInput) {
    event.summary(input.title.trim());
    event.remove_location();
    if !input.location.trim().is_empty() {
        event.location(input.location.trim());
    }
    event.remove_description();
    if !input.description.trim().is_empty() {
        event.description(input.description.trim());
    }
    event.remove_starts();
    event.remove_ends();
    event.remove_property("DURATION");
    if input.all_day {
        event.starts(utc(input.start).date_naive());
        event.ends(utc(input.end).date_naive());
    } else {
        event.starts(utc(input.start));
        event.ends(utc(input.end));
    }
    event.timestamp(Utc::now());
    event.last_modified(Utc::now());
}

/// A new calendar resource with one event.
pub fn new_ics(uid: &str, input: &EventInput) -> String {
    let mut event = IcsEvent::with_uid(uid);
    apply(&mut event, input);
    event.created(Utc::now());
    Calendar::new().push(event.done()).done().to_string()
}

/// A whole calendar as one `.ics` file (Settings › Backups › export), for
/// any calendar app. Repeating events come out as their occurrences.
pub fn export(name: &str, events: &[super::Event]) -> String {
    let mut calendar = Calendar::new();
    calendar.name(name);
    for e in events {
        let uid = match &e.uid {
            // Occurrences share the UID: keep them apart.
            Some(uid) if e.recurring => format!("{uid}-{}", e.start),
            Some(uid) => uid.clone(),
            None => format!("havre-{}@havre", e.id),
        };
        let mut event = IcsEvent::with_uid(&uid);
        apply(
            &mut event,
            &EventInput {
                calendar: e.calendar,
                title: e.title.clone(),
                location: e.location.clone(),
                description: e.description.clone(),
                start: e.start,
                end: e.end,
                all_day: e.all_day,
                attendees: Vec::new(),
            },
        );
        calendar.push(event.done());
    }
    calendar.done().to_string()
}

/// The same resource with the event changed — everything else (alarms,
/// other properties) is kept.
pub fn update_ics(raw: &str, input: &EventInput) -> Result<String> {
    let mut calendar = parse(raw)?;
    let mut found = false;
    for component in calendar.components.iter_mut() {
        if let icalendar::CalendarComponent::Event(event) = component {
            apply(event, input);
            let sequence = event.get_sequence().unwrap_or(0);
            event.sequence(sequence + 1);
            found = true;
            break;
        }
    }
    if !found {
        return Err(Error::Config("no event in this calendar resource".into()));
    }
    Ok(calendar.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    // Monday 5 Oct 2026, 00:00 UTC.
    const MONDAY: i64 = 1_791_158_400;

    fn ics(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:test\r\n{body}END:VCALENDAR\r\n")
    }

    #[test]
    fn reads_a_simple_event_in_a_time_zone() {
        let text = ics("BEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Lunch\r\nLOCATION:Lyon\r\n\
             DTSTART;TZID=Romance Standard Time:20261005T120000\r\nDTEND;TZID=Europe/Paris:20261005T133000\r\n\
             ATTENDEE;CN=Anna;PARTSTAT=ACCEPTED:mailto:anna@example.com\r\nEND:VEVENT\r\n");
        let events =
            events_between(&text, Some("/cal/a.ics"), Some("\"1\""), MONDAY, MONDAY + DAY, true).unwrap();
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.title, "Lunch");
        assert_eq!(e.start, MONDAY + 10 * 3600, "12:00 in Paris is 10:00 UTC (summer time)");
        assert_eq!(e.end - e.start, 90 * 60);
        assert_eq!(e.attendees[0].email, "anna@example.com");
        assert_eq!(e.attendees[0].status, "accepted");
        assert!(!e.read_only);
    }

    #[test]
    fn expands_repeating_events_with_exceptions() {
        let text =
            ics("BEGIN:VEVENT\r\nUID:r\r\nSUMMARY:Standup\r\nDTSTART:20261005T080000Z\r\nDURATION:PT15M\r\n\
             RRULE:FREQ=DAILY;COUNT=10\r\nEXDATE:20261007T080000Z\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:r\r\nRECURRENCE-ID:20261006T080000Z\r\nSUMMARY:Standup (late)\r\n\
             DTSTART:20261006T090000Z\r\nDTEND:20261006T091500Z\r\nEND:VEVENT\r\n");
        let mut events = events_between(&text, None, None, MONDAY, MONDAY + 7 * DAY, true).unwrap();
        events.sort_by_key(|e| e.start);
        let titles: Vec<_> = events.iter().map(|e| (e.title.as_str(), (e.start - MONDAY) / 3600)).collect();
        // Mon 8h, Tue moved to 9h, Wed removed, Thu…Sun 8h.
        assert_eq!(
            titles,
            vec![
                ("Standup", 8),
                ("Standup (late)", 33),
                ("Standup", 80),
                ("Standup", 104),
                ("Standup", 128),
                ("Standup", 152)
            ]
        );
        assert!(events.iter().all(|e| e.recurring && e.read_only && e.end - e.start == 900));
    }

    #[test]
    fn all_day_events_and_writing_back() {
        let text = ics("BEGIN:VEVENT\r\nUID:h\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261006\r\n\
             DTEND;VALUE=DATE:20261008\r\nBEGIN:VALARM\r\nTRIGGER:-PT15M\r\nACTION:DISPLAY\r\nEND:VALARM\r\nEND:VEVENT\r\n");
        let e = &events_between(&text, None, None, MONDAY, MONDAY + 7 * DAY, false).unwrap()[0];
        assert!(e.all_day);
        assert_eq!((e.start, e.end), (MONDAY + DAY, MONDAY + 3 * DAY));

        let input = EventInput {
            calendar: 1,
            title: "Long weekend".into(),
            location: String::new(),
            description: "Away".into(),
            start: MONDAY + 4 * DAY,
            end: MONDAY + 7 * DAY,
            all_day: true,
            attendees: vec![],
        };
        let updated = update_ics(&text, &input).unwrap();
        assert!(updated.contains("VALARM"), "the alarm is kept");
        let e = &events_between(&updated, None, None, MONDAY, MONDAY + 14 * DAY, false).unwrap()[0];
        assert_eq!(
            (e.title.as_str(), e.start, e.end, e.all_day),
            ("Long weekend", MONDAY + 4 * DAY, MONDAY + 7 * DAY, true)
        );

        let created = new_ics(
            "new-1",
            &EventInput { all_day: false, start: MONDAY + 3600, end: MONDAY + 7200, ..input },
        );
        let e = &events_between(&created, None, None, MONDAY, MONDAY + DAY, false).unwrap()[0];
        assert_eq!((e.remote_id.as_str(), e.start, e.end), ("new-1", MONDAY + 3600, MONDAY + 7200));
    }

    #[test]
    fn reads_and_answers_an_invitation() {
        let text = ics("METHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-1\r\nSEQUENCE:2\r\nSUMMARY:Kick-off\r\n\
             ORGANIZER;CN=Claire:mailto:claire@example.com\r\nDTSTART:20261022T080000Z\r\nDTEND:20261022T100000Z\r\n\
             ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:mael@test\r\nEND:VEVENT\r\n");
        let inv = read_invitation(&text).unwrap();
        assert_eq!(inv.method, "REQUEST");
        assert_eq!(inv.uid.as_deref(), Some("inv-1"));
        assert_eq!(inv.organizer_email.as_deref(), Some("claire@example.com"));
        assert_eq!(inv.events.len(), 1);
        let reply = reply_ics(&text, "mael@test", "Maël", "ACCEPTED").unwrap();
        assert!(reply.contains("METHOD:REPLY"));
        assert!(reply.contains("UID:inv-1"));
        assert!(reply.contains("SEQUENCE:2"));
        assert!(reply.contains("PARTSTAT=ACCEPTED"));
        assert!(reply.contains("mailto:mael@test"));
        assert!(!reply.contains("NEEDS-ACTION"), "only my own status");
    }

    #[test]
    fn reads_durations() {
        assert_eq!(duration("PT1H30M"), Some(5400));
        assert_eq!(duration("P1D"), Some(86_400));
        assert_eq!(duration("P2W"), Some(14 * 86_400));
        assert_eq!(duration("P1DT2H"), Some(86_400 + 7200));
    }

    #[test]
    fn finds_the_address() {
        assert_eq!(email_in("Claire Dubois <Claire@Example.com>").as_deref(), Some("claire@example.com"));
        assert_eq!(email_in("anna@example.org").as_deref(), Some("anna@example.org"));
        assert_eq!(email_in("Someone"), None);
    }
}
