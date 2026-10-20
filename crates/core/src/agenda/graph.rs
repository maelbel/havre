//! Microsoft 365 / Outlook.com calendars, through Microsoft Graph.
//!
//! <https://learn.microsoft.com/graph/api/resources/calendar>. Times are
//! asked in UTC (`Prefer: outlook.timezone="UTC"`). `calendarView` returns
//! repeating events already expanded into occurrences, each with its own id,
//! so one occurrence can be changed on its own. The series itself (its
//! "series master", with the `recurrence` rule) is read and changed by its
//! own id.

use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde_json::{Value, json};

use super::{
    Attendee, EventInput, RemoteCalendar, RemoteEvent, SeriesRule, ics,
    repeat::{self, Repeat},
};
use crate::{Error, Result, oauth::http};

fn base() -> String {
    std::env::var("HAVRE_MICROSOFT_GRAPH_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "https://graph.microsoft.com/v1.0".into())
        .trim_end_matches('/')
        .to_owned()
}

/// Sends a request and reads JSON, turning Graph errors into sentences.
async fn call(request: reqwest::RequestBuilder) -> Result<Value> {
    let response = request
        .header("Prefer", "outlook.timezone=\"UTC\", outlook.body-content-type=\"text\"")
        .send()
        .await
        .map_err(|e| Error::Config(format!("Can't reach Microsoft: {e}")))?;
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
            "Microsoft refused calendar access ({message}). Reconnect the account in Settings; your admin may \
             need to approve the Calendars.ReadWrite permission."
        )),
        404 => Error::NotFound("This event no longer exists in Outlook.".into()),
        _ => Error::Config(format!("Microsoft: {message} ({status})")),
    })
}

pub async fn calendars(token: &str) -> Result<Vec<RemoteCalendar>> {
    let url = format!("{}/me/calendars?$select=id,name,hexColor,canEdit,isDefaultCalendar&$top=100", base());
    let body = call(http().get(url).bearer_auth(token)).await?;
    Ok(body["value"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|c| RemoteCalendar {
                    remote_id: c["id"].as_str().unwrap_or_default().to_owned(),
                    name: c["name"].as_str().unwrap_or("Calendar").to_owned(),
                    color: c["hexColor"].as_str().filter(|h| h.len() == 7).map(str::to_lowercase),
                    read_only: !c["canEdit"].as_bool().unwrap_or(true),
                    is_default: c["isDefaultCalendar"].as_bool().unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default())
}

const SELECT: &str = "id,iCalUId,responseStatus,subject,location,body,start,end,isAllDay,type,organizer,attendees,onlineMeeting,\
                      isCancelled,isOrganizer,seriesMasterId,originalStart";

pub async fn events(
    token: &str,
    calendar: &str,
    from: i64,
    to: i64,
    read_only: bool,
) -> Result<Vec<RemoteEvent>> {
    let mut url = format!(
        "{}/me/calendars/{}/calendarView?startDateTime={}&endDateTime={}&$top=250&$select={SELECT}",
        base(),
        urlencode(calendar),
        iso(from),
        iso(to)
    );
    let mut out = Vec::new();
    for _ in 0..100 {
        let body = call(http().get(&url).bearer_auth(token)).await?;
        for e in body["value"].as_array().into_iter().flatten() {
            if e["isCancelled"].as_bool() == Some(true) {
                continue;
            }
            if let Some(event) = to_remote(e, read_only) {
                out.push(event);
            }
        }
        match body["@odata.nextLink"].as_str() {
            Some(next) => url = next.to_owned(),
            None => break,
        }
    }
    Ok(out)
}

fn to_remote(e: &Value, calendar_read_only: bool) -> Option<RemoteEvent> {
    let all_day = e["isAllDay"].as_bool().unwrap_or(false);
    let start = time(&e["start"])?;
    let end = time(&e["end"]).unwrap_or(start);
    let attendees = e["attendees"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(Attendee {
                        name: a["emailAddress"]["name"].as_str().map(str::to_owned),
                        email: a["emailAddress"]["address"].as_str()?.to_owned(),
                        status: match a["status"]["response"].as_str() {
                            Some("accepted" | "organizer") => "accepted",
                            Some("declined") => "declined",
                            Some("tentativelyAccepted") => "tentative",
                            _ => "needs_action",
                        }
                        .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let organizer = &e["organizer"]["emailAddress"];
    let recurring = matches!(e["type"].as_str(), Some("occurrence" | "exception"));
    let series = e["seriesMasterId"].as_str().filter(|_| recurring).map(str::to_owned);
    let original_start = e["originalStart"]
        .as_str()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp())
        .or(Some(start))
        .filter(|_| series.is_some());
    Some(RemoteEvent {
        remote_id: e["id"].as_str()?.to_owned(),
        title: e["subject"].as_str().filter(|s| !s.is_empty()).unwrap_or("(no title)").to_owned(),
        location: e["location"]["displayName"].as_str().unwrap_or_default().to_owned(),
        description: e["body"]["content"].as_str().unwrap_or_default().trim().to_owned(),
        start,
        end,
        all_day,
        recurring,
        series,
        original_start,
        // Someone else's meeting: changing your copy wouldn't change theirs.
        read_only: calendar_read_only || e["isOrganizer"].as_bool() == Some(false),
        organizer: organizer["address"].as_str().map(|addr| match organizer["name"].as_str() {
            Some(name) if !name.is_empty() && name != addr => format!("{name} <{addr}>"),
            _ => addr.to_owned(),
        }),
        attendees,
        join_url: e["onlineMeeting"]["joinUrl"].as_str().map(str::to_owned),
        uid: e["iCalUId"].as_str().map(str::to_owned),
        my_response: if e["isOrganizer"].as_bool() == Some(true) {
            None
        } else {
            e["responseStatus"]["response"].as_str().map(|r| response_name(r).to_owned())
        },
        ..RemoteEvent::default()
    })
}

/// Graph's answer names → Havre's.
fn response_name(graph: &str) -> &'static str {
    match graph {
        "accepted" => "accepted",
        "tentativelyAccepted" => "tentative",
        "declined" => "declined",
        _ => "needs_action",
    }
}

/// Answers an invitation; Microsoft tells the organizer.
pub async fn respond(token: &str, id: &str, response: &str) -> Result<()> {
    let action = match response {
        "accepted" => "accept",
        "tentative" => "tentativelyAccept",
        "declined" => "decline",
        _ => return Err(Error::Config(format!("unknown answer “{response}”"))),
    };
    let url = format!("{}/me/events/{}/{action}", base(), urlencode(id));
    call(http().post(url).bearer_auth(token).json(&json!({ "sendResponse": true }))).await?;
    Ok(())
}

/// `{"dateTime": "2026-10-14T08:00:00.0000000", "timeZone": "UTC"}`
fn time(value: &Value) -> Option<i64> {
    let text = value["dateTime"].as_str()?;
    let text = text.split('.').next()?;
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").ok().map(|t| t.and_utc().timestamp())
}

fn iso(ts: i64) -> String {
    Utc.timestamp_opt(ts, 0).single().unwrap_or_default().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The name Microsoft gets for a time zone: the Windows one when it's a
/// common zone (what Outlook itself writes), else the IANA name.
fn zone_name(tz: Tz) -> &'static str {
    ics::WINDOWS_ZONES.iter().find(|(_, iana)| *iana == tz.name()).map_or(tz.name(), |(windows, _)| windows)
}

/// The IANA zone from what Microsoft writes (a Windows or an IANA name).
fn zone_from(name: &str) -> Option<Tz> {
    let iana = ics::WINDOWS_ZONES.iter().find(|(windows, _)| *windows == name).map_or(name, |(_, iana)| iana);
    iana.parse().ok()
}

/// The event as Microsoft wants it. `tz`: a repeating event's times are
/// written in its time zone, so it repeats at the same clock time.
fn body(input: &EventInput, repeat: Option<&Repeat>, tz: Option<Tz>) -> Value {
    let zone = tz.filter(|_| !input.all_day);
    let when = |ts: i64| match zone {
        Some(tz) => json!({
            "dateTime": repeat::local_time(ts, tz).format("%Y-%m-%dT%H:%M:%S").to_string(),
            "timeZone": zone_name(tz),
        }),
        None => {
            let format = if input.all_day { "%Y-%m-%dT00:00:00" } else { "%Y-%m-%dT%H:%M:%S" };
            json!({ "dateTime": Utc.timestamp_opt(ts, 0).single().unwrap_or_default().format(format).to_string(), "timeZone": "UTC" })
        }
    };
    let mut body = json!({
        "subject": input.title.trim(),
        "body": { "contentType": "text", "content": input.description },
        "location": { "displayName": input.location.trim() },
        "start": when(input.start),
        "end": when(input.end),
        "isAllDay": input.all_day,
    });
    if !input.attendees.is_empty() {
        body["attendees"] = input
            .attendees
            .iter()
            .map(|email| json!({ "emailAddress": { "address": email.trim() }, "type": "required" }))
            .collect();
    }
    if let Some(repeat) = repeat {
        let tz = tz.unwrap_or(Tz::UTC);
        body["recurrence"] = repeat.graph(input.start, input.all_day, tz, zone_name(tz));
    }
    body
}

/// Creates the event (a repeating one when `input.repeat` is set: the
/// series master comes back); Microsoft emails the invitations.
pub async fn create(token: &str, calendar: &str, input: &EventInput, tz: Option<Tz>) -> Result<RemoteEvent> {
    let url = format!("{}/me/calendars/{}/events", base(), urlencode(calendar));
    let created =
        call(http().post(url).bearer_auth(token).json(&body(input, input.repeat.as_ref(), tz))).await?;
    to_remote(&created, false).ok_or_else(|| Error::Config("Microsoft returned an unreadable event".into()))
}

/// Changes one event, or one occurrence of a series.
pub async fn update(token: &str, id: &str, input: &EventInput) -> Result<RemoteEvent> {
    let url = format!("{}/me/events/{}", base(), urlencode(id));
    let updated = call(http().patch(url).bearer_auth(token).json(&body(input, None, None))).await?;
    to_remote(&updated, false).ok_or_else(|| Error::Config("Microsoft returned an unreadable event".into()))
}

/// A series' rule, from its series master.
pub async fn series(token: &str, id: &str) -> Result<SeriesRule> {
    let url = format!(
        "{}/me/events/{}?$select=recurrence,start,end,isAllDay,originalStartTimeZone",
        base(),
        urlencode(id)
    );
    let master = call(http().get(url).bearer_auth(token)).await?;
    let start =
        time(&master["start"]).ok_or_else(|| Error::Config("Microsoft: a series without a start".into()))?;
    let end = time(&master["end"]).unwrap_or(start);
    let recurrence = &master["recurrence"];
    let tz = recurrence["range"]["recurrenceTimeZone"]
        .as_str()
        .or(master["originalStartTimeZone"].as_str())
        .and_then(zone_from);
    Ok(SeriesRule {
        series: id.to_owned(),
        rrule: recurrence.to_string(),
        repeat: Repeat::from_graph(recurrence),
        time_zone: tz.map(|tz| tz.name().to_owned()),
        first_start: start,
        first_end: end,
        all_day: master["isAllDay"].as_bool().unwrap_or(false),
    })
}

/// Changes the whole series (`input` already moved to the series' first
/// occurrence). `repeat`: a new rule, or `None` to keep it.
pub async fn update_series(
    token: &str,
    id: &str,
    input: &EventInput,
    repeat: Option<&Repeat>,
    tz: Tz,
) -> Result<()> {
    let url = format!("{}/me/events/{}", base(), urlencode(id));
    call(http().patch(url).bearer_auth(token).json(&body(input, repeat, Some(tz)))).await?;
    Ok(())
}

/// Ends a series the day before `original_start` (Microsoft's rule ends on
/// a date). `false`: nothing would be left, delete the series instead.
pub async fn cut_series(token: &str, id: &str, original_start: i64, all_day: bool) -> Result<bool> {
    let url = format!("{}/me/events/{}", base(), urlencode(id));
    let master = call(http().get(format!("{url}?$select=recurrence")).bearer_auth(token)).await?;
    let mut recurrence = master["recurrence"].clone();
    if recurrence.is_null() {
        return Err(Error::Config(
            "This event doesn't repeat in Outlook any more: refresh the Agenda.".into(),
        ));
    }
    let tz = recurrence["range"]["recurrenceTimeZone"].as_str().and_then(zone_from).unwrap_or(Tz::UTC);
    let last = repeat::local_date(original_start, all_day, tz) - Duration::days(1);
    let first = recurrence["range"]["startDate"].as_str().unwrap_or_default().to_owned();
    let last = last.format("%Y-%m-%d").to_string();
    if last < first {
        return Ok(false);
    }
    let range = &mut recurrence["range"];
    if range["type"] == "endDate" && range["endDate"].as_str().is_some_and(|end| end <= last.as_str()) {
        return Ok(true); // it already ends before
    }
    range["type"] = json!("endDate");
    range["endDate"] = json!(last);
    if let Some(range) = range.as_object_mut() {
        range.remove("numberOfOccurrences");
    }
    call(http().patch(url).bearer_auth(token).json(&json!({ "recurrence": recurrence }))).await?;
    Ok(true)
}

pub async fn delete(token: &str, id: &str) -> Result<()> {
    let url = format!("{}/me/events/{}", base(), urlencode(id));
    match call(http().delete(url).bearer_auth(token)).await {
        Ok(_) | Err(Error::NotFound(_)) => Ok(()),
        Err(err) => Err(err),
    }
}
