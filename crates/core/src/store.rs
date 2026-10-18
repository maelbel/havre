//! Havre's database: a SQLite file (`havre.db`).
//!
//! - [`Database`] is the whole file: mail accounts (with encrypted
//!   passwords) and, for each account, a local copy of the mailbox.
//! - [`Store`] is the local copy of *one* account's mailbox: folder states,
//!   message headers and the bodies of messages you opened, so the UI never
//!   waits for the mail server. The sync engine (`crate::sync`) keeps it up
//!   to date.
//!
//! We use `sqlx` with plain SQL. Queries are checked at runtime (not at
//! compile time) so building the project never needs a database.

use std::{path::Path, str::FromStr, time::SystemTime};

use serde::{Deserialize, Serialize};
use sqlx::{
    QueryBuilder, Row, Sqlite, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow},
};
use unicode_normalization::UnicodeNormalization;

use crate::{
    Error, Result,
    compose::{Security, SmtpConfig, smtp_defaults},
    config::{ImapAuth, ImapConfig},
    mail::{Folder, FolderRole, ListQuery, MessagePage, MessageSummary, Person},
    oauth::{OAuthApp, OAuthSession, Provider},
    secrets::Secrets,
};

/// A mail account, as shown to the user. The password never leaves the
/// database in clear text (and never goes to the browser).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub email: String,
    /// One of the palette names: work, personal, lyon, hol.
    pub color: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_user: String,
    /// `password`, or the OAuth2 provider: `microsoft`, `google`.
    pub auth: String,
    /// Your name as recipients see it (may be empty).
    pub sender_name: String,
    /// The outgoing server (guessed from the IMAP one unless set).
    pub smtp_host: String,
    pub smtp_port: u16,
    /// `tls` (usually port 465) or `starttls` (usually 587).
    pub smtp_security: String,
    /// `false` for a Microsoft sign-in made before sending existed:
    /// reconnect the account to send.
    pub can_send: bool,
    /// The sign-in allows the Agenda (OAuth2 accounts only; reconnect
    /// once to allow it).
    pub can_calendar: bool,
    /// The sign-in allows Tasks (Microsoft To Do, Google Tasks).
    pub can_tasks: bool,
}

/// What the user types to add an account.
#[derive(Debug, Clone, Deserialize)]
pub struct NewAccount {
    pub name: String,
    pub email: String,
    #[serde(default = "default_color")]
    pub color: String,
    pub imap_host: String,
    #[serde(default = "default_port")]
    pub imap_port: u16,
    /// Defaults to the email address.
    #[serde(default)]
    pub imap_user: String,
    pub password: String,
    /// Your name as recipients see it.
    #[serde(default)]
    pub sender_name: String,
    /// Outgoing server; guessed from the IMAP server when missing.
    #[serde(default)]
    pub smtp_host: Option<String>,
    #[serde(default)]
    pub smtp_port: Option<u16>,
    #[serde(default)]
    pub smtp_security: Option<String>,
}

/// A partial update: only the fields that are present change.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccountUpdate {
    pub name: Option<String>,
    pub email: Option<String>,
    pub color: Option<String>,
    pub imap_host: Option<String>,
    pub imap_port: Option<u16>,
    pub imap_user: Option<String>,
    pub password: Option<String>,
    pub sender_name: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_security: Option<String>,
}

impl AccountUpdate {
    /// Does this change how we connect (so the connection must be tested
    /// and the sync restarted)?
    pub fn touches_connection(&self) -> bool {
        self.imap_host.is_some()
            || self.imap_port.is_some()
            || self.imap_user.is_some()
            || self.password.is_some()
    }
}

fn default_color() -> String {
    "work".into()
}

fn default_port() -> u16 {
    993
}

/// Someone to write to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Contact {
    pub name: Option<String>,
    pub address: String,
}

/// Unread mail across accounts.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UnreadInbox {
    pub total: i64,
    pub messages: Vec<UnreadMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UnreadMessage {
    pub account: i64,
    pub folder: String,
    pub uid: u32,
    pub subject: String,
    pub from: Option<Person>,
    pub date: Option<String>,
}

/// A folder plus its counters, for the sidebar.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FolderInfo {
    #[serde(flatten)]
    pub folder: Folder,
    pub unread: u32,
    pub total: u32,
}

/// What the sync engine remembers about a folder between two syncs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FolderState {
    pub uid_validity: Option<u32>,
    /// Lowest UID we have headers for. `None` = never synced.
    pub oldest_synced_uid: Option<u32>,
    /// Highest UID stored (new mail is anything above it).
    pub newest_uid: Option<u32>,
    pub complete: bool,
}

/// The whole database file.
#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
    secrets: Secrets,
    /// Opened messages are cached up to this many bytes (all accounts).
    cache_limit: u64,
    /// The OAuth2 providers configured in `.env` (Microsoft, Google).
    oauth: Vec<OAuthApp>,
}

impl Database {
    /// Opens (or creates) the database file and applies migrations.
    pub async fn open(path: &Path, secrets: Secrets) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            // WAL: readers don't block the writer — the API reads while sync writes.
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        Self::connect(options, 5, secrets).await
    }

    /// A throwaway database, for tests.
    pub async fn in_memory() -> Result<Self> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        // In-memory databases exist per connection: use exactly one.
        Self::connect(options, 1, Secrets::random()).await
    }

    async fn connect(options: SqliteConnectOptions, max: u32, secrets: Secrets) -> Result<Self> {
        let pool = SqlitePoolOptions::new().max_connections(max).connect_with(options).await?;
        // Applies `migrations/*.sql` that were not applied yet. The files are
        // embedded in the binary at compile time.
        sqlx::migrate!("./migrations").run(&pool).await?;
        let cache_mb =
            std::env::var("HAVRE_CACHE_MB").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(512);
        Ok(Self { pool, secrets, cache_limit: cache_mb * 1024 * 1024, oauth: OAuthApp::all_from_env() })
    }

    /// The login for Havre itself.
    pub fn auth(&self) -> crate::auth::Auth {
        crate::auth::Auth::new(self.pool.clone())
    }

    /// People matching `query` (mail senders and meeting guests), the
    /// ones you deal with most first: the suggestions when typing a
    /// recipient or a guest.
    pub async fn contacts(&self, query: &str, limit: u32) -> Result<Vec<Contact>> {
        let people = self.people().search(query, limit).await?;
        Ok(people
            .into_iter()
            .map(|p| Contact { name: (!p.name.is_empty()).then_some(p.name), address: p.email })
            .collect())
    }

    /// Tasks: lists, to-dos and their links.
    pub fn tasks(&self) -> crate::tasks::TaskStore {
        crate::tasks::TaskStore::new(self.pool.clone())
    }

    /// Files: the attachments of your mail, listed (the Files app).
    pub fn files(&self) -> crate::files::FileStore {
        crate::files::FileStore::new(self.pool.clone())
    }

    /// Notes (Havre's own: nothing to sync).
    pub fn notes(&self) -> crate::notes::NoteStore {
        crate::notes::NoteStore::new(self.pool.clone())
    }

    /// Calendars and events.
    pub fn agenda(&self) -> crate::agenda::AgendaStore {
        crate::agenda::AgendaStore::new(self.pool.clone(), self.secrets.clone())
    }

    /// The database's size on disk, in bytes.
    pub async fn size(&self) -> Result<i64> {
        let pages: i64 = sqlx::query_scalar("PRAGMA page_count").fetch_one(&self.pool).await?;
        let page: i64 = sqlx::query_scalar("PRAGMA page_size").fetch_one(&self.pool).await?;
        Ok(pages * page)
    }

    /// Every account's inbox together, newest first ("All inboxes").
    /// `offset` pages through it; `search` keeps sender/subject matches.
    pub async fn all_inboxes(&self, limit: u32, offset: u32, search: Option<&str>) -> Result<MessagePage> {
        self.across(View::Inboxes, limit, offset, search).await
    }

    /// A list across every account: all inboxes, starred, a label, snoozed.
    pub async fn across(
        &self,
        view: View,
        limit: u32,
        offset: u32,
        search: Option<&str>,
    ) -> Result<MessagePage> {
        let limit = i64::from(limit.max(1));
        let mut sql = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {SUMMARY_COLUMNS}, m.account_id, m.folder FROM messages m \
             JOIN folders f ON f.account_id = m.account_id AND f.path = m.folder \
             LEFT JOIN snoozes s ON s.account_id = m.account_id AND s.folder = m.folder AND s.uid = m.uid \
             WHERE "
        ));
        // What's away (snoozed) shows only in Snoozed; trash and spam only where they are.
        match view {
            View::Inboxes => {
                sql.push("f.role = 'inbox' AND (s.uid IS NULL OR s.back = 1)");
            }
            View::Starred => {
                sql.push(
                    "m.flagged = 1 AND f.role NOT IN ('trash', 'junk') AND (s.uid IS NULL OR s.back = 1)",
                );
            }
            View::Label(id) => {
                sql.push(
                    "f.role NOT IN ('trash', 'junk') AND (s.uid IS NULL OR s.back = 1) AND EXISTS \
                     (SELECT 1 FROM message_labels ml WHERE ml.account_id = m.account_id \
                      AND ml.message_id = m.message_id AND ml.label_id = ",
                )
                .push_bind(id)
                .push(")");
            }
            View::Snoozed => {
                sql.push("s.uid IS NOT NULL AND s.back = 0");
            }
        }
        let words = search.map(|s| normalize(&[Some(s)])).unwrap_or_default();
        for word in words.split_whitespace() {
            sql.push(" AND m.search LIKE ")
                .push_bind(format!("%{}%", escape_like(word)))
                .push(" ESCAPE '\\'");
        }
        // Back from snooze: on top, as if it had just arrived.
        sql.push(match view {
            View::Snoozed => " ORDER BY s.until ASC",
            _ => " ORDER BY MAX(COALESCE(datetime(m.date), ''), CASE WHEN s.back = 1 THEN datetime(s.until, 'unixepoch') ELSE '' END) DESC, m.uid DESC",
        });
        sql.push(" LIMIT ").push_bind(limit + 1).push(" OFFSET ").push_bind(i64::from(offset));
        let rows = sql.build().fetch_all(&self.pool).await?;
        let has_more = rows.len() as i64 > limit;
        Ok(MessagePage {
            messages: rows.iter().take(limit as usize).map(summary_from_row).collect(),
            has_more,
        })
    }

    /// Unread messages in every account's inbox, newest first, and how
    /// many there are in all (for the Today page).
    pub async fn unread_inbox(&self, limit: u32) -> Result<UnreadInbox> {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages m JOIN folders f ON f.account_id = m.account_id AND f.path = m.folder
             WHERE f.role = 'inbox' AND m.unread = 1 AND NOT EXISTS (SELECT 1 FROM snoozes s
                 WHERE s.account_id = m.account_id AND s.folder = m.folder AND s.uid = m.uid AND s.back = 0)",
        )
        .fetch_one(&self.pool)
        .await?;
        let rows = sqlx::query(
            "SELECT m.account_id, m.folder, m.uid, m.subject, m.from_name, m.from_address, m.date
             FROM messages m JOIN folders f ON f.account_id = m.account_id AND f.path = m.folder
             WHERE f.role = 'inbox' AND m.unread = 1 AND NOT EXISTS (SELECT 1 FROM snoozes s
                 WHERE s.account_id = m.account_id AND s.folder = m.folder AND s.uid = m.uid AND s.back = 0)
             ORDER BY datetime(m.date) DESC LIMIT ?",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        let messages = rows
            .iter()
            .map(|r| UnreadMessage {
                account: r.get("account_id"),
                folder: r.get("folder"),
                uid: r.get::<i64, _>("uid") as u32,
                subject: r.get("subject"),
                from: match (
                    r.get::<Option<String>, _>("from_name"),
                    r.get::<Option<String>, _>("from_address"),
                ) {
                    (None, None) => None,
                    (name, address) => Some(Person { name, address }),
                },
                date: r.get("date"),
            })
            .collect();
        Ok(UnreadInbox { total, messages })
    }

    /// Messages whose sender or subject contains any of the words, in all
    /// accounts and folders, newest first (the palette, the assistant).
    pub async fn search_all(&self, query: &str, limit: i64) -> Result<Vec<crate::ai::index::Hit>> {
        let words: Vec<String> = normalize(&[Some(query)])
            .split_whitespace()
            .filter(|w| w.chars().count() > 1)
            .map(|w| format!("%{}%", escape_like(w)))
            .collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let mut sql = QueryBuilder::<Sqlite>::new(
            "SELECT account_id, folder, uid, subject, from_name, from_address, date FROM messages WHERE ",
        );
        // Every word when possible: count matching words, best first.
        sql.push("(");
        for (i, w) in words.iter().enumerate() {
            if i > 0 {
                sql.push(" OR ");
            }
            sql.push("search LIKE ").push_bind(w.clone()).push(" ESCAPE '\\'");
        }
        sql.push(") ORDER BY (");
        for (i, w) in words.iter().enumerate() {
            if i > 0 {
                sql.push(" + ");
            }
            sql.push("(search LIKE ").push_bind(w.clone()).push(" ESCAPE '\\')");
        }
        sql.push(") DESC, datetime(date) DESC LIMIT ").push_bind(limit);
        let rows = sql.build().fetch_all(&self.pool).await?;
        Ok(rows
            .iter()
            .map(|r| crate::ai::index::Hit {
                account: r.get("account_id"),
                folder: r.get("folder"),
                uid: r.get::<i64, _>("uid") as u32,
                subject: r.get("subject"),
                from: match (
                    r.get::<Option<String>, _>("from_name"),
                    r.get::<Option<String>, _>("from_address"),
                ) {
                    (None, None) => None,
                    (name, address) => Some(Person { name, address }),
                },
                date: r.get("date"),
                score: 0.0,
            })
            .collect())
    }

    /// Everyone you write to or meet (built from mail and events).
    pub fn people(&self) -> crate::people::People {
        crate::people::People::new(self.pool.clone(), self.agenda())
    }

    /// Which apps are in the dock, in which order.
    pub fn apps(&self) -> crate::apps::AppSettings {
        crate::apps::AppSettings::new(self.pool.clone())
    }

    // ---- Labels ---------------------------------------------------------------

    /// Every label, in order, with how many messages carry it.
    pub async fn labels(&self) -> Result<Vec<Label>> {
        Ok(sqlx::query_as::<_, Label>(
            "SELECT l.id, l.name, l.color, l.position,
                    (SELECT COUNT(*) FROM message_labels ml WHERE ml.label_id = l.id) AS count
             FROM labels l ORDER BY l.position, l.name COLLATE NOCASE",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_label(&self, name: &str, color: &str) -> Result<Label> {
        let name = label_name(name)?;
        let position: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(position), 0) + 1 FROM labels")
            .fetch_one(&self.pool)
            .await?;
        let id = sqlx::query("INSERT INTO labels (name, color, position) VALUES (?, ?, ?)")
            .bind(&name)
            .bind(label_color(color))
            .bind(position)
            .execute(&self.pool)
            .await
            .map_err(|e| match e {
                sqlx::Error::Database(d) if d.is_unique_violation() => {
                    Error::Config(format!("There's already a label “{name}”."))
                }
                e => e.into(),
            })?
            .last_insert_rowid();
        self.label(id).await
    }

    pub async fn update_label(&self, id: i64, name: Option<&str>, color: Option<&str>) -> Result<Label> {
        if let Some(name) = name {
            let name = label_name(name)?;
            sqlx::query("UPDATE labels SET name = ? WHERE id = ?")
                .bind(&name)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|e| match e {
                    sqlx::Error::Database(d) if d.is_unique_violation() => {
                        Error::Config(format!("There's already a label “{name}”."))
                    }
                    e => e.into(),
                })?;
        }
        if let Some(color) = color {
            sqlx::query("UPDATE labels SET color = ? WHERE id = ?")
                .bind(label_color(color))
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        self.label(id).await
    }

    /// Deletes a label (the messages stay, without it).
    pub async fn delete_label(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM labels WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    async fn label(&self, id: i64) -> Result<Label> {
        self.labels()
            .await?
            .into_iter()
            .find(|l| l.id == id)
            .ok_or_else(|| Error::NotFound(format!("label {id}")))
    }

    // ---- Snooze -------------------------------------------------------------

    /// Snoozed messages whose time has come: `(account, folder, uid)`.
    pub async fn due_snoozes(&self, now: i64) -> Result<Vec<(i64, String, u32)>> {
        let rows: Vec<(i64, String, i64)> =
            sqlx::query_as("SELECT account_id, folder, uid FROM snoozes WHERE back = 0 AND until <= ?")
                .bind(now)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(a, f, u)| (a, f, u as u32)).collect())
    }

    /// How many messages are snoozed, starred (for the sidebar).
    pub async fn organize_counts(&self) -> Result<OrganizeCounts> {
        let snoozed: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM snoozes WHERE back = 0").fetch_one(&self.pool).await?;
        let starred: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages m JOIN folders f ON f.account_id = m.account_id AND f.path = m.folder
             WHERE m.flagged = 1 AND f.role NOT IN ('trash', 'junk')",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(OrganizeCounts { snoozed, starred })
    }

    pub(crate) fn pool(&self) -> SqlitePool {
        self.pool.clone()
    }

    pub(crate) fn secrets(&self) -> &Secrets {
        &self.secrets
    }

    /// Messages being written.
    pub fn drafts(&self) -> crate::compose::Drafts {
        crate::compose::Drafts::new(self.pool.clone())
    }

    /// The local mailbox copy of one account.
    pub fn store(&self, account_id: i64) -> Store {
        Store { pool: self.pool.clone(), account: account_id, cache_limit: self.cache_limit }
    }

    // ---- Accounts -----------------------------------------------------------

    pub async fn accounts(&self) -> Result<Vec<Account>> {
        let rows = sqlx::query(&format!("SELECT {ACCOUNT_COLUMNS} FROM accounts ORDER BY position, id"))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(|r| self.account_from_row(r)).collect())
    }

    pub async fn account(&self, id: i64) -> Result<Account> {
        let row = sqlx::query(&format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("account {id}")))?;
        Ok(self.account_from_row(&row))
    }

    fn account_from_row(&self, r: &SqliteRow) -> Account {
        let imap_host: String = r.get("imap_host");
        let (default_host, default_port, default_security) = smtp_defaults(&imap_host);
        let auth: String = r.get("auth");
        let scope = r.get::<Option<&str>, _>("oauth_scope");
        let app = Provider::parse(&auth).and_then(|p| self.oauth_app(p));
        let can_send = match Provider::parse(&auth) {
            None => true,
            Some(_) => app.is_some_and(|app| app.can_send(scope)),
        };
        let can_calendar = app.is_some_and(|app| app.can_calendar(scope));
        let can_tasks = app.is_some_and(|app| app.can_tasks(scope));
        Account {
            id: r.get("id"),
            name: r.get("name"),
            email: r.get("email"),
            color: r.get("color"),
            imap_port: r.get::<i64, _>("imap_port") as u16,
            imap_user: r.get("imap_user"),
            sender_name: r.get("sender_name"),
            smtp_host: r.get::<Option<String>, _>("smtp_host").unwrap_or(default_host),
            smtp_port: r.get::<Option<i64>, _>("smtp_port").map_or(default_port, |p| p as u16),
            smtp_security: r
                .get::<Option<String>, _>("smtp_security")
                .unwrap_or_else(|| default_security.as_str().to_owned()),
            imap_host,
            auth,
            can_send,
            can_calendar,
            can_tasks,
        }
    }

    /// How to reach the account's outgoing server (same login as IMAP).
    pub async fn smtp_config(&self, id: i64) -> Result<SmtpConfig> {
        let account = self.account(id).await?;
        if !account.can_send {
            return Err(Error::SignIn(
                "This account was connected before Havre could send mail: reconnect it in Settings \
                 (Microsoft will ask to allow sending)."
                    .into(),
            ));
        }
        let imap = self.imap_config(id).await?;
        Ok(SmtpConfig {
            host: account.smtp_host,
            port: account.smtp_port,
            security: Security::parse(&account.smtp_security),
            username: imap.username,
            auth: imap.auth,
            ca_file: imap.ca_file,
        })
    }

    pub async fn insert_account(&self, new: &NewAccount) -> Result<Account> {
        let user = if new.imap_user.trim().is_empty() { new.email.trim() } else { new.imap_user.trim() };
        let password = self.secrets.encrypt(&new.password)?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO accounts (name, email, color, imap_host, imap_port, imap_user, imap_password,
                                   smtp_host, smtp_port, smtp_security, sender_name, position, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, (SELECT COALESCE(MAX(position), 0) + 1 FROM accounts), ?)
             RETURNING id",
        )
        .bind(new.name.trim())
        .bind(new.email.trim())
        .bind(&new.color)
        .bind(new.imap_host.trim())
        .bind(new.imap_port as i64)
        .bind(user)
        .bind(password)
        .bind(clean_host(&new.smtp_host))
        .bind(new.smtp_port.map(i64::from))
        .bind(clean_security(&new.smtp_security)?)
        .bind(new.sender_name.trim())
        .bind(now())
        .fetch_one(&self.pool)
        .await?;
        self.account(id).await
    }

    pub async fn update_account(&self, id: i64, update: &AccountUpdate) -> Result<Account> {
        let mut sql = QueryBuilder::<Sqlite>::new("UPDATE accounts SET id = id");
        if let Some(v) = &update.name {
            sql.push(", name = ").push_bind(v.trim().to_owned());
        }
        if let Some(v) = &update.email {
            sql.push(", email = ").push_bind(v.trim().to_owned());
        }
        if let Some(v) = &update.color {
            sql.push(", color = ").push_bind(v.clone());
        }
        if let Some(v) = &update.imap_host {
            sql.push(", imap_host = ").push_bind(v.trim().to_owned());
        }
        if let Some(v) = update.imap_port {
            sql.push(", imap_port = ").push_bind(v as i64);
        }
        if let Some(v) = &update.imap_user {
            sql.push(", imap_user = ").push_bind(v.trim().to_owned());
        }
        if let Some(v) = &update.password {
            sql.push(", imap_password = ").push_bind(self.secrets.encrypt(v)?);
        }
        if let Some(v) = &update.sender_name {
            sql.push(", sender_name = ").push_bind(v.trim().to_owned());
        }
        if update.smtp_host.is_some() {
            sql.push(", smtp_host = ").push_bind(clean_host(&update.smtp_host));
        }
        if let Some(v) = update.smtp_port {
            sql.push(", smtp_port = ").push_bind(v as i64);
        }
        if update.smtp_security.is_some() {
            sql.push(", smtp_security = ").push_bind(clean_security(&update.smtp_security)?);
        }
        sql.push(" WHERE id = ").push_bind(id);
        if sql.build().execute(&self.pool).await?.rows_affected() == 0 {
            return Err(Error::NotFound(format!("account {id}")));
        }
        self.account(id).await
    }

    /// Deletes the account and, through `ON DELETE CASCADE`, its whole cache.
    pub async fn delete_account(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM accounts WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// How to connect to the account's server (password decrypted, or an
    /// OAuth session that fetches access tokens).
    pub async fn imap_config(&self, id: i64) -> Result<ImapConfig> {
        let row = sqlx::query(
            "SELECT imap_host, imap_port, imap_user, imap_password, auth FROM accounts WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("account {id}")))?;
        let auth = match Provider::parse(row.get::<&str, _>("auth")) {
            Some(provider) => {
                let app = self.oauth_app(provider).cloned().ok_or_else(|| {
                    Error::Config(format!(
                        "OAuth2 with {} isn't set up (see .env): this account can't sign in",
                        provider.name()
                    ))
                })?;
                ImapAuth::OAuth(std::sync::Arc::new(OAuthSession::new(app, id, self.clone())))
            }
            None => ImapAuth::Password(self.secrets.decrypt(row.get::<&[u8], _>("imap_password"))?),
        };
        Ok(ImapConfig {
            host: row.get("imap_host"),
            port: row.get::<i64, _>("imap_port") as u16,
            username: row.get("imap_user"),
            auth,
            ca_file: std::env::var("HAVRE_IMAP_CA_FILE").ok().filter(|v| !v.trim().is_empty()),
        })
    }

    /// How Havre is registered with an OAuth2 provider, if it is (`.env`).
    pub fn oauth_app(&self, provider: Provider) -> Option<&OAuthApp> {
        self.oauth.iter().find(|app| app.provider == provider)
    }

    /// Saves an account that signs in with OAuth2: no password, an
    /// encrypted refresh token instead.
    pub async fn insert_oauth_account(
        &self,
        new: &NewAccount,
        provider: Provider,
        refresh: &str,
        scope: &str,
    ) -> Result<Account> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO accounts (name, email, color, imap_host, imap_port, imap_user, imap_password,
                                   auth, oauth_refresh, oauth_scope, sender_name, position, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, (SELECT COALESCE(MAX(position), 0) + 1 FROM accounts), ?)
             RETURNING id",
        )
        .bind(new.name.trim())
        .bind(new.email.trim())
        .bind(&new.color)
        .bind(new.imap_host.trim())
        .bind(new.imap_port as i64)
        .bind(new.imap_user.trim())
        .bind(self.secrets.encrypt("")?)
        .bind(provider.as_str())
        .bind(self.secrets.encrypt(refresh)?)
        .bind(scope)
        .bind(new.sender_name.trim())
        .bind(now())
        .fetch_one(&self.pool)
        .await?;
        self.account(id).await
    }

    pub async fn refresh_token(&self, id: i64) -> Result<String> {
        let sealed: Option<Vec<u8>> = sqlx::query_scalar("SELECT oauth_refresh FROM accounts WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .flatten();
        let sealed = sealed.ok_or_else(|| Error::SignIn("not signed in with OAuth2".into()))?;
        self.secrets.decrypt(&sealed)
    }

    /// What the account's OAuth2 sign-in allows (`None`: an older sign-in).
    pub async fn oauth_scope(&self, id: i64) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT oauth_scope FROM accounts WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .flatten())
    }

    /// After signing in again: the new refresh token and what it allows.
    pub async fn set_oauth_sign_in(&self, id: i64, refresh: &str, scope: &str) -> Result<()> {
        sqlx::query("UPDATE accounts SET oauth_refresh = ?, oauth_scope = ? WHERE id = ?")
            .bind(self.secrets.encrypt(refresh)?)
            .bind(scope)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_refresh_token(&self, id: i64, token: &str) -> Result<()> {
        sqlx::query("UPDATE accounts SET oauth_refresh = ? WHERE id = ?")
            .bind(self.secrets.encrypt(token)?)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// The columns `account_from_row` reads.
const ACCOUNT_COLUMNS: &str = "id, name, email, color, imap_host, imap_port, imap_user, auth, \
                               smtp_host, smtp_port, smtp_security, oauth_scope, sender_name";

fn clean_host(host: &Option<String>) -> Option<String> {
    host.as_deref().map(str::trim).filter(|h| !h.is_empty()).map(str::to_owned)
}

fn clean_security(security: &Option<String>) -> Result<Option<String>> {
    match security.as_deref() {
        None | Some("") => Ok(None),
        Some(v @ ("tls" | "starttls")) => Ok(Some(v.to_owned())),
        Some(other) => Err(Error::Config(format!("unknown SMTP security “{other}”: use tls or starttls"))),
    }
}

/// The local copy of one account's mailbox. Every query is limited to
/// `account`, so two accounts can never see each other's mail.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
    account: i64,
    cache_limit: u64,
}

impl Store {
    pub fn account_id(&self) -> i64 {
        self.account
    }

    // ---- Folders ----------------------------------------------------------

    /// Makes the stored folder list match the server's (sorted) list.
    /// Folders that disappeared are deleted with their messages.
    pub async fn replace_folders(&self, folders: &[Folder]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for (position, f) in folders.iter().enumerate() {
            sqlx::query(
                "INSERT INTO folders (account_id, path, name, depth, role, declared_role, position)
                 VALUES (?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (account_id, path) DO UPDATE SET
                   name = excluded.name, depth = excluded.depth, role = excluded.role,
                   declared_role = excluded.declared_role, position = excluded.position",
            )
            .bind(self.account)
            .bind(&f.path)
            .bind(&f.name)
            .bind(f.depth as i64)
            .bind(role_str(f.role))
            .bind(f.declared_role)
            .bind(position as i64)
            .execute(&mut *tx)
            .await?;
        }
        let mut query = QueryBuilder::<Sqlite>::new("DELETE FROM folders WHERE account_id = ");
        query.push_bind(self.account);
        if !folders.is_empty() {
            query.push(" AND path NOT IN (");
            let mut list = query.separated(", ");
            for f in folders {
                list.push_bind(&f.path);
            }
            list.push_unseparated(")");
        }
        query.build().execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn folders(&self) -> Result<Vec<FolderInfo>> {
        // `unread` is the server's count, less the unread messages snoozed
        // away: they're out of the list until their time, so the counter
        // (the sidebar, the dock's badge) leaves them out too — like Today.
        let rows = sqlx::query(
            "SELECT path, name, depth, role, declared_role, total,
                    MAX(0, unread - (SELECT COUNT(*) FROM messages m JOIN snoozes s
                        ON s.account_id = m.account_id AND s.folder = m.folder AND s.uid = m.uid
                        WHERE m.account_id = folders.account_id AND m.folder = folders.path
                          AND m.unread = 1 AND s.back = 0)) AS unread
             FROM folders WHERE account_id = ? ORDER BY position",
        )
        .bind(self.account)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| FolderInfo {
                folder: Folder {
                    path: r.get("path"),
                    name: r.get("name"),
                    depth: r.get::<i64, _>("depth") as usize,
                    role: role_from_str(r.get("role")),
                    declared_role: r.get("declared_role"),
                },
                unread: r.get::<i64, _>("unread") as u32,
                total: r.get::<i64, _>("total") as u32,
            })
            .collect())
    }

    pub async fn folder_state(&self, path: &str) -> Result<FolderState> {
        let row = sqlx::query(
            "SELECT uid_validity, oldest_synced_uid, complete,
                    (SELECT MAX(uid) FROM messages m
                     WHERE m.account_id = folders.account_id AND m.folder = folders.path) AS newest
             FROM folders WHERE account_id = ? AND path = ?",
        )
        .bind(self.account)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row
            .map(|r| FolderState {
                uid_validity: r.get::<Option<i64>, _>("uid_validity").map(|v| v as u32),
                oldest_synced_uid: r.get::<Option<i64>, _>("oldest_synced_uid").map(|v| v as u32),
                newest_uid: r.get::<Option<i64>, _>("newest").map(|v| v as u32),
                complete: r.get("complete"),
            })
            .unwrap_or_default())
    }

    /// The server changed the folder's UIDVALIDITY: every UID we know is
    /// meaningless now. Forget the folder's messages and start over.
    pub async fn reset_folder(&self, path: &str, uid_validity: u32) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM messages WHERE account_id = ? AND folder = ?")
            .bind(self.account)
            .bind(path)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE folders SET uid_validity = ?, oldest_synced_uid = NULL, complete = 0
             WHERE account_id = ? AND path = ?",
        )
        .bind(uid_validity as i64)
        .bind(self.account)
        .bind(path)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Counters from IMAP `STATUS`, exact even before every header is synced.
    pub async fn set_counts(&self, path: &str, total: u32, unread: u32) -> Result<()> {
        sqlx::query(
            "UPDATE folders SET total = ?, unread = ?, synced_at = ? WHERE account_id = ? AND path = ?",
        )
        .bind(total as i64)
        .bind(unread as i64)
        .bind(now())
        .bind(self.account)
        .bind(path)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The folder's STATUS "fingerprint" from the last successful sync.
    pub async fn fingerprint(&self, path: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT fingerprint FROM folders WHERE account_id = ? AND path = ?")
            .bind(self.account)
            .bind(path)
            .fetch_optional(&self.pool)
            .await?
            .flatten())
    }

    pub async fn set_fingerprint(&self, path: &str, fingerprint: &str) -> Result<()> {
        sqlx::query("UPDATE folders SET fingerprint = ? WHERE account_id = ? AND path = ?")
            .bind(fingerprint)
            .bind(self.account)
            .bind(path)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Records how far back headers are stored.
    pub async fn set_backfill(
        &self,
        path: &str,
        oldest_synced_uid: Option<u32>,
        complete: bool,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE folders SET oldest_synced_uid = ?, complete = ? WHERE account_id = ? AND path = ?",
        )
        .bind(oldest_synced_uid.map(|u| u as i64))
        .bind(complete)
        .bind(self.account)
        .bind(path)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ---- Messages ---------------------------------------------------------

    /// Inserts or updates message headers. Returns how many rows changed.
    pub async fn upsert_messages(&self, folder: &str, messages: &[MessageSummary]) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        let mut changed = 0;
        for m in messages {
            let (name, address) =
                m.from.as_ref().map(|p| (p.name.clone(), p.address.clone())).unwrap_or_default();
            let search = normalize(&[Some(m.subject.as_str()), name.as_deref(), address.as_deref()]);
            changed += sqlx::query(
                "INSERT INTO messages (account_id, folder, uid, subject, from_name, from_address, date, unread, search,
                                       flagged, message_id)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (account_id, folder, uid) DO UPDATE SET unread = excluded.unread,
                     flagged = excluded.flagged, message_id = COALESCE(excluded.message_id, message_id)
                 WHERE unread != excluded.unread OR flagged != excluded.flagged
                     OR message_id IS NOT excluded.message_id",
            )
            .bind(self.account)
            .bind(folder)
            .bind(m.uid as i64)
            .bind(&m.subject)
            .bind(name)
            .bind(address)
            .bind(&m.date)
            .bind(m.unread)
            .bind(search)
            .bind(m.flagged)
            .bind(m.message_id.as_deref().map(clean_message_id))
            .execute(&mut *tx)
            .await?
            .rows_affected();
        }
        tx.commit().await?;
        Ok(changed)
    }

    /// Brings stored messages with UIDs in `low..=high` in line with the
    /// server: `present` lists every message the server still has in that
    /// range, with its flags. Missing ones were deleted or moved elsewhere.
    /// Returns how many rows changed.
    pub async fn reconcile(
        &self,
        folder: &str,
        low: u32,
        high: u32,
        present: &[(u32, Flags)],
    ) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        let stored: Vec<(i64, bool, bool)> = sqlx::query_as(
            "SELECT uid, unread, flagged FROM messages WHERE account_id = ? AND folder = ? AND uid BETWEEN ? AND ?",
        )
        .bind(self.account)
        .bind(folder)
        .bind(low as i64)
        .bind(high as i64)
        .fetch_all(&mut *tx)
        .await?;
        let server: std::collections::HashMap<u32, Flags> = present.iter().copied().collect();
        let mut changed = 0;
        for (uid, unread, flagged) in stored {
            let uid = uid as u32;
            match server.get(&uid) {
                None => {
                    changed +=
                        sqlx::query("DELETE FROM messages WHERE account_id = ? AND folder = ? AND uid = ?")
                            .bind(self.account)
                            .bind(folder)
                            .bind(uid as i64)
                            .execute(&mut *tx)
                            .await?
                            .rows_affected();
                }
                Some(now) if now.unread != unread || now.flagged != flagged => {
                    changed += sqlx::query(
                        "UPDATE messages SET unread = ?, flagged = ? WHERE account_id = ? AND folder = ? AND uid = ?",
                    )
                    .bind(now.unread)
                    .bind(now.flagged)
                    .bind(self.account)
                    .bind(folder)
                    .bind(uid as i64)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
                }
                Some(_) => {}
            }
        }
        tx.commit().await?;
        Ok(changed)
    }

    /// One page of a folder, newest first, optionally filtered by `search`.
    pub async fn page(&self, folder: &str, query: &ListQuery) -> Result<MessagePage> {
        let limit = query.limit.max(1) as i64;
        let mut sql = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {SUMMARY_COLUMNS} FROM messages m WHERE m.account_id = "
        ));
        sql.push_bind(self.account).push(" AND m.folder = ").push_bind(folder);
        // Snoozed messages are away until their time.
        sql.push(" AND NOT EXISTS (SELECT 1 FROM snoozes s WHERE s.account_id = m.account_id AND s.folder = m.folder AND s.uid = m.uid AND s.back = 0)");
        if let Some(before) = query.before_uid {
            sql.push(" AND m.uid < ").push_bind(before as i64);
        }
        // Every word must appear somewhere in sender or subject.
        let words = query.search.as_deref().map(|s| normalize(&[Some(s)])).unwrap_or_default();
        for word in words.split_whitespace() {
            sql.push(" AND m.search LIKE ")
                .push_bind(format!("%{}%", escape_like(word)))
                .push(" ESCAPE '\\'");
        }
        // One extra row tells us whether there is a next page.
        sql.push(" ORDER BY m.uid DESC LIMIT ").push_bind(limit + 1);

        let rows = sql.build().fetch_all(&self.pool).await?;
        let has_more_rows = rows.len() as i64 > limit;
        let messages: Vec<MessageSummary> =
            rows.into_iter().take(limit as usize).map(|r| summary_from_row(&r)).collect();

        // Older headers may still be on the server only (sync in progress).
        let complete = self.folder_state(folder).await?.complete;
        let has_more = has_more_rows || (!complete && query.search.is_none());
        Ok(MessagePage { messages, has_more })
    }

    /// Whether a stored message is unread (`None` if not stored).
    pub async fn is_unread(&self, folder: &str, uid: u32) -> Result<Option<bool>> {
        Ok(sqlx::query_scalar("SELECT unread FROM messages WHERE account_id = ? AND folder = ? AND uid = ?")
            .bind(self.account)
            .bind(folder)
            .bind(uid as i64)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Marks one message read/unread and keeps the folder counter right.
    pub async fn set_unread(&self, folder: &str, uid: u32, unread: bool) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE messages SET unread = ? WHERE account_id = ? AND folder = ? AND uid = ? AND unread != ?",
        )
        .bind(unread)
        .bind(self.account)
        .bind(folder)
        .bind(uid as i64)
        .bind(unread)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if changed > 0 {
            let delta = if unread { 1 } else { -1 };
            sqlx::query("UPDATE folders SET unread = MAX(0, unread + ?) WHERE account_id = ? AND path = ?")
                .bind(delta)
                .bind(self.account)
                .bind(folder)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Stars or unstars messages here (after the server did).
    pub async fn set_flagged(&self, folder: &str, uids: &[u32], flagged: bool) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for uid in uids {
            sqlx::query("UPDATE messages SET flagged = ? WHERE account_id = ? AND folder = ? AND uid = ?")
                .bind(flagged)
                .bind(self.account)
                .bind(folder)
                .bind(*uid as i64)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The `Message-ID`s of these messages (those that have one).
    pub async fn message_ids(&self, folder: &str, uids: &[u32]) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        for uid in uids {
            let id: Option<Option<String>> = sqlx::query_scalar(
                "SELECT message_id FROM messages WHERE account_id = ? AND folder = ? AND uid = ?",
            )
            .bind(self.account)
            .bind(folder)
            .bind(*uid as i64)
            .fetch_optional(&self.pool)
            .await?;
            if let Some(Some(id)) = id
                && !id.is_empty()
            {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// What a task made from this message remembers about it: subject,
    /// sender (name, address) and `Message-ID`.
    pub async fn mail_link(&self, folder: &str, uid: u32) -> Result<MailFacts> {
        let found: Option<MailFacts> = sqlx::query_as(
            "SELECT subject, from_name, from_address, message_id FROM messages
             WHERE account_id = ? AND folder = ? AND uid = ?",
        )
        .bind(self.account)
        .bind(folder)
        .bind(uid as i64)
        .fetch_optional(&self.pool)
        .await?;
        let (subject, name, address, id) =
            found.ok_or_else(|| Error::NotFound("This message is no longer here.".into()))?;
        // Some messages have an empty Message-ID: as good as none.
        Ok((subject, name, address, id.filter(|id| !id.trim().is_empty())))
    }

    /// Where a message is now (folder, UID), by its `Message-ID` — it may
    /// have moved since. Inbox first, then the most recent copy.
    pub async fn locate(&self, message_id: &str) -> Result<Option<(String, u32)>> {
        let message_id = clean_message_id(message_id);
        if message_id.is_empty() {
            return Ok(None);
        }
        let found: Option<(String, i64)> = sqlx::query_as(
            "SELECT m.folder, m.uid FROM messages m JOIN folders f ON f.account_id = m.account_id AND f.path = m.folder
             WHERE m.account_id = ? AND m.message_id = ?
             ORDER BY f.role = 'inbox' DESC, f.role IN ('trash', 'junk'), m.uid DESC LIMIT 1",
        )
        .bind(self.account)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(found.map(|(folder, uid)| (folder, uid as u32)))
    }

    /// Messages synced before Havre kept `Message-ID`s: up to `limit` UIDs
    /// of this folder still without one (the sync reads them).
    pub async fn missing_message_ids(&self, folder: &str, limit: i64) -> Result<Vec<u32>> {
        let uids: Vec<i64> = sqlx::query_scalar(
            "SELECT uid FROM messages WHERE account_id = ? AND folder = ? AND message_id IS NULL
             ORDER BY uid DESC LIMIT ?",
        )
        .bind(self.account)
        .bind(folder)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(uids.into_iter().map(|u| u as u32).collect())
    }

    pub async fn set_message_id(&self, folder: &str, uid: u32, id: &str) -> Result<()> {
        sqlx::query("UPDATE messages SET message_id = ? WHERE account_id = ? AND folder = ? AND uid = ?")
            .bind(clean_message_id(id))
            .bind(self.account)
            .bind(folder)
            .bind(uid as i64)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Adds (`on`) or removes a label on messages. Messages without a
    /// `Message-ID` can't carry one: they are counted in the error.
    pub async fn set_label(&self, folder: &str, uids: &[u32], label: i64, on: bool) -> Result<()> {
        let ids = self.message_ids(folder, uids).await?;
        if ids.is_empty() {
            return Err(Error::Config(
                "This message can't be labelled: it has no Message-ID (or Havre hasn't read it yet).".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        for id in &ids {
            let sql = if on {
                "INSERT OR IGNORE INTO message_labels (label_id, account_id, message_id) VALUES (?, ?, ?)"
            } else {
                "DELETE FROM message_labels WHERE label_id = ? AND account_id = ? AND message_id = ?"
            };
            sqlx::query(sql).bind(label).bind(self.account).bind(id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Snoozes messages until `until` (unix seconds); `None` wakes them now.
    pub async fn snooze(&self, folder: &str, uids: &[u32], until: Option<i64>) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for uid in uids {
            match until {
                Some(until) => {
                    sqlx::query(
                        "INSERT INTO snoozes (account_id, folder, uid, until, back) VALUES (?, ?, ?, ?, 0)
                         ON CONFLICT (account_id, folder, uid) DO UPDATE SET until = excluded.until, back = 0",
                    )
                    .bind(self.account)
                    .bind(folder)
                    .bind(*uid as i64)
                    .bind(until)
                    .execute(&mut *tx)
                    .await?;
                }
                None => {
                    sqlx::query("DELETE FROM snoozes WHERE account_id = ? AND folder = ? AND uid = ?")
                        .bind(self.account)
                        .bind(folder)
                        .bind(*uid as i64)
                        .execute(&mut *tx)
                        .await?;
                }
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// The snooze is over: it shows again, on top of All inboxes.
    pub async fn mark_back(&self, folder: &str, uid: u32) -> Result<()> {
        sqlx::query("UPDATE snoozes SET back = 1 WHERE account_id = ? AND folder = ? AND uid = ?")
            .bind(self.account)
            .bind(folder)
            .bind(uid as i64)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Removes messages (moved or deleted) and keeps counters right.
    pub async fn remove_messages(&self, folder: &str, uids: &[u32]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for uid in uids {
            let removed = sqlx::query(
                "DELETE FROM messages WHERE account_id = ? AND folder = ? AND uid = ? RETURNING unread",
            )
            .bind(self.account)
            .bind(folder)
            .bind(*uid as i64)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(row) = removed {
                let unread: bool = row.get("unread");
                sqlx::query(
                    "UPDATE folders SET total = MAX(0, total - 1), unread = MAX(0, unread - ?)
                     WHERE account_id = ? AND path = ?",
                )
                .bind(unread as i64)
                .bind(self.account)
                .bind(folder)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    // ---- Bodies -----------------------------------------------------------

    pub async fn body(&self, folder: &str, uid: u32) -> Result<Option<Vec<u8>>> {
        Ok(sqlx::query_scalar("SELECT raw FROM bodies WHERE account_id = ? AND folder = ? AND uid = ?")
            .bind(self.account)
            .bind(folder)
            .bind(uid as i64)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Caches a raw message, then evicts the oldest cached bodies if the
    /// cache grew beyond its limit (`HAVRE_CACHE_MB`, 512 MB by default).
    /// Ignored if the message's headers are not stored (it is then simply
    /// not cached).
    /// Messages of this folder whose body isn't downloaded yet, newest
    /// first (the background download of recent mail).
    pub async fn missing_bodies(&self, folder: &str, limit: i64) -> Result<Vec<u32>> {
        let uids: Vec<i64> = sqlx::query_scalar(
            "SELECT m.uid FROM messages m
             LEFT JOIN bodies b ON b.account_id = m.account_id AND b.folder = m.folder AND b.uid = m.uid
             WHERE m.account_id = ? AND m.folder = ? AND b.uid IS NULL AND m.files IS NULL
             ORDER BY m.uid DESC LIMIT ?",
        )
        .bind(self.account)
        .bind(folder)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(uids.into_iter().map(|u| u as u32).collect())
    }

    /// The Files app looks through older mail: `None` when it isn't in the
    /// dock, else how many of the newest emails per folder (`i64::MAX`: all).
    pub(crate) async fn files_depth(&self) -> Result<Option<i64>> {
        let row: Option<i64> = sqlx::query_scalar(
            "SELECT s.depth FROM files_settings s
             WHERE s.id = 1 AND EXISTS (SELECT 1 FROM app_settings WHERE id = 'files' AND enabled = 1)",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|depth| if depth == 0 { i64::MAX } else { depth }))
    }

    /// Among the newest `depth` messages of `folder`, those whose
    /// attachments aren't known yet (newest first, at most `limit`).
    pub(crate) async fn undigested(&self, folder: &str, depth: i64, limit: i64) -> Result<Vec<u32>> {
        let uids: Vec<i64> = sqlx::query_scalar(
            "SELECT uid FROM (SELECT uid, files FROM messages WHERE account_id = ? AND folder = ?
                              ORDER BY uid DESC LIMIT ?)
             WHERE files IS NULL LIMIT ?",
        )
        .bind(self.account)
        .bind(folder)
        .bind(depth)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(uids.into_iter().map(|u| u as u32).collect())
    }

    /// A message the server didn't send back (deleted meanwhile): nothing
    /// to find in it, so it isn't asked for again.
    pub(crate) async fn mark_no_files(&self, folder: &str, uid: u32) -> Result<()> {
        sqlx::query("UPDATE messages SET files = '[]' WHERE account_id = ? AND folder = ? AND uid = ? AND files IS NULL")
            .bind(self.account)
            .bind(folder)
            .bind(uid as i64)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Bodies downloaded before previews existed (an upgrade): reads up to
    /// `limit` of them again to fill in their preview. Returns how many.
    pub async fn digest_cached(&self, folder: &str, limit: i64) -> Result<usize> {
        let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT b.uid, b.raw FROM bodies b
             JOIN messages m ON m.account_id = b.account_id AND m.folder = b.folder AND m.uid = b.uid
             WHERE b.account_id = ? AND b.folder = ? AND m.files IS NULL
             ORDER BY b.uid DESC LIMIT ?",
        )
        .bind(self.account)
        .bind(folder)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        for (uid, raw) in &rows {
            self.save_digest(folder, *uid as u32, raw).await?;
        }
        Ok(rows.len())
    }

    /// What the list shows (preview, attachments, invitation): kept with
    /// the message, so it stays when the body is evicted. Also used alone
    /// when looking for files: the digest is kept, not the email.
    pub(crate) async fn save_digest(&self, folder: &str, uid: u32, raw: &[u8]) -> Result<()> {
        let digest = crate::mail::Digest::of(raw);
        sqlx::query(
            "UPDATE messages SET preview = ?, files = ?, has_invite = ?, invite_at = ? WHERE account_id = ? AND folder = ? AND uid = ?",
        )
        .bind(&digest.preview)
        .bind(serde_json::to_string(&digest.files).unwrap_or_else(|_| "[]".into()))
        .bind(digest.invite)
        .bind(digest.invite_at)
        .bind(self.account)
        .bind(folder)
        .bind(uid as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn store_body(&self, folder: &str, uid: u32, raw: &[u8]) -> Result<()> {
        self.save_digest(folder, uid, raw).await?;
        sqlx::query(
            "INSERT OR REPLACE INTO bodies (account_id, folder, uid, raw, size, fetched_at)
             SELECT ?, ?, ?, ?, ?, ?
             WHERE EXISTS (SELECT 1 FROM messages WHERE account_id = ? AND folder = ? AND uid = ?)",
        )
        .bind(self.account)
        .bind(folder)
        .bind(uid as i64)
        .bind(raw)
        .bind(raw.len() as i64)
        .bind(now())
        .bind(self.account)
        .bind(folder)
        .bind(uid as i64)
        .execute(&self.pool)
        .await?;
        self.evict_bodies().await
    }

    async fn evict_bodies(&self) -> Result<()> {
        let total: i64 =
            sqlx::query_scalar("SELECT COALESCE(SUM(size), 0) FROM bodies").fetch_one(&self.pool).await?;
        let mut excess = total - self.cache_limit as i64;
        while excess > 0 {
            // Oldest first, one at a time (usually only one or two go).
            let evicted: Option<i64> = sqlx::query_scalar(
                "DELETE FROM bodies WHERE rowid = (SELECT rowid FROM bodies ORDER BY fetched_at, rowid LIMIT 1)
                 RETURNING size",
            )
            .fetch_optional(&self.pool)
            .await?;
            match evicted {
                Some(size) => excess -= size,
                None => break,
            }
        }
        Ok(())
    }
}

/// Lowercase, accents removed, so "Réunion" matches "reunion".
pub fn normalize(parts: &[Option<&str>]) -> String {
    let joined = parts.iter().flatten().copied().collect::<Vec<_>>().join(" ");
    joined
        .nfd() // "é" → "e" + combining accent
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

/// In LIKE patterns `%` and `_` are wildcards: escape them.
/// What a list row needs (messages aliased `m`): `summary_from_row` reads it.
const SUMMARY_COLUMNS: &str = "m.uid, m.subject, m.from_name, m.from_address, m.date, m.unread, m.flagged, \
     m.preview, m.files, m.has_invite, m.invite_at, \
     (SELECT group_concat(ml.label_id) FROM message_labels ml \
      WHERE ml.account_id = m.account_id AND ml.message_id = m.message_id) AS labels, \
     (SELECT sn.until FROM snoozes sn WHERE sn.account_id = m.account_id AND sn.folder = m.folder \
      AND sn.uid = m.uid AND sn.back = 0) AS snoozed_until";

/// A Havre label ("Clients", "Receipts"…).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Label {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub position: i64,
    /// Messages carrying it.
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrganizeCounts {
    pub snoozed: i64,
    pub starred: i64,
}

/// A label's name, trimmed and checked.
fn label_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Config("Give the label a name.".into()));
    }
    if name.chars().count() > 40 {
        return Err(Error::Config("Label names are 40 characters at most.".into()));
    }
    Ok(name.to_owned())
}

/// One of the design's colors (anything else becomes the default).
fn label_color(color: &str) -> &str {
    match color {
        "work" | "personal" | "lyon" | "hol" => color,
        _ => "lyon",
    }
}

/// Which list across accounts (`Database::across`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Inboxes,
    Starred,
    Label(i64),
    Snoozed,
}

/// A message's flags as the server has them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags {
    pub unread: bool,
    pub flagged: bool,
}

/// `<abc@x>` → `abc@x` (the angle brackets are optional in practice).
/// (subject, sender name, sender address, `Message-ID`) of a message.
pub type MailFacts = (String, Option<String>, Option<String>, Option<String>);

pub(crate) fn clean_message_id(id: &str) -> String {
    id.trim().trim_start_matches('<').trim_end_matches('>').to_owned()
}

/// A message row (with `preview`, `files`, `has_invite`) → its summary.
pub(crate) fn summary_from_row(r: &SqliteRow) -> MessageSummary {
    let files: Option<String> = r.try_get("files").ok().flatten();
    MessageSummary {
        uid: r.get::<i64, _>("uid") as u32,
        from: match (r.get::<Option<String>, _>("from_name"), r.get::<Option<String>, _>("from_address")) {
            (None, None) => None,
            (name, address) => Some(Person { name, address }),
        },
        subject: r.get("subject"),
        date: r.get("date"),
        unread: r.get("unread"),
        preview: r.try_get::<Option<String>, _>("preview").ok().flatten().filter(|p| !p.is_empty()),
        attachments: files.is_some_and(|f| f != "[]"),
        invite: r.try_get("has_invite").unwrap_or(false),
        invite_at: r.try_get::<Option<i64>, _>("invite_at").ok().flatten(),
        flagged: r.try_get("flagged").unwrap_or(false),
        labels: r
            .try_get::<Option<String>, _>("labels")
            .ok()
            .flatten()
            .map(|l| l.split(',').filter_map(|x| x.parse().ok()).collect())
            .unwrap_or_default(),
        snoozed_until: r.try_get::<Option<i64>, _>("snoozed_until").ok().flatten(),
        message_id: None,
        account: r.try_get("account_id").ok(),
        folder: r.try_get("folder").ok(),
    }
}

pub(crate) fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn now() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn role_str(role: FolderRole) -> &'static str {
    match role {
        FolderRole::Inbox => "inbox",
        FolderRole::Drafts => "drafts",
        FolderRole::Sent => "sent",
        FolderRole::Archive => "archive",
        FolderRole::Junk => "junk",
        FolderRole::Trash => "trash",
        FolderRole::All => "all",
        FolderRole::Other => "other",
    }
}

fn role_from_str(s: &str) -> FolderRole {
    match s {
        "inbox" => FolderRole::Inbox,
        "drafts" => FolderRole::Drafts,
        "sent" => FolderRole::Sent,
        "archive" => FolderRole::Archive,
        "junk" => FolderRole::Junk,
        "trash" => FolderRole::Trash,
        "all" => FolderRole::All,
        _ => FolderRole::Other,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn folder(path: &str, role: FolderRole) -> Folder {
        Folder { path: path.into(), name: path.into(), depth: 0, role, declared_role: true }
    }

    fn msg(uid: u32, from: &str, subject: &str, unread: bool) -> MessageSummary {
        MessageSummary {
            uid,
            from: Some(Person { name: Some(from.into()), address: Some(format!("{}@x.test", uid)) }),
            subject: subject.into(),
            date: None,
            unread,
            ..Default::default()
        }
    }

    pub(crate) fn new_account(email: &str) -> NewAccount {
        NewAccount {
            name: "Work".into(),
            email: email.into(),
            color: "work".into(),
            imap_host: "imap.x.test".into(),
            imap_port: 993,
            imap_user: String::new(),
            password: "s3cret".into(),
            sender_name: String::new(),
            smtp_host: None,
            smtp_port: None,
            smtp_security: None,
        }
    }

    /// The inbox's unread counter, as the sidebar and the dock show it.
    async fn inbox_unread(store: &Store) -> u32 {
        store.folders().await.unwrap().iter().find(|f| f.folder.path == "INBOX").unwrap().unread
    }

    async fn store_with_inbox() -> Store {
        let db = Database::in_memory().await.unwrap();
        let account = db.insert_account(&new_account("me@x.test")).await.unwrap();
        let store = db.store(account.id);
        store
            .replace_folders(&[folder("INBOX", FolderRole::Inbox), folder("Trash", FolderRole::Trash)])
            .await
            .unwrap();
        store
    }

    #[tokio::test]
    async fn accounts_keep_passwords_encrypted() {
        let db = Database::in_memory().await.unwrap();
        let account = db.insert_account(&new_account("me@x.test")).await.unwrap();
        assert_eq!(account.imap_user, "me@x.test", "user defaults to the email");

        let raw: Vec<u8> =
            sqlx::query_scalar("SELECT imap_password FROM accounts").fetch_one(&db.pool).await.unwrap();
        assert!(!raw.windows(6).any(|w| w == b"s3cret"), "password must not be stored in clear");
        assert!(
            matches!(db.imap_config(account.id).await.unwrap().auth, ImapAuth::Password(p) if p == "s3cret")
        );

        let update =
            AccountUpdate { name: Some("Perso".into()), password: Some("new".into()), ..Default::default() };
        let updated = db.update_account(account.id, &update).await.unwrap();
        assert_eq!(updated.name, "Perso");
        assert!(
            matches!(db.imap_config(account.id).await.unwrap().auth, ImapAuth::Password(p) if p == "new")
        );
    }

    #[tokio::test]
    async fn accounts_are_isolated_and_deleted_with_their_cache() {
        let db = Database::in_memory().await.unwrap();
        let a = db.insert_account(&new_account("a@x.test")).await.unwrap();
        let b = db.insert_account(&new_account("b@x.test")).await.unwrap();
        for id in [a.id, b.id] {
            db.store(id).replace_folders(&[folder("INBOX", FolderRole::Inbox)]).await.unwrap();
        }
        db.store(a.id).upsert_messages("INBOX", &[msg(1, "x", "only in A", false)]).await.unwrap();

        let query = ListQuery { limit: 10, ..Default::default() };
        assert_eq!(db.store(b.id).page("INBOX", &query).await.unwrap().messages.len(), 0);
        assert_eq!(db.store(a.id).page("INBOX", &query).await.unwrap().messages.len(), 1);

        db.delete_account(a.id).await.unwrap();
        let left: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages").fetch_one(&db.pool).await.unwrap();
        assert_eq!(left, 0);
        assert_eq!(db.accounts().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn pages_newest_first_and_searches_without_accents() {
        let store = store_with_inbox().await;
        let messages: Vec<_> = (1..=5).map(|i| msg(i, "Thomas", &format!("Devis {i}"), false)).collect();
        store.upsert_messages("INBOX", &messages).await.unwrap();
        store.upsert_messages("INBOX", &[msg(6, "Anaïs Leroy", "Réunion sécurité", true)]).await.unwrap();
        store.set_backfill("INBOX", Some(1), true).await.unwrap();

        let page = store.page("INBOX", &ListQuery { limit: 4, ..Default::default() }).await.unwrap();
        assert_eq!(page.messages.iter().map(|m| m.uid).collect::<Vec<_>>(), [6, 5, 4, 3]);
        assert!(page.has_more);

        let older =
            store.page("INBOX", &ListQuery { limit: 4, before_uid: Some(3), search: None }).await.unwrap();
        assert_eq!(older.messages.iter().map(|m| m.uid).collect::<Vec<_>>(), [2, 1]);
        assert!(!older.has_more);

        let found = store
            .page("INBOX", &ListQuery { limit: 10, before_uid: None, search: Some("REUNION anais".into()) })
            .await
            .unwrap();
        assert_eq!(found.messages.iter().map(|m| m.uid).collect::<Vec<_>>(), [6]);
    }

    #[tokio::test]
    async fn reconcile_applies_flags_and_deletions() {
        let store = store_with_inbox().await;
        store.upsert_messages("INBOX", &[msg(1, "a", "one", true), msg(2, "b", "two", true)]).await.unwrap();
        store.store_body("INBOX", 2, b"raw").await.unwrap();

        // Server: 1 is now read, 2 is gone.
        let changed =
            store.reconcile("INBOX", 1, 2, &[(1, Flags { unread: false, flagged: true })]).await.unwrap();
        assert_eq!(changed, 2);
        let page = store.page("INBOX", &ListQuery { limit: 10, ..Default::default() }).await.unwrap();
        assert_eq!(page.messages.len(), 1);
        assert!(!page.messages[0].unread);
        assert!(page.messages[0].flagged, "starred on the server: starred here");
        assert_eq!(store.body("INBOX", 2).await.unwrap(), None, "body deleted with its message");
    }

    fn with_id(mut m: MessageSummary, id: &str) -> MessageSummary {
        m.message_id = Some(id.into());
        m
    }

    #[tokio::test]
    async fn labels_follow_the_message_id() {
        let store = store_with_inbox().await;
        let db = Database { pool: store.pool.clone(), ..Database::in_memory().await.unwrap() };
        store.upsert_messages("INBOX", &[with_id(msg(1, "a", "quote", false), "<q1@x.test>")]).await.unwrap();
        let clients = db.create_label(" Clients ", "work").await.unwrap();
        assert_eq!(clients.name, "Clients");
        assert!(db.create_label("clients", "").await.is_err(), "names are unique, ignoring case");

        store.set_label("INBOX", &[1], clients.id, true).await.unwrap();
        let page = store.page("INBOX", &ListQuery { limit: 10, ..Default::default() }).await.unwrap();
        assert_eq!(page.messages[0].labels, [clients.id]);

        // Moved to Trash and back (a new UID): still labelled.
        store.remove_messages("INBOX", &[1]).await.unwrap();
        store.upsert_messages("INBOX", &[with_id(msg(7, "a", "quote", false), "q1@x.test")]).await.unwrap();
        let view = db.across(View::Label(clients.id), 10, 0, None).await.unwrap();
        assert_eq!(view.messages.iter().map(|m| m.uid).collect::<Vec<_>>(), [7]);
        assert_eq!(db.labels().await.unwrap()[0].count, 1);

        // No Message-ID: can't be labelled.
        store.upsert_messages("INBOX", &[with_id(msg(8, "b", "x", false), "")]).await.unwrap();
        assert!(store.set_label("INBOX", &[8], clients.id, true).await.is_err());

        db.delete_label(clients.id).await.unwrap();
        let page = store.page("INBOX", &ListQuery { limit: 10, ..Default::default() }).await.unwrap();
        assert!(page.messages.iter().all(|m| m.labels.is_empty()));
    }

    #[tokio::test]
    async fn tasks_find_their_email_again() {
        let store = store_with_inbox().await;
        store.upsert_messages("INBOX", &[with_id(msg(3, "a", "quote", false), "<q1@x.test>")]).await.unwrap();
        store.upsert_messages("INBOX", &[with_id(msg(4, "b", "no id", false), "")]).await.unwrap();
        let (subject, _, _, id) = store.mail_link("INBOX", 3).await.unwrap();
        assert_eq!((subject.as_str(), id.as_deref()), ("quote", Some("q1@x.test")));
        assert_eq!(store.mail_link("INBOX", 4).await.unwrap().3, None, "an empty Message-ID is none");
        assert_eq!(store.locate("<q1@x.test>").await.unwrap(), Some(("INBOX".to_owned(), 3)));
        assert_eq!(store.locate("").await.unwrap(), None, "never matches messages without one");
        assert!(store.mail_link("INBOX", 99).await.is_err());
    }

    #[tokio::test]
    async fn snoozed_messages_go_away_and_come_back_on_top() {
        let store = store_with_inbox().await;
        let db = Database { pool: store.pool.clone(), ..Database::in_memory().await.unwrap() };
        let mut old = msg(1, "a", "old", false);
        old.date = Some("2026-10-01T09:00:00+00:00".into());
        let mut new = msg(2, "b", "new", false);
        new.date = Some("2026-10-14T09:00:00+00:00".into());
        store.upsert_messages("INBOX", &[old, new]).await.unwrap();

        store.set_unread("INBOX", 1, true).await.unwrap();
        let before = inbox_unread(&store).await;
        store.snooze("INBOX", &[1], Some(2_000_000_000)).await.unwrap();
        assert_eq!(inbox_unread(&store).await, before - 1, "a snoozed unread message isn't counted");
        let page = store.page("INBOX", &ListQuery { limit: 10, ..Default::default() }).await.unwrap();
        assert_eq!(page.messages.iter().map(|m| m.uid).collect::<Vec<_>>(), [2], "away from the inbox");
        let snoozed = db.across(View::Snoozed, 10, 0, None).await.unwrap();
        assert_eq!(snoozed.messages[0].snoozed_until, Some(2_000_000_000));
        assert!(db.due_snoozes(1_999_999_999).await.unwrap().is_empty());
        assert_eq!(db.due_snoozes(2_000_000_000).await.unwrap(), [(store.account, "INBOX".to_owned(), 1)]);

        // Its time came (2033): back, and above the newer message.
        store.mark_back("INBOX", 1).await.unwrap();
        assert_eq!(inbox_unread(&store).await, before, "counted again once back");
        let all = db.across(View::Inboxes, 10, 0, None).await.unwrap();
        assert_eq!(all.messages.iter().map(|m| m.uid).collect::<Vec<_>>(), [1, 2]);
        assert!(db.across(View::Snoozed, 10, 0, None).await.unwrap().messages.is_empty());
    }

    #[tokio::test]
    async fn starred_across_accounts_skips_the_trash() {
        let store = store_with_inbox().await;
        let db = Database { pool: store.pool.clone(), ..Database::in_memory().await.unwrap() };
        let mut a = msg(1, "a", "keep", false);
        a.flagged = true;
        let mut b = msg(2, "b", "binned", false);
        b.flagged = true;
        store.upsert_messages("INBOX", &[a]).await.unwrap();
        store.upsert_messages("Trash", &[b]).await.unwrap();
        let starred = db.across(View::Starred, 10, 0, None).await.unwrap();
        assert_eq!(starred.messages.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["keep"]);
        store.set_flagged("INBOX", &[1], false).await.unwrap();
        assert_eq!(db.organize_counts().await.unwrap().starred, 0);
    }

    #[tokio::test]
    async fn body_cache_evicts_the_oldest() {
        let mut store = store_with_inbox().await;
        store.cache_limit = 10;
        store
            .upsert_messages("INBOX", &[msg(1, "a", "one", false), msg(2, "b", "two", false)])
            .await
            .unwrap();
        store.store_body("INBOX", 1, b"123456").await.unwrap();
        // Make body 1 clearly older.
        sqlx::query("UPDATE bodies SET fetched_at = 0").execute(&store.pool).await.unwrap();
        store.store_body("INBOX", 2, b"abcdef").await.unwrap();
        assert_eq!(store.body("INBOX", 1).await.unwrap(), None);
        assert_eq!(store.body("INBOX", 2).await.unwrap().as_deref(), Some(&b"abcdef"[..]));
    }

    #[tokio::test]
    async fn counters_follow_local_changes() {
        let store = store_with_inbox().await;
        store.upsert_messages("INBOX", &[msg(1, "a", "one", true), msg(2, "b", "two", false)]).await.unwrap();
        store.set_counts("INBOX", 2, 1).await.unwrap();

        store.set_unread("INBOX", 1, false).await.unwrap();
        store.set_unread("INBOX", 1, false).await.unwrap(); // no double count
        let inbox = &store.folders().await.unwrap()[0];
        assert_eq!((inbox.unread, inbox.total), (0, 2));

        store.remove_messages("INBOX", &[2]).await.unwrap();
        let inbox = &store.folders().await.unwrap()[0];
        assert_eq!((inbox.unread, inbox.total), (0, 1));
    }

    #[tokio::test]
    async fn folders_follow_the_server_and_uidvalidity_resets() {
        let store = store_with_inbox().await;
        store.upsert_messages("Trash", &[msg(9, "x", "old", false)]).await.unwrap();
        store.replace_folders(&[folder("INBOX", FolderRole::Inbox)]).await.unwrap();
        assert_eq!(store.folders().await.unwrap().len(), 1);

        store.upsert_messages("INBOX", &[msg(3, "x", "y", false)]).await.unwrap();
        store.reset_folder("INBOX", 42).await.unwrap();
        let state = store.folder_state("INBOX").await.unwrap();
        assert_eq!((state.uid_validity, state.newest_uid, state.complete), (Some(42), None, false));
    }
}
