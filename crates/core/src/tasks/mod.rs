//! Tasks: to-dos, each due on a day, linked to what it's about.
//!
//! Tasks live in **lists**, and a list lives in one place:
//!
//! - **Microsoft To Do** (Microsoft Graph) and **Google Tasks**, for mail
//!   accounts signed in with OAuth2 — the same sign-in, with task
//!   permission;
//! - **CalDAV** task lists (`VTODO`, e.g. Nextcloud Tasks) on the servers
//!   added for the Agenda;
//! - **Havre** itself: lists on your server, with no account.
//!
//! Like the Agenda, Havre keeps a local copy (instant, works offline) and
//! [`Tasks`] (in `manager.rs`) keeps it in sync and writes changes to the
//! provider first. Each provider has a small module (`todo`, `gtasks`,
//! `caldav`) answering the same questions: list the lists, fetch the
//! tasks of one, create/update/delete a task.
//!
//! What only Havre knows: a task's **links** to an email, a person and a
//! meeting. They're kept in the local copy and survive syncs.
//!
//! **Dates.** A task is due on a *day* (`due`, midnight UTC of that date,
//! like all-day events), and optionally at a *time* (`due_at`, the moment).
//! Microsoft and Google only keep the day; the time stays in Havre.

mod caldav;
mod gtasks;
mod manager;
pub mod parse;
mod store;
mod todo;

pub use manager::{TaskSource, Tasks};
use serde::{Deserialize, Serialize};
pub use store::{TaskQuery, TaskStore};

/// A list of tasks, as the UI sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskList {
    pub id: i64,
    /// `local`, `microsoft`, `google` or `caldav`.
    pub source: String,
    /// The mail account it belongs to (Microsoft, Google).
    pub account: Option<i64>,
    /// The CalDAV server it comes from.
    pub caldav: Option<i64>,
    /// Who it belongs to, for grouping: the account's name, or "Havre".
    pub owner: String,
    pub name: String,
    /// `#rrggbb`
    pub color: String,
    pub read_only: bool,
    /// Where new tasks go.
    pub is_default: bool,
    /// Tasks still to do.
    pub open: i64,
    pub synced_at: Option<i64>,
    pub error: Option<String>,
}

/// The email a task was made from.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MailLink {
    pub account: i64,
    /// Finds the email wherever it is now.
    pub message_id: Option<String>,
    /// Where it was (when it has no Message-ID).
    pub folder: Option<String>,
    pub uid: Option<u32>,
    pub subject: String,
    /// The sender, as shown.
    pub from: String,
}

/// The person a task is with.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PersonLink {
    pub email: String,
    pub name: Option<String>,
}

/// The meeting a task is for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EventLink {
    pub uid: String,
    pub title: String,
    pub start: i64,
}

/// What a task is about.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Links {
    #[serde(default)]
    pub mail: Option<MailLink>,
    #[serde(default)]
    pub person: Option<PersonLink>,
    #[serde(default)]
    pub event: Option<EventLink>,
}

/// A task.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Task {
    pub id: i64,
    pub list: i64,
    pub title: String,
    pub notes: String,
    /// The day it's due: midnight UTC of that date.
    pub due: Option<i64>,
    /// The moment it's due, when it has a time.
    pub due_at: Option<i64>,
    pub remind_at: Option<i64>,
    /// When it was done; `None` while still to do.
    pub done_at: Option<i64>,
    pub updated_at: Option<i64>,
    /// In a read-only list (shared with you).
    pub read_only: bool,
    #[serde(flatten)]
    pub links: Links,
}

/// What the task editor sends.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TaskInput {
    /// `None`: the default list.
    #[serde(default)]
    pub list: Option<i64>,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub due: Option<i64>,
    #[serde(default)]
    pub due_at: Option<i64>,
    #[serde(default)]
    pub remind_at: Option<i64>,
    #[serde(default)]
    pub done: bool,
    /// `None` keeps the links a task has (when changing it).
    #[serde(default)]
    pub links: Option<Links>,
}

/// A task as a provider describes it, before it gets a local id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteTask {
    /// Unique within the list.
    pub remote_id: String,
    pub href: Option<String>,
    pub etag: Option<String>,
    pub raw: Option<String>,
    pub title: String,
    pub notes: String,
    pub due: Option<i64>,
    pub due_at: Option<i64>,
    pub remind_at: Option<i64>,
    pub done_at: Option<i64>,
    pub updated_at: Option<i64>,
}

/// A list as a provider gives it.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteList {
    pub remote_id: String,
    pub name: String,
    pub color: Option<String>,
    pub read_only: bool,
    pub is_default: bool,
}

pub(crate) const DAY: i64 = 86_400;

/// Midnight UTC of the day `ts` falls on.
pub(crate) fn day_of(ts: i64) -> i64 {
    ts - ts.rem_euclid(DAY)
}

/// The day a moment falls on *on this server's clock* (its time zone, `TZ`
/// with Docker): what "due at 17:00" means for a provider that only keeps
/// days.
pub(crate) fn local_day(ts: i64) -> i64 {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(ts, 0).single() {
        Some(local) => day_of(local.naive_local().and_utc().timestamp()),
        None => day_of(ts),
    }
}

pub(crate) fn validate(input: &TaskInput) -> crate::Result<()> {
    if input.title.trim().is_empty() {
        return Err(crate::Error::Config("Give the task a title.".into()));
    }
    if input.title.chars().count() > 500 {
        return Err(crate::Error::Config(
            "Keep the title under 500 characters (put the rest in the notes).".into(),
        ));
    }
    if input.due.is_some_and(|d| d.rem_euclid(DAY) != 0) {
        return Err(crate::Error::Config("A due day is midnight (UTC).".into()));
    }
    if input.due_at.is_some() && input.due.is_none() {
        return Err(crate::Error::Config("A due time needs a due day.".into()));
    }
    Ok(())
}

/// The palette used when a provider gives no color.
pub(crate) const COLORS: [&str; 6] = ["#86a9d1", "#e3bc6a", "#a9c79c", "#c99ab0", "#9fb8b0", "#d39a7a"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_what_the_editor_sends() {
        let ok = TaskInput { title: "Call Thomas".into(), due: Some(20 * DAY), ..Default::default() };
        assert!(validate(&ok).is_ok());
        assert!(validate(&TaskInput { title: "  ".into(), ..Default::default() }).is_err());
        assert!(validate(&TaskInput { due: Some(20 * DAY + 5), ..ok.clone() }).is_err());
        assert!(validate(&TaskInput { due: None, due_at: Some(5), ..ok.clone() }).is_err());
        assert_eq!(day_of(20 * DAY + 3600), 20 * DAY);
        assert_eq!(day_of(-1), -DAY);
    }
}
