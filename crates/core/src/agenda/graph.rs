//! Microsoft 365 / Outlook.com calendars, through Microsoft Graph.
//!
//! <https://learn.microsoft.com/graph/api/resources/calendar>. Times are
//! asked in UTC (`Prefer: outlook.timezone="UTC"`). `calendarView` returns
//! repeating events already expanded into occurrences, each with its own id,
//! so one occurrence can be changed on its own.

use chrono::{NaiveDateTime, TimeZone, Utc};
use serde_json::{Value, json};

use super::{Attendee, EventInput, RemoteCalendar, RemoteEvent};
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
                      isCancelled,isOrganizer";

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
    Some(RemoteEvent {
        remote_id: e["id"].as_str()?.to_owned(),
        title: e["subject"].as_str().filter(|s| !s.is_empty()).unwrap_or("(no title)").to_owned(),
        location: e["location"]["displayName"].as_str().unwrap_or_default().to_owned(),
        description: e["body"]["content"].as_str().unwrap_or_default().trim().to_owned(),
        start,
        end,
        all_day,
        recurring: matches!(e["type"].as_str(), Some("occurrence" | "exception")),
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

fn body(input: &EventInput) -> Value {
    let when = |ts: i64| {
        let format = if input.all_day { "%Y-%m-%dT00:00:00" } else { "%Y-%m-%dT%H:%M:%S" };
        json!({ "dateTime": Utc.timestamp_opt(ts, 0).single().unwrap_or_default().format(format).to_string(), "timeZone": "UTC" })
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
    body
}

/// Creates the event; Microsoft emails the invitations.
pub async fn create(token: &str, calendar: &str, input: &EventInput) -> Result<RemoteEvent> {
    let url = format!("{}/me/calendars/{}/events", base(), urlencode(calendar));
    let created = call(http().post(url).bearer_auth(token).json(&body(input))).await?;
    to_remote(&created, false).ok_or_else(|| Error::Config("Microsoft returned an unreadable event".into()))
}

pub async fn update(token: &str, id: &str, input: &EventInput) -> Result<RemoteEvent> {
    let url = format!("{}/me/events/{}", base(), urlencode(id));
    let updated = call(http().patch(url).bearer_auth(token).json(&body(input))).await?;
    to_remote(&updated, false).ok_or_else(|| Error::Config("Microsoft returned an unreadable event".into()))
}

pub async fn delete(token: &str, id: &str) -> Result<()> {
    let url = format!("{}/me/events/{}", base(), urlencode(id));
    match call(http().delete(url).bearer_auth(token)).await {
        Ok(_) | Err(Error::NotFound(_)) => Ok(()),
        Err(err) => Err(err),
    }
}
