//! CalDAV task lists: to-dos (`VTODO`) on the servers added for the Agenda
//! (Nextcloud Tasks, Fastmail, Radicale…).
//!
//! The same WebDAV as calendars (see `agenda/caldav.rs`, whose helpers are
//! reused): a task list is a calendar collection that accepts `VTODO`s, and
//! each task is an `.ics` file holding one. CalDAV keeps the due time;
//! reminders (`VALARM`) stay in Havre for now.

use chrono::{DateTime, Utc};
use icalendar::{Calendar, CalendarDateTime, Component, DatePerhapsTime, Todo, TodoStatus};

use super::{RemoteList, RemoteTask, TaskInput, local_day};
use crate::{
    Error, Result,
    agenda::{
        caldav::{CALDAV, DAV, absolute, collections, delete_with, href_in, prop, put_with, request},
        ics,
        store::CalDavLogin,
    },
};

const CONFLICT: &str = "This task was changed elsewhere: refresh Tasks and try again.";

/// The collections that hold to-dos.
pub async fn lists(login: &CalDavLogin) -> Result<Vec<RemoteList>> {
    let mut out = Vec::new();
    for c in collections(login).await?.into_iter().filter(|c| c.todos) {
        out.push(RemoteList {
            remote_id: c.href,
            name: c.name,
            color: c.color,
            read_only: c.read_only,
            is_default: out.is_empty(),
        });
    }
    Ok(out)
}

pub async fn tasks(login: &CalDavLogin, list: &str) -> Result<Vec<RemoteTask>> {
    let query = r#"<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/><c:calendar-data/></d:prop>
  <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VTODO"/></c:comp-filter></c:filter>
</c:calendar-query>"#;
    let (status, xml) = request(login, "REPORT", list, "1", query.into()).await?;
    if !(status.is_success() || status.as_u16() == 207) {
        return Err(Error::Config(format!("CalDAV: the task list answered {status}")));
    }
    let doc = roxmltree::Document::parse(&xml)
        .map_err(|e| Error::Config(format!("CalDAV: unreadable answer ({e})")))?;
    let mut out = Vec::new();
    for response in doc.descendants().filter(|n| n.has_tag_name((DAV, "response"))) {
        let Some(href) = href_in(response) else { continue };
        let Some(data) = prop(response, CALDAV, "calendar-data").and_then(|n| n.text()) else { continue };
        let etag = prop(response, DAV, "getetag").and_then(|n| n.text()).map(|t| t.trim().to_owned());
        match read(data, &absolute(list, &href), etag) {
            Ok(Some(task)) => out.push(task),
            Ok(None) => {}
            Err(err) => tracing::warn!(href, "skipping an unreadable task: {err}"),
        }
    }
    Ok(out)
}

/// The first `VTODO` of an `.ics` resource (`None`: it has none).
pub(crate) fn read(text: &str, href: &str, etag: Option<String>) -> Result<Option<RemoteTask>> {
    let calendar = ics::parse(text)?;
    let Some(todo) = calendar.components.iter().find_map(|c| c.as_todo()) else { return Ok(None) };
    let (due, due_at) = match todo.get_due().as_ref().and_then(ics::timestamp) {
        // A date: due that day.
        Some((ts, true)) => (Some(ts), None),
        // A moment: due at that time, on its day (on this server's clock).
        Some((ts, false)) => (Some(local_day(ts)), Some(ts)),
        None => (None, None),
    };
    let done = matches!(todo.get_status(), Some(TodoStatus::Completed)) || todo.get_completed().is_some();
    Ok(Some(RemoteTask {
        remote_id: todo.get_uid().map(str::to_owned).unwrap_or_else(|| href.to_owned()),
        href: Some(href.to_owned()),
        etag,
        raw: Some(text.to_owned()),
        title: todo.get_summary().unwrap_or_default().trim().to_owned(),
        notes: todo.get_description().unwrap_or_default().trim().to_owned(),
        due,
        due_at,
        remind_at: None,
        done_at: if done { Some(todo.get_completed().map(|d| d.timestamp()).unwrap_or(0)) } else { None },
        updated_at: todo.get_last_modified().map(|d| d.timestamp()),
    }))
}

fn utc(ts: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(ts, 0).unwrap_or_default()
}

/// Sets a to-do's fields from the editor (everything else is kept).
fn apply(todo: &mut Todo, input: &TaskInput, now: i64) {
    todo.summary(input.title.trim());
    todo.remove_description();
    if !input.notes.trim().is_empty() {
        todo.description(input.notes.trim());
    }
    todo.remove_due();
    match (input.due, input.due_at) {
        (_, Some(at)) => {
            todo.due(DatePerhapsTime::DateTime(CalendarDateTime::Utc(utc(at))));
        }
        (Some(day), None) => {
            todo.due(utc(day).date_naive());
        }
        (None, None) => {}
    }
    if input.done {
        if todo.get_completed().is_none() {
            todo.completed(utc(now));
        }
        todo.percent_complete(100);
        todo.status(TodoStatus::Completed);
    } else {
        todo.mark_uncompleted();
        todo.status(TodoStatus::NeedsAction);
    }
    todo.timestamp(utc(now));
    todo.last_modified(utc(now));
}

pub(crate) fn new_ics(uid: &str, input: &TaskInput, now: i64) -> String {
    let mut todo = Todo::with_uid(uid);
    apply(&mut todo, input, now);
    todo.created(utc(now));
    Calendar::new().push(todo.done()).done().to_string()
}

pub(crate) fn update_ics(raw: &str, input: &TaskInput, now: i64) -> Result<String> {
    let mut calendar = ics::parse(raw)?;
    let todo = calendar
        .components
        .iter_mut()
        .find_map(|c| match c {
            icalendar::CalendarComponent::Todo(todo) => Some(todo),
            _ => None,
        })
        .ok_or_else(|| Error::Config("no task in this calendar resource".into()))?;
    apply(todo, input, now);
    Ok(calendar.to_string())
}

fn read_back(text: &str, href: &str, etag: Option<String>) -> Result<RemoteTask> {
    read(text, href, etag)?.ok_or_else(|| Error::Config("the saved task can't be read back".into()))
}

pub async fn create(login: &CalDavLogin, list: &str, input: &TaskInput, now: i64) -> Result<RemoteTask> {
    let uid = format!("{}@havre", crate::agenda::random_id());
    let href = format!("{}/{uid}.ics", list.trim_end_matches('/'));
    let text = new_ics(&uid, input, now);
    let etag = put_with(login, &href, None, &text, CONFLICT).await?;
    read_back(&text, &href, etag)
}

pub async fn update(
    login: &CalDavLogin,
    href: &str,
    etag: Option<&str>,
    raw: &str,
    input: &TaskInput,
    now: i64,
) -> Result<RemoteTask> {
    let text = update_ics(raw, input, now)?;
    let etag = put_with(login, href, etag, &text, CONFLICT).await?;
    read_back(&text, href, etag)
}

pub async fn delete(login: &CalDavLogin, href: &str, etag: Option<&str>) -> Result<()> {
    delete_with(login, href, etag, CONFLICT).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::DAY;

    #[test]
    fn reads_and_writes_vtodos() {
        let text = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Nextcloud Tasks\r\nBEGIN:VTODO\r\nUID:abc\r\n\
                    SUMMARY:Book the boiler service\r\nDESCRIPTION:Before winter\r\nDUE;VALUE=DATE:20241004\r\n\
                    X-APPLE-SORT-ORDER:12\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let task = read(text, "https://c/x.ics", Some("\"1\"".into())).unwrap().unwrap();
        assert_eq!(task.remote_id, "abc");
        assert_eq!(task.title, "Book the boiler service");
        assert_eq!(task.due, Some(20_000 * DAY));
        assert_eq!(task.due_at, None);
        assert_eq!(task.done_at, None);

        // Done, with a time: other properties are kept.
        let input = TaskInput {
            title: "Book the boiler".into(),
            due: Some(20_000 * DAY),
            due_at: Some(20_000 * DAY + 10 * 3600),
            done: true,
            ..Default::default()
        };
        let changed = update_ics(text, &input, 20_000 * DAY + 60).unwrap();
        assert!(changed.contains("X-APPLE-SORT-ORDER:12"));
        let back = read(&changed, "https://c/x.ics", None).unwrap().unwrap();
        assert_eq!(back.title, "Book the boiler");
        assert_eq!(back.due_at, Some(20_000 * DAY + 10 * 3600));
        assert_eq!(back.done_at, Some(20_000 * DAY + 60));
        assert_eq!(back.notes, "", "empty notes clear the description");

        // Not done again.
        let reopened = update_ics(&changed, &TaskInput { done: false, ..input }, 0).unwrap();
        assert_eq!(read(&reopened, "h", None).unwrap().unwrap().done_at, None);

        // A new one, and a resource with only an event.
        let fresh = new_ics("n1@havre", &TaskInput { title: "Pay".into(), ..Default::default() }, 0);
        assert_eq!(read(&fresh, "h", None).unwrap().unwrap().remote_id, "n1@havre");
        let event = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:e\r\nDTSTART:20241004T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        assert!(read(event, "h", None).unwrap().is_none());
    }
}
