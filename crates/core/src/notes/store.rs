//! The Notes tables: notes, who they're about, the tasks made from them.

use std::collections::HashMap;

use serde::Deserialize;
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use super::markdown::{self, Block};
use super::{EventLink, MailLink, Note, NoteInput, NotePerson, NoteSummary, NoteView, PersonLink};
use crate::{Error, Result};

#[derive(Clone)]
pub struct NoteStore {
    pool: SqlitePool,
}

/// Which notes to list (`GET /api/notes?…`). Every filter is optional.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NoteQuery {
    /// Words in the title or the text (all of them, in any order).
    pub q: Option<String>,
    /// About this person (address).
    pub person: Option<String>,
    /// For this meeting (its iCalendar UID)…
    pub event_uid: Option<String>,
    /// …and this occurrence, for a repeating one.
    pub event_start: Option<i64>,
    /// Made from this email.
    pub message_id: Option<String>,
    pub pinned: Option<bool>,
    pub limit: Option<i64>,
}

const COLUMNS: &str = "id, title, body, pinned, created_at, updated_at, event_uid, event_title, event_start, \
     mail_account, mail_message_id, mail_folder, mail_uid, mail_subject, mail_from";

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

impl NoteStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Takes the write lock at once (see `TaskStore::write` for why).
    async fn write(&self) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }

    /// Notes matching `query`: pinned first, then the latest edited.
    pub async fn notes(&self, query: &NoteQuery) -> Result<Vec<NoteSummary>> {
        // Same trick as Tasks: `(? IS NULL OR …)` turns a filter off when
        // its value is missing. SQL finds notes with the first word; Rust
        // checks the others.
        let words: Vec<String> =
            query.q.as_deref().unwrap_or_default().split_whitespace().map(str::to_lowercase).collect();
        let first = words.first().map(|w| format!("%{w}%"));
        let person = query.person.as_ref().map(|p| p.trim().to_lowercase());
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM notes n
             WHERE (? IS NULL OR n.title LIKE ? OR n.body LIKE ?)
               AND (? IS NULL OR EXISTS (SELECT 1 FROM note_people p WHERE p.note_id = n.id AND p.email = ?))
               AND (? IS NULL OR n.event_uid = ?)
               AND (? IS NULL OR n.event_start = ?)
               AND (? IS NULL OR n.mail_message_id = ?)
               AND (? IS NULL OR n.pinned = ?)
             ORDER BY n.pinned DESC, n.updated_at DESC, n.id DESC"
        ))
        .bind(&first)
        .bind(&first)
        .bind(&first)
        .bind(&person)
        .bind(&person)
        .bind(&query.event_uid)
        .bind(&query.event_uid)
        .bind(query.event_start)
        .bind(query.event_start)
        .bind(&query.message_id)
        .bind(&query.message_id)
        .bind(query.pinned)
        .bind(query.pinned)
        .fetch_all(&self.pool)
        .await?;

        let mut people = self.all_people().await?;
        let limit = query.limit.unwrap_or(2000).clamp(1, 5000) as usize;
        let notes = rows
            .iter()
            .map(note_from_row)
            .filter(|n| {
                let text = format!("{} {}", n.title, n.body).to_lowercase();
                words.iter().all(|w| text.contains(w.as_str()))
            })
            .take(limit)
            .map(|n| NoteSummary {
                snippet: markdown::snippet(&n.body, 160),
                open_todos: markdown::open_todos(&n.body),
                // `remove` hands over the Vec instead of copying it.
                people: people.remove(&n.id).unwrap_or_default(),
                id: n.id,
                title: n.title,
                pinned: n.pinned,
                updated_at: n.updated_at,
                event: n.event,
                mail: n.mail,
            })
            .collect();
        Ok(notes)
    }

    /// How many notes there are (the App library's Notes card).
    pub async fn count(&self) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM notes").fetch_one(&self.pool).await?)
    }

    pub async fn note(&self, id: i64) -> Result<Note> {
        let row = sqlx::query(&format!("SELECT {COLUMNS} FROM notes WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("note {id}")))?;
        let mut note = note_from_row(&row);
        note.people = self.people_of(id).await?;
        Ok(note)
    }

    /// A note with its Markdown read into blocks, and each to-do's task.
    pub async fn view(&self, id: i64) -> Result<NoteView> {
        let note = self.note(id).await?;
        let tasks: HashMap<String, i64> =
            sqlx::query_as::<_, (String, i64)>("SELECT text, task_id FROM note_tasks WHERE note_id = ?")
                .bind(id)
                .fetch_all(&self.pool)
                .await?
                .into_iter()
                .collect();
        let mut blocks = markdown::parse(&note.body);
        for block in &mut blocks {
            // `if let` with a pattern that binds by `&mut`: we change the
            // block in place.
            if let Block::Todo { text, task, .. } = block {
                *task = tasks.get(text.as_str()).copied();
            }
        }
        Ok(NoteView { note, blocks })
    }

    pub async fn create(&self, input: &NoteInput) -> Result<NoteView> {
        let now = now();
        let mut tx = self.write().await?;
        let id = sqlx::query("INSERT INTO notes (title, body, created_at, updated_at) VALUES ('', '', ?, ?)")
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?
            .last_insert_rowid();
        write_note(&mut tx, id, input, now).await?;
        tx.commit().await?;
        self.view(id).await
    }

    /// Saves the whole note (title, text, pin, links).
    pub async fn update(&self, id: i64, input: &NoteInput) -> Result<NoteView> {
        self.note(id).await?; // 404 if it's gone
        let mut tx = self.write().await?;
        write_note(&mut tx, id, input, now()).await?;
        tx.commit().await?;
        self.view(id).await
    }

    pub async fn set_pinned(&self, id: i64, pinned: bool) -> Result<NoteView> {
        // Pinning isn't an edit: `updated_at` stays.
        let done = sqlx::query("UPDATE notes SET pinned = ? WHERE id = ?")
            .bind(pinned)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound(format!("note {id}")));
        }
        self.view(id).await
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        // `note_people` and `note_tasks` go with it (`ON DELETE CASCADE`);
        // the tasks themselves stay in Tasks.
        sqlx::query("DELETE FROM notes WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Ticks or unticks the to-do on `line`. Also returns the task made
    /// from it, if any, so the caller can tick that too.
    pub async fn set_todo(&self, id: i64, line: usize, done: bool) -> Result<(NoteView, Option<i64>)> {
        let note = self.note(id).await?;
        let body = markdown::set_todo(&note.body, line, done)
            .ok_or_else(|| Error::Config("This line is no longer a to-do. Reload the note.".into()))?;
        sqlx::query("UPDATE notes SET body = ?, updated_at = ? WHERE id = ?")
            .bind(&body)
            .bind(now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        let view = self.view(id).await?;
        let task = view.blocks.iter().find_map(|b| match b {
            Block::Todo { line: l, task, .. } if *l == line => *task,
            _ => None,
        });
        Ok((view, task))
    }

    /// The to-do on `line`, to make a task of it: its text, if it has one.
    pub async fn todo_text(&self, id: i64, line: usize) -> Result<String> {
        let note = self.note(id).await?;
        markdown::parse(&note.body)
            .into_iter()
            .find_map(|b| match b {
                Block::Todo { line: l, text, .. } if l == line && !text.is_empty() => Some(text),
                _ => None,
            })
            .ok_or_else(|| Error::Config("This line is no longer a to-do. Reload the note.".into()))
    }

    /// Remembers that the to-do `text` became task `task`.
    pub async fn link_task(&self, id: i64, text: &str, task: i64) -> Result<NoteView> {
        sqlx::query(
            "INSERT INTO note_tasks (note_id, text, task_id) VALUES (?, ?, ?)
             ON CONFLICT (note_id, text) DO UPDATE SET task_id = excluded.task_id",
        )
        .bind(id)
        .bind(text)
        .bind(task)
        .execute(&self.pool)
        .await?;
        self.view(id).await
    }

    /// Everyone with notes about them, most notes first.
    pub async fn people(&self) -> Result<Vec<NotePerson>> {
        let rows: Vec<(String, Option<String>, i64)> = sqlx::query_as(
            "SELECT email, MAX(name), COUNT(*) FROM note_people GROUP BY email
             ORDER BY COUNT(*) DESC, MAX(name) COLLATE NOCASE",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(email, name, notes)| NotePerson { email, name, notes }).collect())
    }

    async fn people_of(&self, id: i64) -> Result<Vec<PersonLink>> {
        let rows: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT email, name FROM note_people WHERE note_id = ? ORDER BY rowid")
                .bind(id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(email, name)| PersonLink { email, name }).collect())
    }

    /// Every note's people at once (one query instead of one per note).
    async fn all_people(&self) -> Result<HashMap<i64, Vec<PersonLink>>> {
        let rows: Vec<(i64, String, Option<String>)> =
            sqlx::query_as("SELECT note_id, email, name FROM note_people ORDER BY rowid")
                .fetch_all(&self.pool)
                .await?;
        let mut map: HashMap<i64, Vec<PersonLink>> = HashMap::new();
        for (id, email, name) in rows {
            // `entry(..).or_default()`: the Vec for that note, made empty
            // the first time.
            map.entry(id).or_default().push(PersonLink { email, name });
        }
        Ok(map)
    }
}

/// Writes everything the editor sent, inside the caller's transaction.
async fn write_note(
    tx: &mut sqlx::Transaction<'static, sqlx::Sqlite>,
    id: i64,
    input: &NoteInput,
    now: i64,
) -> Result<()> {
    let event = input.event.as_ref();
    let mail = input.mail.as_ref();
    sqlx::query(
        "UPDATE notes SET title = ?, body = ?, pinned = ?, updated_at = ?,
                event_uid = ?, event_title = ?, event_start = ?,
                mail_account = ?, mail_message_id = ?, mail_folder = ?, mail_uid = ?, mail_subject = ?, mail_from = ?
         WHERE id = ?",
    )
    .bind(input.title.trim())
    .bind(&input.body)
    .bind(input.pinned)
    .bind(now)
    .bind(event.map(|e| e.uid.clone()))
    .bind(event.map(|e| e.title.clone()))
    .bind(event.map(|e| e.start))
    .bind(mail.map(|m| m.account))
    .bind(mail.and_then(|m| m.message_id.clone()).filter(|id| !id.trim().is_empty()))
    .bind(mail.and_then(|m| m.folder.clone()))
    .bind(mail.and_then(|m| m.uid).map(i64::from))
    .bind(mail.map(|m| m.subject.clone()))
    .bind(mail.map(|m| m.from.clone()))
    .bind(id)
    .execute(&mut **tx)
    .await?;

    // People: replaced as a whole. Addresses are kept lowercase so that
    // "Thomas@Renard.fr" and "thomas@renard.fr" are one person.
    sqlx::query("DELETE FROM note_people WHERE note_id = ?").bind(id).execute(&mut **tx).await?;
    for person in &input.people {
        let email = person.email.trim().to_lowercase();
        if email.is_empty() {
            continue;
        }
        sqlx::query("INSERT OR IGNORE INTO note_people (note_id, email, name) VALUES (?, ?, ?)")
            .bind(id)
            .bind(email)
            .bind(person.name.as_deref().map(str::trim).filter(|n| !n.is_empty()))
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

fn note_from_row(row: &SqliteRow) -> Note {
    let event = row.get::<Option<String>, _>("event_uid").map(|uid| EventLink {
        uid,
        title: row.get::<Option<String>, _>("event_title").unwrap_or_default(),
        start: row.get::<Option<i64>, _>("event_start").unwrap_or_default(),
    });
    let mail = row.get::<Option<i64>, _>("mail_account").map(|account| MailLink {
        account,
        message_id: row.get("mail_message_id"),
        folder: row.get("mail_folder"),
        uid: row.get::<Option<i64>, _>("mail_uid").map(|u| u as u32),
        subject: row.get::<Option<String>, _>("mail_subject").unwrap_or_default(),
        from: row.get::<Option<String>, _>("mail_from").unwrap_or_default(),
    });
    Note {
        id: row.get("id"),
        title: row.get("title"),
        body: row.get("body"),
        pinned: row.get("pinned"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        event,
        people: Vec::new(), // filled in by the caller
        mail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::from_meeting;
    use crate::store::Database;
    use crate::tasks::TaskQuery;

    fn person(email: &str, name: &str) -> PersonLink {
        PersonLink { email: email.into(), name: Some(name.into()) }
    }

    #[tokio::test]
    async fn notes_and_their_links() {
        let db = Database::in_memory().await.unwrap();
        let notes = db.notes();

        // From a meeting: its title, its people, the headings.
        let kickoff = EventLink { uid: "kick@lyon".into(), title: "Lyon kick-off".into(), start: 1_000 };
        let input = from_meeting(kickoff.clone(), vec![person("Claire@Lyon.fr", "Claire Martin")]);
        let note = notes.create(&input).await.unwrap();
        assert_eq!(note.note.title, "Lyon kick-off");
        assert_eq!(note.note.people, vec![person("claire@lyon.fr", "Claire Martin")]);
        assert!(note.blocks.iter().any(|b| matches!(b, Block::Todo { .. })));

        // Edited: text with a to-do, a second person.
        let mut input = input.clone();
        input.body = "Planning first.\n\n- [ ] Send Claire the planning\n- [ ] Book the train".into();
        input.people.push(person("julie@havre.test", "Julie"));
        let note = notes.update(note.note.id, &input).await.unwrap();
        assert_eq!(note.note.people.len(), 2);

        let other = notes
            .create(&NoteInput { title: "Boiler".into(), body: "Two quotes".into(), ..Default::default() })
            .await
            .unwrap();

        // Listing and filters.
        let all = notes.notes(&NoteQuery::default()).await.unwrap();
        assert_eq!(all.len(), 2);
        let first = all.iter().find(|n| n.id == note.note.id).unwrap();
        assert_eq!(first.open_todos, 2);
        assert_eq!(first.snippet, "Planning first. · Send Claire the planning · Book the train");
        let by_person = NoteQuery { person: Some("CLAIRE@lyon.fr".into()), ..Default::default() };
        assert_eq!(notes.notes(&by_person).await.unwrap().len(), 1);
        let by_event = NoteQuery { event_uid: Some("kick@lyon".into()), ..Default::default() };
        assert_eq!(notes.notes(&by_event).await.unwrap().len(), 1);
        let other_day = NoteQuery { event_start: Some(2_000), ..by_event.clone() };
        assert!(notes.notes(&other_day).await.unwrap().is_empty(), "another occurrence");
        let words = NoteQuery { q: Some("train claire".into()), ..Default::default() };
        assert_eq!(notes.notes(&words).await.unwrap().len(), 1);
        let missing = NoteQuery { q: Some("train boiler".into()), ..Default::default() };
        assert!(notes.notes(&missing).await.unwrap().is_empty());

        // Pinned notes come first.
        notes.set_pinned(other.note.id, true).await.unwrap();
        assert_eq!(notes.notes(&NoteQuery::default()).await.unwrap()[0].id, other.note.id);
        let people = notes.people().await.unwrap();
        assert_eq!(people.len(), 2);

        // A to-do becomes a task; ticking the line finds the task again.
        // (A task in Havre's Inbox, written directly: Tasks has its own tests.)
        let task_id = sqlx::query(
            "INSERT INTO tasks (list_id, remote_id, title, updated_at)
             VALUES ((SELECT id FROM task_lists LIMIT 1), 't1', 'Send Claire the planning', 0)",
        )
        .execute(&notes.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        assert_eq!(notes.todo_text(note.note.id, 2).await.unwrap(), "Send Claire the planning");
        assert!(notes.todo_text(note.note.id, 0).await.is_err(), "line 0 is text");
        let view = notes.link_task(note.note.id, "Send Claire the planning", task_id).await.unwrap();
        assert!(view.blocks.iter().any(|b| matches!(b, Block::Todo { task: Some(t), .. } if *t == task_id)));
        let (view, linked) = notes.set_todo(note.note.id, 2, true).await.unwrap();
        assert_eq!(linked, Some(task_id));
        assert!(view.note.body.contains("- [x] Send Claire"));
        let (_, linked) = notes.set_todo(note.note.id, 3, true).await.unwrap();
        assert_eq!(linked, None);
        assert_eq!(notes.notes(&by_person).await.unwrap()[0].open_todos, 0);

        // Deleting a note keeps its tasks.
        notes.delete(note.note.id).await.unwrap();
        assert!(notes.note(note.note.id).await.is_err());
        assert_eq!(db.tasks().tasks(&TaskQuery::default()).await.unwrap().len(), 1);
        assert_eq!(notes.people().await.unwrap().len(), 0, "its people went with it");
    }
}
