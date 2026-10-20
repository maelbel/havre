//! The Agenda: calendars and events.
//!
//! Events come from three kinds of calendars, all stored the same way in a
//! local copy (so the week view is instant and works offline):
//!
//! - **Microsoft 365 / Outlook.com** through Microsoft Graph, and **Google
//!   Calendar** through its API — for mail accounts signed in with OAuth2
//!   (the same sign-in, with calendar permission);
//! - **CalDAV** servers (Nextcloud, iCloud, Fastmail…), added in Settings;
//! - **Havre** itself: a local calendar that needs no account.
//!
//! [`Agenda`] (in `manager.rs`) keeps them in sync in the background and
//! writes changes back to the provider. Each provider has its own small
//! module (`graph`, `google`, `caldav`) answering the same four questions:
//! list the calendars, fetch events in a period, create/update/delete one.

pub(crate) mod caldav;
mod google;
mod graph;
pub mod ics;
mod manager;
pub mod repeat;
pub(crate) mod store;

pub use manager::Agenda;
pub(crate) use manager::random_id;
pub use repeat::Repeat;
use serde::{Deserialize, Serialize};
pub use store::{AgendaStore, CalDavAccount};

/// A calendar, as the UI sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Calendar {
    pub id: i64,
    /// `local`, `microsoft`, `google` or `caldav`.
    pub source: String,
    /// The mail account it belongs to (Microsoft, Google).
    pub account: Option<i64>,
    /// The CalDAV server it comes from.
    pub caldav: Option<i64>,
    pub name: String,
    /// `#rrggbb`
    pub color: String,
    pub read_only: bool,
    /// Shown and kept in sync.
    pub enabled: bool,
    /// Where new events go.
    pub is_default: bool,
    pub synced_at: Option<i64>,
    pub error: Option<String>,
}

/// Someone invited to an event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attendee {
    pub name: Option<String>,
    pub email: String,
    /// `accepted`, `declined`, `tentative` or `needs_action`.
    pub status: String,
}

/// An event (one occurrence, for repeating events).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    pub id: i64,
    pub calendar: i64,
    pub title: String,
    pub location: String,
    pub description: String,
    /// Unix seconds. All-day events: midnight UTC of the first day…
    pub start: i64,
    /// …and of the day after the last one (exclusive).
    pub end: i64,
    pub all_day: bool,
    /// An occurrence of a repeating event.
    pub recurring: bool,
    /// How it repeats, when Havre can describe the rule (`None` for a rule
    /// made elsewhere that Havre shows but doesn't rewrite).
    pub repeat: Option<Repeat>,
    /// The first occurrence of its series (there are no "following" ones to
    /// split off: they are all of them).
    pub first_occurrence: bool,
    /// Can't be changed from Havre (someone else's meeting, a read-only
    /// calendar).
    pub read_only: bool,
    pub organizer: Option<String>,
    pub attendees: Vec<Attendee>,
    /// Teams / Meet link.
    pub join_url: Option<String>,
    /// The iCalendar UID: the same for every copy of a meeting (the
    /// invitation in your mail, the event in your calendar).
    pub uid: Option<String>,
    /// Your answer when you're a guest (`accepted`, `tentative`, `declined`,
    /// `needs_action`); `None` for your own events.
    pub my_response: Option<String>,
}

/// What the event editor sends.
#[derive(Debug, Clone, Deserialize)]
pub struct EventInput {
    pub calendar: i64,
    pub title: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub description: String,
    pub start: i64,
    pub end: i64,
    #[serde(default)]
    pub all_day: bool,
    /// Email addresses to invite (Microsoft and Google send the invitations).
    #[serde(default)]
    pub attendees: Vec<String>,
    /// Repeats like this (`None`: doesn't repeat).
    #[serde(default)]
    pub repeat: Option<Repeat>,
    /// The browser's time zone (`Europe/Paris`): a repeating event keeps its
    /// clock time in it.
    #[serde(default)]
    pub time_zone: Option<String>,
}

/// Which occurrences of a repeating event a change or a deletion is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Only this event.
    #[default]
    This,
    /// This one and the following ones.
    Following,
    /// All events of the series.
    All,
}

/// A series' rule, as a provider gives it (read once per series).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SeriesRule {
    /// The provider's id for the series.
    pub series: String,
    /// The rule as the provider wrote it (`RRULE` text, or Microsoft's JSON).
    pub rrule: String,
    pub repeat: Option<Repeat>,
    pub time_zone: Option<String>,
    pub first_start: i64,
    pub first_end: i64,
    pub all_day: bool,
}

/// An event as a provider describes it, before it gets a local id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteEvent {
    /// Unique within the calendar (one per occurrence).
    pub remote_id: String,
    pub href: Option<String>,
    pub etag: Option<String>,
    pub raw: Option<String>,
    pub title: String,
    pub location: String,
    pub description: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub recurring: bool,
    pub read_only: bool,
    pub organizer: Option<String>,
    pub attendees: Vec<Attendee>,
    pub join_url: Option<String>,
    pub uid: Option<String>,
    pub my_response: Option<String>,
    /// Repeating: the series it belongs to…
    pub series: Option<String>,
    /// …and which of its occurrences it is.
    pub original_start: Option<i64>,
    /// The series' rule, when the provider gives it with the occurrences
    /// (iCalendar does; Microsoft and Google are asked separately).
    pub rule: Option<SeriesRule>,
}

/// A calendar as a provider lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteCalendar {
    pub remote_id: String,
    pub name: String,
    pub color: Option<String>,
    pub read_only: bool,
    pub is_default: bool,
}

/// "cloud.example.com" from a CalDAV address: the default name.
pub(crate) fn caldav_name(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or("CalDAV").to_owned()
}

/// Midnight UTC of the day `ts` falls on.
pub fn day_start(ts: i64) -> i64 {
    ts - ts.rem_euclid(86_400)
}

/// The palette used when a provider gives no color.
pub(crate) const COLORS: [&str; 6] = ["#86a9d1", "#a9c79c", "#e3bc6a", "#c99ab0", "#9fb8b0", "#d39a7a"];

pub(crate) fn validate(input: &EventInput) -> crate::Result<()> {
    if input.title.trim().is_empty() {
        return Err(crate::Error::Config("Give the event a title.".into()));
    }
    if input.end <= input.start {
        return Err(crate::Error::Config("The event must end after it starts.".into()));
    }
    if input.all_day && (input.start % 86_400 != 0 || input.end % 86_400 != 0) {
        return Err(crate::Error::Config("All-day events start and end at midnight (UTC).".into()));
    }
    if let Some(repeat) = &input.repeat {
        let tz = repeat::zone(input.time_zone.as_deref())?;
        repeat.validate(input.start, input.all_day, tz)?;
    }
    for email in &input.attendees {
        if !email.contains('@') || email.contains(char::is_whitespace) {
            return Err(crate::Error::Config(format!("“{email}” isn't an email address.")));
        }
    }
    Ok(())
}
