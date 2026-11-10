//! Buttons on notifications (step 26): what they are, what they act on, and
//! which go together.
//!
//! A notification carries its buttons (`actions`, shown by the service
//! worker) and a [`Target`] (`data`): the email, task, event or invitation.
//! Tapping a button sends both back to `POST /api/push/action`. The server
//! never trusts them blindly: [`Action::parse`] only knows Havre's buttons,
//! [`Action::check`] refuses a button on the wrong kind of target, and the
//! work is done with the same functions as the app's own buttons, which look
//! the email, task or event up (and fail if it isn't there).

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// A button, as the service worker shows it (`{action, title}` is what
/// `showNotification` wants).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Button {
    pub action: String,
    pub title: String,
}

/// What a notification's buttons act on. `#[serde(tag = "kind")]` writes the
/// variant's name in the JSON next to its fields:
/// `{"kind": "mail", "account": 1, "folder": "INBOX", "uid": 42}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// One email.
    Mail {
        account: i64,
        folder: String,
        uid: u32,
    },
    /// Several new emails in one notification.
    Inbox {
        account: i64,
        folder: String,
        uids: Vec<u32>,
    },
    Task {
        task: i64,
    },
    /// An event's occurrence (a repeating event has several starts).
    Event {
        event: i64,
        start: i64,
    },
    /// The invitation attached to an email (`index`: which attachment).
    Invitation {
        account: i64,
        folder: String,
        uid: u32,
        index: usize,
    },
}

/// What a button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Archive,
    /// Mark as read (one email, or all of a summary's).
    Read,
    /// A task: done.
    Done,
    /// A reminder: again in this many minutes.
    Snooze(i64),
    Accept,
    Decline,
}

/// The snoozes a button may ask for, in minutes.
const SNOOZES: [i64; 3] = [5, 10, 60];

impl Action {
    /// The button's name as the service worker sends it back (`"snooze-10"`).
    pub fn name(self) -> String {
        match self {
            Action::Archive => "archive".into(),
            Action::Read => "read".into(),
            Action::Done => "done".into(),
            Action::Snooze(minutes) => format!("snooze-{minutes}"),
            Action::Accept => "accept".into(),
            Action::Decline => "decline".into(),
        }
    }

    /// The other way round. Anything that isn't one of Havre's buttons is
    /// refused (the request could come from anywhere).
    pub fn parse(name: &str) -> Result<Self> {
        let action = match name {
            "archive" => Action::Archive,
            "read" => Action::Read,
            "done" => Action::Done,
            "accept" => Action::Accept,
            "decline" => Action::Decline,
            other => {
                // `strip_prefix` gives the rest after "snooze-", or `None`.
                let minutes = other.strip_prefix("snooze-").and_then(|m| m.parse::<i64>().ok());
                match minutes {
                    Some(m) if SNOOZES.contains(&m) => Action::Snooze(m),
                    _ => return Err(Error::Config(format!("Havre has no “{other}” button."))),
                }
            }
        };
        Ok(action)
    }

    /// Whether this button goes with this kind of target.
    pub fn check(self, target: &Target) -> Result<()> {
        let fits = matches!(
            (self, target),
            (Action::Archive, Target::Mail { .. })
                | (Action::Read, Target::Mail { .. } | Target::Inbox { .. })
                | (Action::Done, Target::Task { .. })
                | (Action::Snooze(_), Target::Task { .. } | Target::Event { .. })
                | (Action::Accept | Action::Decline, Target::Invitation { .. })
        );
        if fits {
            Ok(())
        } else {
            Err(Error::Config("This button doesn't go with this notification.".into()))
        }
    }

    /// The button for a notification.
    pub fn button(self, title: &str) -> Button {
        Button { action: self.name(), title: title.to_owned() }
    }

    /// The words for "it didn't work" (`Couldn't archive`).
    pub fn verb(self) -> &'static str {
        match self {
            Action::Archive => "archive",
            Action::Read => "mark as read",
            Action::Done => "tick the task",
            Action::Snooze(_) => "remind you later",
            Action::Accept => "accept",
            Action::Decline => "decline",
        }
    }
}

/// "In 10 min", "In 1 hour".
pub(crate) fn snooze_title(minutes: i64) -> String {
    if minutes % 60 == 0 {
        let hours = minutes / 60;
        if hours == 1 { "In 1 hour".into() } else { format!("In {hours} hours") }
    } else {
        format!("In {minutes} min")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_go_both_ways_and_unknown_ones_are_refused() {
        for action in [
            Action::Archive,
            Action::Read,
            Action::Done,
            Action::Snooze(5),
            Action::Snooze(60),
            Action::Accept,
            Action::Decline,
        ] {
            assert_eq!(Action::parse(&action.name()).unwrap(), action);
        }
        // Only the snoozes a button offers, and nothing else.
        for bad in ["snooze-7", "snooze-", "snooze-x", "delete", ""] {
            assert!(Action::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(snooze_title(10), "In 10 min");
        assert_eq!(snooze_title(60), "In 1 hour");
    }

    #[test]
    fn buttons_only_fit_their_targets() {
        let mail = Target::Mail { account: 1, folder: "INBOX".into(), uid: 3 };
        let task = Target::Task { task: 4 };
        let invitation = Target::Invitation { account: 1, folder: "INBOX".into(), uid: 3, index: 0 };
        assert!(Action::Archive.check(&mail).is_ok());
        assert!(Action::Archive.check(&task).is_err());
        assert!(Action::Done.check(&mail).is_err());
        assert!(Action::Snooze(10).check(&task).is_ok());
        assert!(Action::Snooze(10).check(&mail).is_err());
        assert!(Action::Accept.check(&invitation).is_ok());
        assert!(Action::Accept.check(&mail).is_err());
        // The JSON a notification carries, and reads back.
        let json = serde_json::to_value(&mail).unwrap();
        assert_eq!(json, serde_json::json!({"kind": "mail", "account": 1, "folder": "INBOX", "uid": 3}));
        assert_eq!(serde_json::from_value::<Target>(json).unwrap(), mail);
        assert!(serde_json::from_value::<Target>(serde_json::json!({"kind": "nothing"})).is_err());
    }
}
