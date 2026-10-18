//! People: the address book Havre builds by itself.
//!
//! There is no contact list to fill in. Everyone who wrote to you (Mail)
//! and everyone you meet (Agenda guests and organizers) is a person; their
//! email address is the key. The same list feeds:
//!
//! - the **People** app (a page per person: recent mail, meetings);
//! - the suggestions when you type a recipient or invite a guest.
//!
//! It's all SQL over tables Mail and the Agenda already fill (`messages`,
//! `events`), so it's always up to date and costs nothing to store.

use serde::Serialize;
use sqlx::{Row, SqlitePool};

use crate::{
    Result,
    agenda::{AgendaStore, Event},
    store::escape_like,
};

/// One person, in a list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Person {
    pub email: String,
    /// The name they use (empty if we never saw one).
    pub name: String,
    /// Messages from them, in all your accounts.
    pub messages: i64,
    /// Meetings with them.
    pub meetings: i64,
}

/// A message from someone, wherever it is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MessageRef {
    pub account: i64,
    pub folder: String,
    pub uid: u32,
    pub subject: String,
    pub date: Option<String>,
    pub unread: bool,
    /// Who sent it (for lists mixing several people).
    pub from: Option<String>,
    pub preview: Option<String>,
}

/// A person's page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PersonDetail {
    #[serde(flatten)]
    pub person: Person,
    /// Newest first.
    pub recent_messages: Vec<MessageRef>,
    /// Soonest first.
    pub upcoming: Vec<Event>,
    /// Most recent first.
    pub past: Vec<Event>,
    /// Files they sent (from the messages Havre downloaded), newest first.
    pub files: Vec<SharedFile>,
}

/// An attachment someone sent.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SharedFile {
    pub name: String,
    pub mime: String,
    pub size: usize,
    /// The message it came with, and its number there (to download it).
    pub account: i64,
    pub folder: String,
    pub uid: u32,
    pub index: usize,
    pub date: Option<String>,
}

/// Mail and files from some people (an event's guests: "Related emails").
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Related {
    pub messages: Vec<MessageRef>,
    pub files: Vec<SharedFile>,
}

/// Reads people out of the database.
#[derive(Clone)]
pub struct People {
    pool: SqlitePool,
    agenda: AgendaStore,
}

/// Everyone, once per address (lowercase), with how often we saw them.
/// Your own addresses (your accounts) are left out.
///
/// `json_each` turns the JSON list of guests stored with each event into
/// rows, so SQLite can count them like a normal table.
const PEOPLE: &str = "
    WITH seen AS (
        SELECT lower(from_address) AS email, from_name AS name, 1 AS mail, 0 AS meeting
        FROM messages WHERE from_address IS NOT NULL AND from_address <> ''
        UNION ALL
        SELECT lower(json_extract(a.value, '$.email')), json_extract(a.value, '$.name'), 0, 1
        FROM events e, json_each(e.attendees) a
    )
    SELECT email, COALESCE(MAX(NULLIF(name, '')), '') AS name, SUM(mail) AS messages, SUM(meeting) AS meetings
    FROM seen
    WHERE email IS NOT NULL AND email <> '' AND email NOT IN (SELECT lower(email) FROM accounts)";

impl People {
    pub fn new(pool: SqlitePool, agenda: AgendaStore) -> Self {
        Self { pool, agenda }
    }

    /// People whose name or address contains `query` (everyone if empty),
    /// the ones you deal with most first. A meeting counts like three mails.
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<Person>> {
        let pattern = format!("%{}%", escape_like(&query.trim().to_lowercase()));
        let rows = sqlx::query(&format!(
            "{PEOPLE}
             GROUP BY email
             HAVING email LIKE ?1 ESCAPE '\\' OR lower(name) LIKE ?1 ESCAPE '\\'
             ORDER BY messages + 3 * meetings DESC, email
             LIMIT ?2"
        ))
        .bind(pattern)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(person_from_row).collect())
    }

    /// One person's page. Works for any address, even one never seen
    /// (the page is then empty).
    pub async fn person(&self, email: &str) -> Result<PersonDetail> {
        let email = email.trim().to_lowercase();
        let person = sqlx::query(&format!("{PEOPLE} AND email = ?1 GROUP BY email"))
            .bind(&email)
            .fetch_optional(&self.pool)
            .await?
            .map(|r| person_from_row(&r))
            .unwrap_or(Person { email: email.clone(), name: String::new(), messages: 0, meetings: 0 });

        let Related { messages: recent_messages, files } =
            self.related(std::slice::from_ref(&email), 15).await?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        Ok(PersonDetail {
            person,
            recent_messages,
            upcoming: self.agenda.events_with(&email, Some(now), 10).await?,
            past: self.agenda.events_with(&email, None, 10).await?,
            files,
        })
    }

    /// Recent mail from these addresses (any case), and the files in it.
    pub async fn related(&self, emails: &[String], limit: u32) -> Result<Related> {
        let emails: Vec<String> =
            emails.iter().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect();
        if emails.is_empty() {
            return Ok(Related { messages: Vec::new(), files: Vec::new() });
        }
        let marks = vec!["?"; emails.len()].join(", ");
        // `datetime(date)` reads the RFC 3339 date (with its offset) as UTC,
        // so messages from different time zones sort correctly.
        let sql = format!(
            "SELECT account_id, folder, uid, subject, date, unread, from_name, from_address, preview, files
             FROM messages WHERE lower(from_address) IN ({marks})
             ORDER BY datetime(date) DESC LIMIT ?"
        );
        let mut q = sqlx::query(&sql);
        for e in &emails {
            q = q.bind(e);
        }
        let rows = q.bind(i64::from(limit)).fetch_all(&self.pool).await?;
        let mut messages = Vec::new();
        let mut files = Vec::new();
        for r in &rows {
            let m = MessageRef {
                account: r.get("account_id"),
                folder: r.get("folder"),
                uid: r.get::<i64, _>("uid") as u32,
                subject: r.get("subject"),
                date: r.get("date"),
                unread: r.get("unread"),
                from: r.get::<Option<String>, _>("from_name").or(r.get("from_address")),
                preview: r.get::<Option<String>, _>("preview").filter(|p| !p.is_empty()),
            };
            let attached: Vec<crate::mail::FileInfo> = r
                .get::<Option<String>, _>("files")
                .and_then(|f| serde_json::from_str(&f).ok())
                .unwrap_or_default();
            for f in attached.into_iter().filter(|f| f.mime != "text/calendar") {
                files.push(SharedFile {
                    name: f.name,
                    mime: f.mime,
                    size: f.size,
                    account: m.account,
                    folder: m.folder.clone(),
                    uid: m.uid,
                    index: f.index,
                    date: m.date.clone(),
                });
            }
            messages.push(m);
        }
        files.truncate(8);
        Ok(Related { messages, files })
    }
}

fn person_from_row(r: &sqlx::sqlite::SqliteRow) -> Person {
    Person {
        email: r.get("email"),
        name: r.get("name"),
        messages: r.get("messages"),
        meetings: r.get("meetings"),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        mail::{Folder, FolderRole, MessageSummary, Person as From},
        store::{Database, NewAccount},
    };

    #[tokio::test]
    async fn people_come_from_mail_and_meetings() {
        let db = Database::in_memory().await.unwrap();
        let account = db
            .insert_account(&NewAccount {
                name: "Me".into(),
                email: "me@example.org".into(),
                imap_host: "mail.example.org".into(),
                imap_port: 993,
                imap_user: "me".into(),
                color: "work".into(),
                password: "x".into(),
                sender_name: String::new(),
                smtp_host: None,
                smtp_port: None,
                smtp_security: None,
            })
            .await
            .unwrap();
        let store = db.store(account.id);
        store
            .replace_folders(&[Folder {
                path: "INBOX".into(),
                name: "Inbox".into(),
                depth: 0,
                role: FolderRole::Inbox,
                declared_role: true,
            }])
            .await
            .unwrap();
        let mail = |uid, name: &str, address: &str| MessageSummary {
            uid,
            from: Some(From { name: Some(name.into()), address: Some(address.into()) }),
            subject: format!("Hello {uid}"),
            date: Some(format!("2026-10-0{uid}T09:00:00+02:00")),
            unread: uid == 3,
            ..Default::default()
        };
        store
            .upsert_messages(
                "INBOX",
                &[
                    mail(1, "Anna", "anna@example.org"),
                    mail(2, "Anna Martin", "Anna@Example.org"),
                    mail(3, "Bob", "bob@example.org"),
                    mail(4, "Me", "me@example.org"),
                ],
            )
            .await
            .unwrap();
        // A meeting with Carla (in the local calendar).
        sqlx::query(
            "INSERT INTO events (calendar_id, remote_id, title, start_ts, end_ts, attendees)
             VALUES ((SELECT id FROM calendars LIMIT 1), 'x', 'Review', 4102444800, 4102448400,
                     '[{\"name\":\"Carla\",\"email\":\"carla@example.org\",\"status\":\"accepted\"}]')",
        )
        .execute(&db.pool())
        .await
        .unwrap();

        let people = db.people();
        let all = people.search("", 10).await.unwrap();
        let emails: Vec<&str> = all.iter().map(|p| p.email.as_str()).collect();
        // Carla's meeting counts 3, Anna's two mails 2, Bob 1; "me" is left out.
        assert_eq!(emails, ["carla@example.org", "anna@example.org", "bob@example.org"]);
        assert_eq!(all[1].name, "Anna Martin");

        assert_eq!(people.search("mart", 10).await.unwrap().len(), 1);

        let anna = people.person("ANNA@example.org").await.unwrap();
        assert_eq!(anna.person.messages, 2);
        assert_eq!(anna.recent_messages[0].uid, 2, "newest first");
        let carla = people.person("carla@example.org").await.unwrap();
        assert_eq!(carla.upcoming.len(), 1);
    }
}
