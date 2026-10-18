//! Microsoft To Do, through Microsoft Graph.
//!
//! <https://learn.microsoft.com/graph/api/resources/todo-overview>. The
//! same sign-in as the Agenda, with the `Tasks.ReadWrite` permission.
//! To Do keeps a due *day* and a reminder moment; the due time stays in
//! Havre.

use chrono::{DateTime, NaiveDateTime, Timelike, Utc};
use serde_json::{Value, json};

use super::{DAY, RemoteList, RemoteTask, TaskInput, day_of};
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
        400 => Error::Config(format!("Microsoft: {message} (400)")),
        401 | 403 => Error::SignIn(format!(
            "Microsoft refused access to To Do ({message}). Reconnect the account in Settings; your admin may \
             need to approve the Tasks.ReadWrite permission."
        )),
        404 => Error::NotFound("This task no longer exists in To Do.".into()),
        _ => Error::Config(format!("Microsoft: {message} ({status})")),
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

pub async fn lists(token: &str) -> Result<Vec<RemoteList>> {
    let body = call(http().get(format!("{}/me/todo/lists?$top=100", base())).bearer_auth(token)).await?;
    Ok(body["value"]
        .as_array()
        .map(|lists| {
            lists
                .iter()
                // "Flagged email" is To Do's view of starred mail: Havre has
                // its own Starred list in Mail.
                .filter(|l| l["wellknownListName"].as_str() != Some("flaggedEmails"))
                .map(|l| RemoteList {
                    remote_id: l["id"].as_str().unwrap_or_default().to_owned(),
                    name: l["displayName"].as_str().unwrap_or("Tasks").to_owned(),
                    color: None,
                    read_only: false,
                    is_default: l["wellknownListName"].as_str() == Some("defaultList"),
                })
                .collect()
        })
        .unwrap_or_default())
}

/// The tasks of a list: those still to do, and those done in the last 30
/// days (older ones would only make the sync slow).
pub async fn tasks(token: &str, list: &str, now: i64) -> Result<Vec<RemoteTask>> {
    let since = iso(now - 30 * DAY);
    let filtered = format!(
        "{}/me/todo/lists/{}/tasks?$top=100&$filter=status ne 'completed' or lastModifiedDateTime ge {since}",
        base(),
        enc(list)
    );
    match fetch_all(token, &filtered).await {
        // Some tenants refuse the filter: fall back to everything.
        Err(Error::Config(msg)) if msg.contains("(400)") => {
            fetch_all(token, &format!("{}/me/todo/lists/{}/tasks?$top=100", base(), enc(list))).await
        }
        other => other,
    }
}

async fn fetch_all(token: &str, first: &str) -> Result<Vec<RemoteTask>> {
    let mut url = first.to_owned();
    let mut out = Vec::new();
    // At most 50 pages of 100: a list that big is synced partly.
    for _ in 0..50 {
        let body = call(http().get(&url).bearer_auth(token)).await?;
        out.extend(body["value"].as_array().into_iter().flatten().filter_map(to_remote));
        match body["@odata.nextLink"].as_str() {
            Some(next) => url = next.to_owned(),
            None => break,
        }
    }
    Ok(out)
}

/// `{ "dateTime": "2026-10-16T00:00:00.0000000", "timeZone": "UTC" }` →
/// unix seconds (read as UTC, which is what we ask for).
fn moment(value: &Value) -> Option<i64> {
    let text = value["dateTime"].as_str()?;
    let text = text.split('.').next()?; // drop the fraction of a second
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").ok().map(|d| d.and_utc().timestamp())
}

/// A due date → the day. To Do stores midnight *in the time zone it was set
/// in*: read in UTC, midnight in Paris is 22:00 the day before. Rounding to
/// the nearest midnight gives the day back, for any time zone.
fn due_day(value: &Value) -> Option<i64> {
    let ts = moment(value)?;
    let hour = DateTime::<Utc>::from_timestamp(ts, 0)?.hour();
    Some(if hour >= 12 { day_of(ts) + DAY } else { day_of(ts) })
}

fn iso(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0).unwrap_or_default().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn graph_time(ts: i64) -> Value {
    json!({
        "dateTime": DateTime::<Utc>::from_timestamp(ts, 0).unwrap_or_default().format("%Y-%m-%dT%H:%M:%S").to_string(),
        "timeZone": "UTC",
    })
}

fn to_remote(t: &Value) -> Option<RemoteTask> {
    let id = t["id"].as_str()?.to_owned();
    let notes = match t["body"]["contentType"].as_str() {
        Some("html") => strip_tags(t["body"]["content"].as_str().unwrap_or_default()),
        _ => t["body"]["content"].as_str().unwrap_or_default().trim().to_owned(),
    };
    let done = t["status"].as_str() == Some("completed");
    Some(RemoteTask {
        remote_id: id,
        title: t["title"].as_str().unwrap_or_default().trim().to_owned(),
        notes,
        due: due_day(&t["dueDateTime"]),
        due_at: None,
        remind_at: if t["isReminderOn"].as_bool() == Some(true) {
            moment(&t["reminderDateTime"])
        } else {
            None
        },
        done_at: if done { moment(&t["completedDateTime"]).or(Some(0)) } else { None },
        updated_at: t["lastModifiedDateTime"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp()),
        ..RemoteTask::default()
    })
}

/// Bodies written in To Do or Outlook can be HTML: keep the text.
fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.replace("<br>", "\n").replace("</p>", "\n").chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_owned()
}

/// What we send to create or change a task.
fn body(input: &TaskInput, now: i64) -> Value {
    json!({
        "title": input.title.trim(),
        "body": { "content": input.notes, "contentType": "text" },
        "dueDateTime": input.due.map(graph_time),
        "isReminderOn": input.remind_at.is_some(),
        "reminderDateTime": input.remind_at.map(graph_time),
        "status": if input.done { "completed" } else { "notStarted" },
        "completedDateTime": if input.done { Some(graph_time(now)) } else { None },
    })
}

pub async fn create(token: &str, list: &str, input: &TaskInput, now: i64) -> Result<RemoteTask> {
    let url = format!("{}/me/todo/lists/{}/tasks", base(), enc(list));
    let created = call(http().post(url).bearer_auth(token).json(&body(input, now))).await?;
    to_remote(&created).ok_or_else(|| Error::Config("Microsoft didn't return the task".into()))
}

pub async fn update(token: &str, list: &str, id: &str, input: &TaskInput, now: i64) -> Result<RemoteTask> {
    let url = format!("{}/me/todo/lists/{}/tasks/{}", base(), enc(list), enc(id));
    let changed = call(http().patch(url).bearer_auth(token).json(&body(input, now))).await?;
    to_remote(&changed).ok_or_else(|| Error::Config("Microsoft didn't return the task".into()))
}

pub async fn delete(token: &str, list: &str, id: &str) -> Result<()> {
    let url = format!("{}/me/todo/lists/{}/tasks/{}", base(), enc(list), enc(id));
    match call(http().delete(url).bearer_auth(token)).await {
        Ok(_) | Err(Error::NotFound(_)) => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_to_do_task() {
        let t = json!({
            "id": "AAMk",
            "title": " Confirm the quote ",
            "status": "notStarted",
            "body": { "content": "<p>4 320 &amp; co</p>", "contentType": "html" },
            // Set in Paris: midnight there is 22:00 UTC the day before.
            "dueDateTime": { "dateTime": "2026-10-15T22:00:00.0000000", "timeZone": "UTC" },
            "isReminderOn": true,
            "reminderDateTime": { "dateTime": "2026-10-16T07:00:00.0000000", "timeZone": "UTC" },
            "lastModifiedDateTime": "2026-10-14T09:00:00.123Z"
        });
        let task = to_remote(&t).unwrap();
        let oct16 = NaiveDateTime::parse_from_str("2026-10-16T00:00:00", "%Y-%m-%dT%H:%M:%S")
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(task.title, "Confirm the quote");
        assert_eq!(task.notes, "4 320 & co");
        assert_eq!(task.due, Some(oct16));
        assert_eq!(task.remind_at, Some(oct16 + 7 * 3600));
        assert_eq!(task.done_at, None);

        let done = json!({ "id": "B", "title": "x", "status": "completed",
            "completedDateTime": { "dateTime": "2026-10-14T00:00:00.0000000", "timeZone": "UTC" } });
        assert!(to_remote(&done).unwrap().done_at.is_some());
    }

    #[test]
    fn writes_days_at_midnight_utc() {
        let input =
            TaskInput { title: "Pay".into(), due: Some(20_000 * DAY), done: true, ..Default::default() };
        let b = body(&input, 20_000 * DAY + 60);
        assert_eq!(b["dueDateTime"]["dateTime"], "2024-10-04T00:00:00");
        assert_eq!(b["status"], "completed");
        assert_eq!(b["isReminderOn"], false);
    }
}
