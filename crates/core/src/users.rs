//! Users (step 29): the people who share one Havre.
//!
//! Every piece of data belongs to one user, through a few "roots" that carry a
//! `user_id` column: mail accounts, calendar accounts and calendars, task
//! lists, notes, contacts and address books, labels, push devices. Everything
//! else hangs off a root (a message off its account, an event off its
//! calendar…).
//!
//! The rule that keeps users apart is enforced by types, not by remembering:
//! the stores that read or write a user's data are made with a [`UserId`]
//! (`db.notes(user)`, `db.tasks(user)`…), so a query can't "forget" whose data
//! it wants: without a `UserId` there's no store to query with.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::Result;

/// A user's id. A type of its own (a "newtype") rather than a bare `i64`, so
/// that a user can't be passed where an account or a note id is expected:
/// the compiler tells them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(pub i64);

impl UserId {
    /// The admin: user 1, who set Havre up.
    pub const ADMIN: UserId = UserId(1);
}

/// A user as the app shows them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct User {
    pub id: UserId,
    pub name: String,
    pub login: Option<String>,
    pub admin: bool,
}

/// Reading users.
#[derive(Clone)]
pub struct Users {
    pool: SqlitePool,
}

impl Users {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, id: UserId) -> Result<User> {
        let (name, login, admin): (String, Option<String>, bool) =
            sqlx::query_as("SELECT name, login, admin FROM users WHERE id = ?")
                .bind(id.0)
                .fetch_optional(&self.pool)
                .await?
                .ok_or_else(|| crate::Error::NotFound(format!("user {}", id.0)))?;
        Ok(User { id, name, login, admin })
    }

    /// Adds someone (not an admin). They start like a new Havre: Havre's own
    /// calendar and an Inbox task list, the default dock (no row: defaults).
    /// How they sign in comes later (a password in 29b, SSO in step 30).
    pub async fn add(&self, name: &str) -> Result<UserId> {
        let now = now();
        // One transaction: the user and their starting calendar and list, or nothing.
        let mut tx = self.pool.begin().await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO users (name, admin, created_at, updated_at) VALUES (?, 0, ?, ?) RETURNING id",
        )
        .bind(name.trim())
        .bind(now)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO calendars (source, remote_id, name, color, is_default, user_id)
             VALUES ('local', 'havre', 'Havre', '#e3bc6a', 1, ?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO task_lists (source, remote_id, name, color, is_default, user_id)
             VALUES ('local', 'inbox', 'Inbox', '#c99ab0', 1, ?)",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(UserId(id))
    }

    /// Everyone, oldest first.
    pub async fn all(&self) -> Result<Vec<UserId>> {
        let ids: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM users ORDER BY id").fetch_all(&self.pool).await?;
        Ok(ids.into_iter().map(UserId).collect())
    }

    /// Whose a mail account is (background work finds its user this way).
    pub async fn of_account(&self, account: i64) -> Result<Option<UserId>> {
        let user: Option<Option<i64>> = sqlx::query_scalar("SELECT user_id FROM accounts WHERE id = ?")
            .bind(account)
            .fetch_optional(&self.pool)
            .await?;
        Ok(user.flatten().map(UserId))
    }

    /// Is `account` one of `user`'s? A user asking for another's account gets
    /// "not found", never "forbidden": no hint that it exists.
    pub async fn owns_account(&self, user: UserId, account: i64) -> Result<()> {
        if self.of_account(account).await? == Some(user) {
            Ok(())
        } else {
            Err(crate::Error::NotFound(format!("account {account}")))
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, str::FromStr};

    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    use super::*;
    use crate::store::Database;

    /// An existing Havre (before step 29) moves to users without losing
    /// anything: everything becomes user 1's, the admin's.
    #[tokio::test]
    async fn an_existing_havre_becomes_the_admins() {
        let options = SqliteConnectOptions::from_str("sqlite::memory:").unwrap().foreign_keys(true);
        let pool = SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        // The migrations up to step 28, then some of what a Havre holds.
        let mut before = sqlx::migrate!("./migrations");
        before.migrations =
            Cow::Owned(before.migrations.iter().filter(|m| m.version < 28).cloned().collect());
        before.run(&pool).await.unwrap();
        for sql in [
            "INSERT INTO owner (id, password_hash, created_at, updated_at) VALUES (1, 'hash', 1, 1)",
            "INSERT INTO sessions (token_hash, created_at, last_seen, user_agent) VALUES (x'01', 1, 1, 'laptop')",
            "INSERT INTO accounts (name, email, imap_host, imap_port, imap_user, imap_password, created_at)
             VALUES ('Work', 'me@x.test', 'imap.x.test', 993, 'me', x'00', 1)",
            "INSERT INTO notes (title, created_at, updated_at) VALUES ('Atelier', 1, 1)",
            "INSERT INTO labels (name, color, position) VALUES ('Clients', 'work', 1)",
            "INSERT INTO message_labels (label_id, account_id, message_id) VALUES (1, 1, 'm1@x.test')",
            "INSERT INTO contacts (name, created_at, updated_at) VALUES ('Claire', 1, 1)",
            "INSERT INTO contact_emails (email, contact_id, position) VALUES ('claire@x.test', 1, 0)",
            "INSERT INTO invitation_answers (uid, response, answered_at) VALUES ('meet-1', 'accepted', 1)",
            "INSERT INTO app_settings (id, enabled, position) VALUES ('notes', 1, 3)",
            "UPDATE mail_settings SET undo_seconds = 30",
            "UPDATE agenda_settings SET time_zone = 'Europe/Paris'",
            "UPDATE push_settings SET new_mail = 0",
            "UPDATE files_settings SET depth = 500",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(sql)).execute(&pool).await.unwrap();
        }

        // Step 29a's migration.
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        let one = |sql: &'static str| sqlx::query_scalar::<_, i64>(sql).fetch_one(&pool);
        assert_eq!(
            one("SELECT COUNT(*) FROM users WHERE id = 1 AND admin = 1 AND password_hash = 'hash'")
                .await
                .unwrap(),
            1
        );
        for table in [
            "sessions",
            "accounts",
            "notes",
            "labels",
            "contacts",
            "contact_emails",
            "calendars",
            "task_lists",
            "invitation_answers",
            "user_apps",
        ] {
            let sql = format!("SELECT COUNT(*) FROM {table} WHERE user_id IS NOT 1");
            let others: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).fetch_one(&pool).await.unwrap();
            assert_eq!(others, 0, "{table}: everything is user 1's");
        }
        // Nothing lost: the label stays on its message, the settings are kept.
        assert_eq!(one("SELECT COUNT(*) FROM message_labels").await.unwrap(), 1);
        assert_eq!(one("SELECT undo_seconds FROM mail_settings WHERE user_id = 1").await.unwrap(), 30);
        assert_eq!(one("SELECT new_mail FROM push_prefs WHERE user_id = 1").await.unwrap(), 0);
        assert_eq!(one("SELECT depth FROM files_settings WHERE user_id = 1").await.unwrap(), 500);
        let zone: String = sqlx::query_scalar("SELECT time_zone FROM agenda_settings WHERE user_id = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(zone, "Europe/Paris");
    }

    /// A new user starts like a new Havre, and the same names are free for
    /// them (a label, a contact's address).
    #[tokio::test]
    async fn a_new_user_starts_empty_and_apart() {
        let db = Database::in_memory().await.unwrap();
        let lea = db.users().add("Léa").await.unwrap();
        assert_ne!(lea, UserId::ADMIN);
        assert_eq!(db.users().all().await.unwrap(), [UserId::ADMIN, lea]);
        assert!(!db.users().get(lea).await.unwrap().admin);

        // Her own calendar and Inbox list, the admin's untouched.
        let calendars = db.agenda(lea).calendars().await.unwrap();
        assert_eq!(calendars.len(), 1);
        assert!(calendars[0].is_default);
        assert_ne!(calendars[0].id, db.agenda(UserId::ADMIN).calendars().await.unwrap()[0].id);
        assert_eq!(db.tasks(lea).lists().await.unwrap().len(), 1);

        // The same label name, the same contact address: each their own.
        db.create_label(UserId::ADMIN, "Family", "work").await.unwrap();
        db.create_label(lea, "family", "work").await.unwrap();
        db.people(UserId::ADMIN).create("Mum", Some("mum@x.test")).await.unwrap();
        db.people(lea).create("Grandma", Some("mum@x.test")).await.unwrap();
        assert_eq!(db.people(lea).person("mum@x.test").await.unwrap().person.name, "Grandma");
        assert_eq!(db.people(UserId::ADMIN).person("mum@x.test").await.unwrap().person.name, "Mum");

        // AI is the admin's only: not even in her app library.
        assert!(db.apps(lea).list().await.unwrap().iter().all(|a| a.manifest.id != "ai"));
        assert!(db.apps(lea).set_enabled("ai", true).await.is_err());
        assert!(db.apps(UserId::ADMIN).list().await.unwrap().iter().any(|a| a.manifest.id == "ai"));
    }

    /// A mail account is reached through its user: anyone else's is "not found".
    #[tokio::test]
    async fn accounts_and_their_engines_are_their_users() {
        let db = Database::in_memory().await.unwrap();
        let lea = db.users().add("Léa").await.unwrap();
        let new = crate::store::tests::new_account("me@x.test");
        // Port 1 on this machine: the sync engine fails at once, no network.
        let new = crate::store::NewAccount { imap_host: "127.0.0.1".into(), imap_port: 1, ..new };
        let mine = db.insert_account(UserId::ADMIN, &new).await.unwrap().id;
        let manager = crate::accounts::AccountManager::start(db.clone()).await.unwrap();

        assert!(manager.engine(UserId::ADMIN, mine).await.is_ok());
        assert!(matches!(manager.engine(lea, mine).await, Err(crate::Error::NotFound(_))));
        assert!(matches!(db.mailbox(lea, mine).await, Err(crate::Error::NotFound(_))));
        assert!(manager.accounts(lea).await.unwrap().is_empty());
        assert!(manager.remove(lea, mine).await.is_err());
        assert_eq!(manager.accounts(UserId::ADMIN).await.unwrap().len(), 1, "still there");
        assert!(db.drafts(lea).list().await.unwrap().is_empty());
        let draft = crate::compose::NewDraft {
            account: mine,
            kind: crate::compose::DraftKind::New,
            folder: None,
            uid: None,
            to: String::new(),
            subject: String::new(),
        };
        assert!(manager.new_draft(lea, draft).await.is_err(), "not from someone else's account");
    }
}
