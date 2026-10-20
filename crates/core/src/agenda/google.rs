//! Google Calendar, through its REST API (v3).
//!
//! <https://developers.google.com/calendar/api/v3/reference>. With
//! `singleEvents=true`, repeating events come expanded into occurrences,
//! each with its own id; the series is the event they name in
//! `recurringEventId`, with its rule in `recurrence` (iCalendar lines).

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde_json::{Value, json};

use super::{
    Attendee, EventInput, RemoteCalendar, RemoteEvent, SeriesRule,
    repeat::{self, Repeat},
};
use crate::{Error, Result, oauth::http};

fn base() -> String {
    std::env::var("HAVRE_GOOGLE_CALENDAR_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "https://www.googleapis.com/calendar/v3".into())
        .trim_end_matches('/')
        .to_owned()
}

async fn call(request: reqwest::RequestBuilder) -> Result<Value> {
    let response = request.send().await.map_err(|e| Error::Config(format!("Can't reach Google: {e}")))?;
    let status = response.status();
    if status == reqwest::StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(body);
    }
    let message = body["error"]["message"].as_str().unwrap_or_default();
    Err(match status.as_u16() {
        401 | 403 => Error::SignIn(format!(
            "Google refused calendar access ({message}). Reconnect the account in Settings, and check that the \
             Google Calendar API is enabled in your Google Cloud project."
        )),
        404 | 410 => Error::NotFound("This event no longer exists in Google Calendar.".into()),
        _ => Error::Config(format!("Google: {message} ({status})")),
    })
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub async fn calendars(token: &str) -> Result<Vec<RemoteCalendar>> {
    let body =
        call(http().get(format!("{}/users/me/calendarList?maxResults=250", base())).bearer_auth(token))
            .await?;
    Ok(body["items"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|c| c["accessRole"].as_str() != Some("freeBusyReader"))
                .map(|c| RemoteCalendar {
                    remote_id: c["id"].as_str().unwrap_or_default().to_owned(),
                    name: c["summaryOverride"]
                        .as_str()
                        .or(c["summary"].as_str())
                        .unwrap_or("Calendar")
                        .to_owned(),
                    color: c["backgroundColor"].as_str().filter(|h| h.len() == 7).map(str::to_lowercase),
                    read_only: !matches!(c["accessRole"].as_str(), Some("owner" | "writer")),
                    is_default: c["primary"].as_bool().unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default())
}

pub async fn events(
    token: &str,
    calendar: &str,
    from: i64,
    to: i64,
    read_only: bool,
) -> Result<Vec<RemoteEvent>> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    for _ in 0..50 {
        let mut url = format!(
            "{}/calendars/{}/events?singleEvents=true&maxResults=2500&timeMin={}&timeMax={}",
            base(),
            enc(calendar),
            enc(&rfc3339(from)),
            enc(&rfc3339(to))
        );
        if let Some(token) = &page {
            url.push_str(&format!("&pageToken={}", enc(token)));
        }
        let body = call(http().get(url).bearer_auth(token)).await?;
        for e in body["items"].as_array().into_iter().flatten() {
            if e["status"].as_str() == Some("cancelled") {
                continue;
            }
            if let Some(event) = to_remote(e, read_only) {
                out.push(event);
            }
        }
        match body["nextPageToken"].as_str() {
            Some(next) => page = Some(next.to_owned()),
            None => break,
        }
    }
    Ok(out)
}

fn to_remote(e: &Value, calendar_read_only: bool) -> Option<RemoteEvent> {
    let (start, all_day) = time(&e["start"])?;
    let (end, _) = time(&e["end"]).unwrap_or((start, all_day));
    let attendees = e["attendees"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|a| a["resource"].as_bool() != Some(true))
                .filter_map(|a| {
                    Some(Attendee {
                        name: a["displayName"].as_str().map(str::to_owned),
                        email: a["email"].as_str()?.to_owned(),
                        status: match a["responseStatus"].as_str() {
                            Some("accepted") => "accepted",
                            Some("declined") => "declined",
                            Some("tentative") => "tentative",
                            _ => "needs_action",
                        }
                        .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let organizer = &e["organizer"];
    let mine = organizer["self"].as_bool().unwrap_or(organizer.is_null());
    let series = e["recurringEventId"].as_str().map(str::to_owned);
    let original_start = series.as_ref().map(|_| time(&e["originalStartTime"]).map_or(start, |(t, _)| t));
    Some(RemoteEvent {
        remote_id: e["id"].as_str()?.to_owned(),
        etag: e["etag"].as_str().map(str::to_owned),
        title: e["summary"].as_str().filter(|s| !s.is_empty()).unwrap_or("(no title)").to_owned(),
        location: e["location"].as_str().unwrap_or_default().to_owned(),
        description: e["description"].as_str().unwrap_or_default().to_owned(),
        start,
        end,
        all_day,
        recurring: series.is_some(),
        series,
        original_start,
        read_only: calendar_read_only || (!mine && e["guestsCanModify"].as_bool() != Some(true)),
        organizer: organizer["email"].as_str().map(|addr| match organizer["displayName"].as_str() {
            Some(name) if !name.is_empty() => format!("{name} <{addr}>"),
            _ => addr.to_owned(),
        }),
        attendees,
        join_url: e["hangoutLink"].as_str().map(str::to_owned),
        uid: e["iCalUID"].as_str().map(str::to_owned),
        my_response: if mine {
            None
        } else {
            e["attendees"]
                .as_array()
                .and_then(|list| list.iter().find(|a| a["self"].as_bool() == Some(true)))
                .map(|me| response_name(me["responseStatus"].as_str().unwrap_or_default()).to_owned())
        },
        ..RemoteEvent::default()
    })
}

fn response_name(google: &str) -> &'static str {
    match google {
        "accepted" => "accepted",
        "tentative" => "tentative",
        "declined" => "declined",
        _ => "needs_action",
    }
}

/// Answers an invitation: sets your status among the guests; Google tells
/// the organizer.
pub async fn respond(token: &str, calendar: &str, id: &str, response: &str) -> Result<()> {
    let status = match response {
        "accepted" | "tentative" | "declined" => response,
        _ => return Err(Error::Config(format!("unknown answer “{response}”"))),
    };
    let url = format!("{}/calendars/{}/events/{}", base(), enc(calendar), enc(id));
    let event = call(http().get(&url).bearer_auth(token)).await?;
    let mut attendees = event["attendees"].as_array().cloned().unwrap_or_default();
    let me = attendees
        .iter_mut()
        .find(|a| a["self"].as_bool() == Some(true))
        .ok_or_else(|| Error::Config("You're not a guest of this event.".into()))?;
    me["responseStatus"] = json!(status);
    call(
        http()
            .patch(format!("{url}?sendUpdates=all"))
            .bearer_auth(token)
            .json(&json!({ "attendees": attendees })),
    )
    .await?;
    Ok(())
}

/// `{"dateTime": "2026-10-14T10:00:00+02:00"}` or `{"date": "2026-10-14"}`
fn time(value: &Value) -> Option<(i64, bool)> {
    if let Some(date) = value["date"].as_str() {
        let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
        return Some((date.and_hms_opt(0, 0, 0)?.and_utc().timestamp(), true));
    }
    let dt = DateTime::parse_from_rfc3339(value["dateTime"].as_str()?).ok()?;
    Some((dt.timestamp(), false))
}

fn rfc3339(ts: i64) -> String {
    Utc.timestamp_opt(ts, 0).single().unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The event as Google wants it. `tz`: a repeating event repeats in its
/// time zone (Google needs it to keep the clock time).
fn body(input: &EventInput, tz: Option<Tz>) -> Value {
    let when = |ts: i64| {
        if input.all_day {
            json!({ "date": Utc.timestamp_opt(ts, 0).single().unwrap_or_default().format("%Y-%m-%d").to_string() })
        } else if let Some(tz) = tz {
            json!({ "dateTime": rfc3339(ts), "timeZone": tz.name() })
        } else {
            json!({ "dateTime": rfc3339(ts) })
        }
    };
    let mut body = json!({
        "summary": input.title.trim(),
        "description": input.description,
        "location": input.location.trim(),
        "start": when(input.start),
        "end": when(input.end),
    });
    if !input.attendees.is_empty() {
        body["attendees"] = input.attendees.iter().map(|email| json!({ "email": email.trim() })).collect();
    }
    body
}

/// Creates the event (a repeating one when `input.repeat` is set: the series
/// comes back); `sendUpdates=all` makes Google email the invitations.
pub async fn create(token: &str, calendar: &str, input: &EventInput, tz: Option<Tz>) -> Result<RemoteEvent> {
    let url = format!("{}/calendars/{}/events?sendUpdates=all", base(), enc(calendar));
    let mut event = body(input, tz.filter(|_| input.repeat.is_some()));
    if let Some(repeat) = &input.repeat {
        let rule = repeat.rrule(input.all_day, tz.unwrap_or(Tz::UTC));
        event["recurrence"] = json!([format!("RRULE:{rule}")]);
    }
    let created = call(http().post(url).bearer_auth(token).json(&event)).await?;
    to_remote(&created, false).ok_or_else(|| Error::Config("Google returned an unreadable event".into()))
}

/// Changes one event, or one occurrence of a series.
pub async fn update(token: &str, calendar: &str, id: &str, input: &EventInput) -> Result<RemoteEvent> {
    let url = format!("{}/calendars/{}/events/{}?sendUpdates=all", base(), enc(calendar), enc(id));
    let updated = call(http().patch(url).bearer_auth(token).json(&body(input, None))).await?;
    to_remote(&updated, false).ok_or_else(|| Error::Config("Google returned an unreadable event".into()))
}

/// The series (the event its occurrences name in `recurringEventId`).
async fn master(token: &str, calendar: &str, id: &str) -> Result<(String, Value)> {
    let url = format!("{}/calendars/{}/events/{}", base(), enc(calendar), enc(id));
    let master = call(http().get(&url).bearer_auth(token)).await?;
    Ok((url, master))
}

fn rule_lines(master: &Value) -> Vec<String> {
    master["recurrence"]
        .as_array()
        .map(|lines| lines.iter().filter_map(|l| l.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

/// A series' rule.
pub async fn series(token: &str, calendar: &str, id: &str) -> Result<SeriesRule> {
    let (_, master) = master(token, calendar, id).await?;
    let (start, all_day) =
        time(&master["start"]).ok_or_else(|| Error::Config("Google: a series without a start".into()))?;
    let (end, _) = time(&master["end"]).unwrap_or((start, all_day));
    let tz: Option<Tz> = master["start"]["timeZone"].as_str().and_then(|name| name.parse().ok());
    let lines = rule_lines(&master);
    let rrule = lines.iter().find_map(|l| l.strip_prefix("RRULE:")).unwrap_or_default().to_owned();
    // Extra dates added by hand (`RDATE`): the rule alone doesn't describe it.
    let repeat = if lines.iter().any(|l| l.starts_with("RDATE")) {
        None
    } else {
        Repeat::parse(&rrule, all_day, tz.unwrap_or(Tz::UTC))
    };
    Ok(SeriesRule {
        series: id.to_owned(),
        rrule,
        repeat,
        time_zone: tz.map(|tz| tz.name().to_owned()),
        first_start: start,
        first_end: end,
        all_day,
    })
}

/// Changes the whole series (`input` already moved to its first
/// occurrence). `repeat`: a new rule, or `None` to keep it. `moved`: the
/// occurrences moved, so dates removed from it no longer match.
pub async fn update_series(
    token: &str,
    calendar: &str,
    id: &str,
    input: &EventInput,
    repeat: Option<&Repeat>,
    tz: Tz,
    moved: bool,
) -> Result<()> {
    let (url, master) = master(token, calendar, id).await?;
    let mut lines: Vec<String> =
        rule_lines(&master).into_iter().filter(|l| !(moved && l.starts_with("EXDATE"))).collect();
    if let Some(repeat) = repeat {
        lines.retain(|l| !l.starts_with("RRULE:"));
        lines.insert(0, format!("RRULE:{}", repeat.rrule(input.all_day, tz)));
    }
    let mut event = body(input, Some(tz));
    event["recurrence"] = json!(lines);
    call(http().patch(format!("{url}?sendUpdates=all")).bearer_auth(token).json(&event)).await?;
    Ok(())
}

/// Ends a series just before `original_start`. `false`: nothing would be
/// left (it's the first one), delete the series instead.
pub async fn cut_series(token: &str, calendar: &str, id: &str, original_start: i64) -> Result<bool> {
    let (url, master) = master(token, calendar, id).await?;
    let (first, all_day) = time(&master["start"]).unwrap_or((i64::MIN, false));
    if original_start <= first {
        return Ok(false);
    }
    let tz: Tz = master["start"]["timeZone"].as_str().and_then(|name| name.parse().ok()).unwrap_or(Tz::UTC);
    let lines: Vec<String> = rule_lines(&master)
        .into_iter()
        .map(|l| match l.strip_prefix("RRULE:") {
            Some(rule) => format!("RRULE:{}", repeat::cut_rule(rule, original_start, all_day, tz)),
            None => l,
        })
        .collect();
    call(
        http()
            .patch(format!("{url}?sendUpdates=all"))
            .bearer_auth(token)
            .json(&json!({ "recurrence": lines })),
    )
    .await?;
    Ok(true)
}

pub async fn delete(token: &str, calendar: &str, id: &str) -> Result<()> {
    let url = format!("{}/calendars/{}/events/{}?sendUpdates=all", base(), enc(calendar), enc(id));
    match call(http().delete(url).bearer_auth(token)).await {
        Ok(_) | Err(Error::NotFound(_)) => Ok(()),
        Err(err) => Err(err),
    }
}
