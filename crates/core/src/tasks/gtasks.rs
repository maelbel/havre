//! Google Tasks, through its REST API (v1).
//!
//! <https://developers.google.com/tasks/reference/rest>. The same sign-in
//! as Gmail and the Agenda, with the `tasks` permission. Google keeps a due
//! *day* only, and has no reminders in its API: the time and the reminder
//! stay in Havre.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use super::{DAY, RemoteList, RemoteTask, TaskInput, day_of};
use crate::{Error, Result, oauth::http};

fn base() -> String {
    std::env::var("HAVRE_GOOGLE_TASKS_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "https://tasks.googleapis.com/tasks/v1".into())
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
            "Google refused access to Tasks ({message}). Reconnect the account in Settings, and check that the \
             Google Tasks API is enabled in your Google Cloud project."
        )),
        404 | 410 => Error::NotFound("This task no longer exists in Google Tasks.".into()),
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

fn rfc3339(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0).unwrap_or_default().format("%Y-%m-%dT%H:%M:%S.000Z").to_string()
}

fn parse_time(value: &Value) -> Option<i64> {
    value.as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.timestamp())
}

pub async fn lists(token: &str) -> Result<Vec<RemoteList>> {
    let body =
        call(http().get(format!("{}/users/@me/lists?maxResults=100", base())).bearer_auth(token)).await?;
    Ok(body["items"]
        .as_array()
        .map(|lists| {
            lists
                .iter()
                .enumerate()
                .map(|(i, l)| RemoteList {
                    remote_id: l["id"].as_str().unwrap_or_default().to_owned(),
                    name: l["title"].as_str().unwrap_or("My Tasks").to_owned(),
                    color: None,
                    read_only: false,
                    // Google lists "My Tasks" (your default list) first.
                    is_default: i == 0,
                })
                .collect()
        })
        .unwrap_or_default())
}

/// The tasks still to do, and those done in the last 30 days: two
/// questions, because `completedMin` leaves out the tasks not done.
pub async fn tasks(token: &str, list: &str, now: i64) -> Result<Vec<RemoteTask>> {
    let open = format!("{}/lists/{}/tasks?maxResults=100&showCompleted=false", base(), enc(list));
    let done = format!(
        "{}/lists/{}/tasks?maxResults=100&showCompleted=true&showHidden=true&completedMin={}",
        base(),
        enc(list),
        enc(&rfc3339(now - 30 * DAY))
    );
    let mut out = fetch_all(token, &open).await?;
    for task in fetch_all(token, &done).await? {
        if task.done_at.is_some() && !out.iter().any(|t| t.remote_id == task.remote_id) {
            out.push(task);
        }
    }
    Ok(out)
}

async fn fetch_all(token: &str, first: &str) -> Result<Vec<RemoteTask>> {
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    for _ in 0..50 {
        let url = match &page {
            Some(token) => format!("{first}&pageToken={}", enc(token)),
            None => first.to_owned(),
        };
        let body = call(http().get(&url).bearer_auth(token)).await?;
        out.extend(body["items"].as_array().into_iter().flatten().filter_map(to_remote));
        match body["nextPageToken"].as_str() {
            Some(next) => page = Some(next.to_owned()),
            None => break,
        }
    }
    Ok(out)
}

fn to_remote(t: &Value) -> Option<RemoteTask> {
    if t["deleted"].as_bool() == Some(true) {
        return None;
    }
    let done = t["status"].as_str() == Some("completed");
    Some(RemoteTask {
        remote_id: t["id"].as_str()?.to_owned(),
        etag: t["etag"].as_str().map(str::to_owned),
        title: t["title"].as_str().unwrap_or_default().trim().to_owned(),
        notes: t["notes"].as_str().unwrap_or_default().trim().to_owned(),
        // "2026-10-16T00:00:00.000Z": Google says only the date counts.
        due: parse_time(&t["due"]).map(day_of),
        done_at: if done { parse_time(&t["completed"]).or(Some(0)) } else { None },
        updated_at: parse_time(&t["updated"]),
        ..RemoteTask::default()
    })
}

fn body(input: &TaskInput, now: i64) -> Value {
    json!({
        "title": input.title.trim(),
        "notes": input.notes,
        // `null` clears it.
        "due": input.due.map(rfc3339),
        "status": if input.done { "completed" } else { "needsAction" },
        "completed": if input.done { Some(rfc3339(now)) } else { None },
    })
}

pub async fn create(token: &str, list: &str, input: &TaskInput, now: i64) -> Result<RemoteTask> {
    let url = format!("{}/lists/{}/tasks", base(), enc(list));
    let created = call(http().post(url).bearer_auth(token).json(&body(input, now))).await?;
    to_remote(&created).ok_or_else(|| Error::Config("Google didn't return the task".into()))
}

pub async fn update(token: &str, list: &str, id: &str, input: &TaskInput, now: i64) -> Result<RemoteTask> {
    let url = format!("{}/lists/{}/tasks/{}", base(), enc(list), enc(id));
    let changed = call(http().patch(url).bearer_auth(token).json(&body(input, now))).await?;
    to_remote(&changed).ok_or_else(|| Error::Config("Google didn't return the task".into()))
}

pub async fn delete(token: &str, list: &str, id: &str) -> Result<()> {
    let url = format!("{}/lists/{}/tasks/{}", base(), enc(list), enc(id));
    match call(http().delete(url).bearer_auth(token)).await {
        Ok(_) | Err(Error::NotFound(_)) => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_google_tasks() {
        let t = json!({ "id": "x1", "etag": "\"e\"", "title": "Boiler", "notes": "call before 10",
            "status": "needsAction", "due": "2024-10-04T00:00:00.000Z", "updated": "2024-10-01T08:00:00.000Z" });
        let task = to_remote(&t).unwrap();
        assert_eq!(task.due, Some(20_000 * DAY));
        assert_eq!(task.notes, "call before 10");
        assert!(to_remote(&json!({ "id": "gone", "deleted": true })).is_none());

        let open = body(&TaskInput { title: "Boiler".into(), ..Default::default() }, 0);
        assert!(open["due"].is_null() && open["completed"].is_null());
        assert_eq!(open["status"], "needsAction");
        let dated = body(&TaskInput { title: "B".into(), due: Some(20_000 * DAY), ..Default::default() }, 0);
        assert_eq!(dated["due"], "2024-10-04T00:00:00.000Z");
    }
}
