//! Notes: Markdown notes, attached to a meeting, to people, to an email.
//!
//! Unlike Mail, the Agenda or Tasks, notes don't come from anywhere: they
//! live in Havre's database only (they're in the nightly backups, and each
//! one downloads as a `.md` file). So there's no sync engine here, just a
//! store — [`NoteStore`] — and a small Markdown reader ([`markdown`]).
//!
//! **Links.** A note can be about
//! - one **meeting**: kept like a task's ([`EventLink`]: the iCalendar UID,
//!   the title and the start), so the Agenda finds the note from the event;
//! - any number of **people** (a meeting's attendees, or who the note is
//!   about), by address, so People and the sender's card in Mail show it;
//! - one **email**, by `Message-ID` (found again after a move).
//!
//! **To-dos.** A `- [ ]` line can be ticked from the note, or turned into a
//! task linked to the same meeting and person. The note remembers which
//! task (table `note_tasks`), so ticking the line ticks the task too.

pub mod markdown;
mod store;

use serde::{Deserialize, Serialize};
pub use store::{NoteQuery, NoteStore};

pub use crate::tasks::{EventLink, MailLink, PersonLink};
use markdown::Block;

/// A note, whole.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Note {
    pub id: i64,
    pub title: String,
    /// Markdown.
    pub body: String,
    pub pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub event: Option<EventLink>,
    pub people: Vec<PersonLink>,
    pub mail: Option<MailLink>,
}

/// A note in a list: what the list shows, not the whole text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoteSummary {
    pub id: i64,
    pub title: String,
    /// The start of the text, without Markdown marks.
    pub snippet: String,
    pub pinned: bool,
    pub updated_at: i64,
    pub event: Option<EventLink>,
    pub people: Vec<PersonLink>,
    pub mail: Option<MailLink>,
    /// `- [ ]` lines still open.
    pub open_todos: usize,
}

/// A note ready to read: the note plus its Markdown, read into blocks.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoteView {
    #[serde(flatten)]
    pub note: Note,
    pub blocks: Vec<Block>,
}

/// What the editor sends: the whole note (it's small, and it keeps the
/// API simple — no "which fields changed").
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NoteInput {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub event: Option<EventLink>,
    #[serde(default)]
    pub people: Vec<PersonLink>,
    #[serde(default)]
    pub mail: Option<MailLink>,
}

/// Someone who has notes about them, for the sidebar.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NotePerson {
    pub email: String,
    pub name: Option<String>,
    pub notes: i64,
}

/// What a note started from a meeting looks like: its title, the
/// attendees, and a few headings to fill in (a start, not a form).
pub fn from_meeting(event: EventLink, people: Vec<PersonLink>) -> NoteInput {
    NoteInput {
        title: event.title.clone(),
        body: "## Agenda\n- \n\n## Notes\n\n\n## Decisions\n- \n\n## To-dos\n- [ ] \n".into(),
        pinned: false,
        event: Some(event),
        people,
        mail: None,
    }
}

/// "Re: Fwd: Devis Q4" → "Devis Q4": an email's subject as a note's title.
pub fn title_from_subject(subject: &str) -> String {
    let mut rest = subject.trim();
    // Strip "Re:", "Fwd:", "TR:" (French), "AW:"/"WG:" (German)… as many
    // as there are, in any case.
    loop {
        let lower = rest.to_lowercase();
        let Some(prefix) =
            ["re:", "fw:", "fwd:", "tr:", "aw:", "wg:"].iter().find(|p| lower.starts_with(**p))
        else {
            break;
        };
        rest = rest[prefix.len()..].trim_start();
    }
    rest.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_from_subjects() {
        assert_eq!(title_from_subject("Re: Devis maintenance Q4"), "Devis maintenance Q4");
        assert_eq!(title_from_subject("RE: Fwd:  TR: Lyon"), "Lyon");
        assert_eq!(title_from_subject("Réunion"), "Réunion", "an accent right after isn't a prefix");
        assert_eq!(title_from_subject(""), "");
    }
}
