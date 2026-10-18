//! The Tasks tables: lists, tasks (with their links), which accounts sync.

use serde::Deserialize;
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use super::{COLORS, DAY, EventLink, Links, MailLink, PersonLink, RemoteList, RemoteTask, Task, TaskList};
use crate::{Error, Result};

#[derive(Clone)]
pub struct TaskStore {
    pool: SqlitePool,
}

/// A list plus what syncing needs.
#[derive(Debug, Clone)]
pub(crate) struct ListRow {
    pub list: TaskList,
    pub remote_id: String,
}

/// A task plus what writing it back needs.
#[derive(Debug, Clone)]
pub(crate) struct TaskRow {
    pub task: Task,
    pub remote_id: String,
    pub href: Option<String>,
    pub etag: Option<String>,
    pub raw: Option<String>,
}

/// What a provider can't keep, and Havre keeps for it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Keeps {
    /// The due *time* (Microsoft and Google only keep the day).
    pub time: bool,
    /// The reminder (Google Tasks has none in its API).
    pub reminder: bool,
}

/// Which tasks to list (`GET /api/tasks?…`). Every filter is optional.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TaskQuery {
    pub list: Option<i64>,
    /// `true`: only done ones, `false`: only those still to do. Done tasks
    /// older than 30 days are never listed.
    pub done: Option<bool>,
    /// Due on a day in `[from, to)` (midnight UTC).
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Linked to this person (address).
    pub person: Option<String>,
    /// Made from this email.
    pub message_id: Option<String>,
    /// Words in the title or notes.
    pub q: Option<String>,
    pub limit: Option<i64>,
}

const LIST_COLUMNS: &str = "l.id, l.source, l.account_id, l.caldav_id, l.remote_id, l.name, l.color, l.read_only, \
     l.is_default, l.synced_at, l.error, COALESCE(a.name, c.name, 'Havre') AS owner, \
     (SELECT COUNT(*) FROM tasks t WHERE t.list_id = l.id AND t.done_at IS NULL) AS open";

const LIST_FROM: &str = "task_lists l LEFT JOIN accounts a ON a.id = l.account_id \
     LEFT JOIN caldav_accounts c ON c.id = l.caldav_id";

const TASK_COLUMNS: &str = "t.id, t.list_id, t.remote_id, t.href, t.etag, t.raw, t.title, t.notes, t.due, t.due_at, \
     t.remind_at, t.done_at, t.updated_at, l.read_only, t.mail_account, t.mail_message_id, t.mail_folder, \
     t.mail_uid, t.mail_subject, t.mail_from, t.person, t.person_name, t.event_uid, t.event_title, t.event_start";

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

impl TaskStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// A transaction that takes the write lock at once. A plain `BEGIN`
    /// starts as a reader; when it then writes while another connection is
    /// writing (mail sync, the Agenda), SQLite can't upgrade it and answers
    /// "database is locked" without waiting. `BEGIN IMMEDIATE` waits its
    /// turn instead (up to the busy timeout).
    async fn write(&self) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }

    // ---- Lists ----------------------------------------------------------------------

    pub async fn lists(&self) -> Result<Vec<TaskList>> {
        Ok(self.list_rows().await?.into_iter().map(|r| r.list).collect())
    }

    pub(crate) async fn list_rows(&self) -> Result<Vec<ListRow>> {
        let rows = sqlx::query(&format!(
            "SELECT {LIST_COLUMNS} FROM {LIST_FROM}
             ORDER BY l.source = 'local', l.caldav_id IS NOT NULL, l.account_id, l.caldav_id, l.is_default DESC,
                      l.name COLLATE NOCASE"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(list_from_row).collect())
    }

    pub(crate) async fn list_row(&self, id: i64) -> Result<ListRow> {
        let row = sqlx::query(&format!("SELECT {LIST_COLUMNS} FROM {LIST_FROM} WHERE l.id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("task list {id}")))?;
        Ok(list_from_row(&row))
    }

    /// The list new tasks go to.
    pub(crate) async fn default_list(&self) -> Result<i64> {
        let id: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM task_lists WHERE read_only = 0 ORDER BY is_default DESC, source = 'local' DESC, id LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        id.ok_or_else(|| Error::Config("There's no list to put the task in.".into()))
    }

    /// Makes the stored lists of one source match what it lists. Lists that
    /// disappeared go, with their tasks.
    pub(crate) async fn replace_lists(
        &self,
        source: &str,
        account: Option<i64>,
        caldav: Option<i64>,
        remotes: &[RemoteList],
    ) -> Result<()> {
        // `IS` (not `=`) so that NULL matches NULL.
        let existing: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, remote_id FROM task_lists WHERE account_id IS ? AND caldav_id IS ? AND source = ?",
        )
        .bind(account)
        .bind(caldav)
        .bind(source)
        .fetch_all(&self.pool)
        .await?;
        // The first account's main list becomes the default (instead of
        // Havre's Inbox) — unless you already chose another one.
        let has_default: bool = sqlx::query_scalar(
            "SELECT COUNT(*) > 0 FROM task_lists WHERE is_default = 1 AND source <> 'local'",
        )
        .fetch_one(&self.pool)
        .await?;
        let mut tx = self.write().await?;
        for (i, remote) in remotes.iter().enumerate() {
            match existing.iter().find(|(_, r)| *r == remote.remote_id) {
                Some((id, _)) => {
                    sqlx::query("UPDATE task_lists SET name = ?, read_only = ? WHERE id = ?")
                        .bind(&remote.name)
                        .bind(remote.read_only)
                        .bind(id)
                        .execute(&mut *tx)
                        .await?;
                }
                None => {
                    let color = remote.color.clone().unwrap_or_else(|| COLORS[i % COLORS.len()].to_owned());
                    let default = remote.is_default && !has_default && !remote.read_only;
                    if default {
                        sqlx::query("UPDATE task_lists SET is_default = 0").execute(&mut *tx).await?;
                    }
                    sqlx::query(
                        "INSERT INTO task_lists (source, account_id, caldav_id, remote_id, name, color, read_only, is_default)
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind(source)
                    .bind(account)
                    .bind(caldav)
                    .bind(&remote.remote_id)
                    .bind(&remote.name)
                    .bind(color)
                    .bind(remote.read_only)
                    .bind(default)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }
        for (id, remote_id) in &existing {
            if !remotes.iter().any(|r| r.remote_id == *remote_id) {
                sqlx::query("DELETE FROM task_lists WHERE id = ?").bind(id).execute(&mut *tx).await?;
            }
        }
        tx.commit().await?;
        self.keep_a_default().await
    }

    /// Forgets the lists of a source (its sync was turned off, or it lost
    /// the permission). The tasks stay at the provider.
    pub(crate) async fn remove_lists(
        &self,
        source: &str,
        account: Option<i64>,
        caldav: Option<i64>,
    ) -> Result<()> {
        sqlx::query("DELETE FROM task_lists WHERE account_id IS ? AND caldav_id IS ? AND source = ?")
            .bind(account)
            .bind(caldav)
            .bind(source)
            .execute(&self.pool)
            .await?;
        self.keep_a_default().await
    }

    /// If the default list went away, Havre's Inbox takes over.
    async fn keep_a_default(&self) -> Result<()> {
        sqlx::query(
            "UPDATE task_lists SET is_default = 1
             WHERE NOT EXISTS (SELECT 1 FROM task_lists WHERE is_default = 1)
               AND id = (SELECT id FROM task_lists WHERE read_only = 0 ORDER BY source = 'local' DESC, id LIMIT 1)",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(crate) async fn set_list_status(&self, id: i64, error: Option<&str>) -> Result<()> {
        let synced = if error.is_none() { Some(now()) } else { None };
        sqlx::query("UPDATE task_lists SET error = ?, synced_at = COALESCE(?, synced_at) WHERE id = ?")
            .bind(error)
            .bind(synced)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(crate) async fn set_source_error(
        &self,
        source: &str,
        account: Option<i64>,
        caldav: Option<i64>,
        error: &str,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE task_lists SET error = ? WHERE account_id IS ? AND caldav_id IS ? AND source = ?",
        )
        .bind(error)
        .bind(account)
        .bind(caldav)
        .bind(source)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A new Havre list.
    pub async fn create_local_list(&self, name: &str) -> Result<TaskList> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Config("Give the list a name.".into()));
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_lists").fetch_one(&self.pool).await?;
        let id =
            sqlx::query("INSERT INTO task_lists (source, remote_id, name, color) VALUES ('local', ?, ?, ?)")
                .bind(format!("local-{}", crate::agenda::random_id()))
                .bind(name)
                .bind(COLORS[count as usize % COLORS.len()])
                .execute(&self.pool)
                .await?
                .last_insert_rowid();
        Ok(self.list_row(id).await?.list)
    }

    /// Renames or recolors a list, or makes it the default. Only Havre's own
    /// lists can be renamed (the others: in their app).
    pub async fn update_list(
        &self,
        id: i64,
        name: Option<&str>,
        color: Option<&str>,
        is_default: Option<bool>,
    ) -> Result<TaskList> {
        let row = self.list_row(id).await?;
        if let Some(name) = name {
            if row.list.source != "local" {
                return Err(Error::Config("Rename this list in the app it comes from.".into()));
            }
            if name.trim().is_empty() {
                return Err(Error::Config("Give the list a name.".into()));
            }
            sqlx::query("UPDATE task_lists SET name = ? WHERE id = ?")
                .bind(name.trim())
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        if let Some(color) = color {
            let valid = color.len() == 7
                && color.starts_with('#')
                && color[1..].chars().all(|c| c.is_ascii_hexdigit());
            if !valid {
                return Err(Error::Config("A color looks like #86a9d1.".into()));
            }
            sqlx::query("UPDATE task_lists SET color = ? WHERE id = ?")
                .bind(color.to_lowercase())
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        if is_default == Some(true) {
            if row.list.read_only {
                return Err(Error::Config(format!("“{}” is read-only.", row.list.name)));
            }
            let mut tx = self.write().await?;
            sqlx::query("UPDATE task_lists SET is_default = (id = ?)").bind(id).execute(&mut *tx).await?;
            tx.commit().await?;
        }
        Ok(self.list_row(id).await?.list)
    }

    /// Deletes one of Havre's own lists, with its tasks. The last one stays.
    pub async fn delete_local_list(&self, id: i64) -> Result<()> {
        let row = self.list_row(id).await?;
        if row.list.source != "local" {
            return Err(Error::Config("Delete this list in the app it comes from.".into()));
        }
        let locals: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM task_lists WHERE source = 'local'")
            .fetch_one(&self.pool)
            .await?;
        if locals <= 1 {
            return Err(Error::Config("Keep at least one Havre list.".into()));
        }
        sqlx::query("DELETE FROM task_lists WHERE id = ?").bind(id).execute(&self.pool).await?;
        self.keep_a_default().await
    }

    // ---- Which accounts sync ----------------------------------------------------------

    /// `account:3` / `caldav:2` → on or off. No choice yet: mail accounts
    /// on, CalDAV servers off.
    pub async fn sync_enabled(&self, key: &str) -> Result<bool> {
        let stored: Option<bool> = sqlx::query_scalar("SELECT enabled FROM task_sync WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(stored.unwrap_or(key.starts_with("account:")))
    }

    pub async fn set_sync_enabled(&self, key: &str, enabled: bool) -> Result<()> {
        sqlx::query(
            "INSERT INTO task_sync (key, enabled) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET enabled = excluded.enabled",
        )
        .bind(key)
        .bind(enabled)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ---- Tasks ------------------------------------------------------------------------

    pub async fn tasks(&self, query: &TaskQuery) -> Result<Vec<Task>> {
        // Each filter is written `(? IS NULL OR …)`: when a filter isn't
        // given, its `?` is NULL and the condition is always true. Plain SQL,
        // no string building.
        let since = now() - 30 * DAY;
        // Words: SQL finds tasks with the first one; the others are
        // checked below (every word must be in the title or the notes).
        let words: Vec<String> =
            query.q.as_deref().unwrap_or_default().split_whitespace().map(str::to_lowercase).collect();
        let first = words.first().map(|w| format!("%{w}%"));
        let rows = sqlx::query(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks t JOIN task_lists l ON l.id = t.list_id
             WHERE (t.done_at IS NULL OR t.done_at >= ?)
               AND (? IS NULL OR t.list_id = ?)
               AND (? IS NULL OR (t.done_at IS NOT NULL) = ?)
               AND (? IS NULL OR t.due >= ?)
               AND (? IS NULL OR t.due < ?)
               AND (? IS NULL OR t.person = lower(?))
               AND (? IS NULL OR t.mail_message_id = ?)
               AND (? IS NULL OR t.title LIKE ? OR t.notes LIKE ?)
             ORDER BY t.done_at IS NOT NULL, t.due IS NULL, t.due, COALESCE(t.due_at, t.due), t.id
             LIMIT ?"
        ))
        .bind(since)
        .bind(query.list)
        .bind(query.list)
        .bind(query.done)
        .bind(query.done)
        .bind(query.from)
        .bind(query.from)
        .bind(query.to)
        .bind(query.to)
        .bind(&query.person)
        .bind(&query.person)
        .bind(&query.message_id)
        .bind(&query.message_id)
        .bind(&first)
        .bind(&first)
        .bind(&first)
        .bind(if words.len() > 1 { 5000 } else { query.limit.unwrap_or(2000).clamp(1, 5000) })
        .fetch_all(&self.pool)
        .await?;
        let has_all = |t: &Task| {
            let text = format!("{} {}", t.title, t.notes).to_lowercase();
            words.iter().all(|w| text.contains(w.as_str()))
        };
        let limit = query.limit.unwrap_or(2000).clamp(1, 5000) as usize;
        Ok(rows.iter().map(|r| task_from_row(r).task).filter(has_all).take(limit).collect())
    }

    pub async fn task(&self, id: i64) -> Result<Task> {
        Ok(self.task_row(id).await?.task)
    }

    pub(crate) async fn task_row(&self, id: i64) -> Result<TaskRow> {
        let row = sqlx::query(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks t JOIN task_lists l ON l.id = t.list_id WHERE t.id = ?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| Error::NotFound(format!("task {id}")))?;
        Ok(task_from_row(&row))
    }

    /// Makes the stored tasks of a list match what the provider has. Links
    /// are kept; so is what the provider can't hold (`keeps`).
    pub(crate) async fn replace_tasks(&self, list: i64, remotes: &[RemoteTask], keeps: Keeps) -> Result<()> {
        let mut tx = self.write().await?;
        for remote in remotes {
            save(&mut tx, list, remote, None, keeps).await?;
        }
        let existing: Vec<(i64, String)> =
            sqlx::query_as("SELECT id, remote_id FROM tasks WHERE list_id = ?")
                .bind(list)
                .fetch_all(&mut *tx)
                .await?;
        for (id, remote_id) in existing {
            if !remotes.iter().any(|r| r.remote_id == remote_id) {
                sqlx::query("DELETE FROM tasks WHERE id = ?").bind(id).execute(&mut *tx).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Saves one task (just created or changed at the provider), with its
    /// links when given.
    pub(crate) async fn save_task(
        &self,
        list: i64,
        remote: &RemoteTask,
        links: Option<&Links>,
        keeps: Keeps,
    ) -> Result<Task> {
        let mut tx = self.write().await?;
        let id = save(&mut tx, list, remote, links, keeps).await?;
        tx.commit().await?;
        self.task(id).await
    }

    pub(crate) async fn delete_task(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM tasks WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Tasks whose reminder time has come and that weren't shown yet.
    pub(crate) async fn due_reminders(&self, now: i64) -> Result<Vec<Task>> {
        let rows = sqlx::query(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks t JOIN task_lists l ON l.id = t.list_id
             WHERE t.remind_at IS NOT NULL AND t.remind_at <= ? AND t.reminded = 0 AND t.done_at IS NULL"
        ))
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| task_from_row(r).task).collect())
    }

    pub(crate) async fn mark_reminded(&self, id: i64) -> Result<()> {
        sqlx::query("UPDATE tasks SET reminded = 1 WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(())
    }
}

/// Inserts or updates one task, by `(list, remote_id)`.
async fn save(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    list: i64,
    remote: &RemoteTask,
    links: Option<&Links>,
    keeps: Keeps,
) -> Result<i64> {
    // (id, due, due_at, remind_at) of the copy we have, if any.
    type Current = (i64, Option<i64>, Option<i64>, Option<i64>);
    let current: Option<Current> =
        sqlx::query_as("SELECT id, due, due_at, remind_at FROM tasks WHERE list_id = ? AND remote_id = ?")
            .bind(list)
            .bind(&remote.remote_id)
            .fetch_optional(&mut **tx)
            .await?;

    let mut due_at = remote.due_at;
    let mut remind_at = remote.remind_at;
    if let Some((_, old_due, old_due_at, old_remind)) = current {
        // Still due on the same day: keep the time we know.
        if keeps.time && due_at.is_none() && remote.due.is_some() && remote.due == old_due {
            due_at = old_due_at;
        }
        if keeps.reminder && remind_at.is_none() {
            remind_at = old_remind;
        }
    }
    let updated = remote.updated_at.unwrap_or_else(now);

    let id = match current {
        Some((id, ..)) => {
            // A new reminder time is shown again.
            sqlx::query(
                "UPDATE tasks SET href = ?, etag = ?, raw = ?, title = ?, notes = ?, due = ?, due_at = ?,
                        reminded = CASE WHEN remind_at IS ? THEN reminded ELSE 0 END, remind_at = ?,
                        done_at = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&remote.href)
            .bind(&remote.etag)
            .bind(&remote.raw)
            .bind(&remote.title)
            .bind(&remote.notes)
            .bind(remote.due)
            .bind(due_at)
            .bind(remind_at)
            .bind(remind_at)
            .bind(remote.done_at)
            .bind(updated)
            .bind(id)
            .execute(&mut **tx)
            .await?;
            id
        }
        None => sqlx::query(
            "INSERT INTO tasks (list_id, remote_id, href, etag, raw, title, notes, due, due_at, remind_at, done_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(list)
        .bind(&remote.remote_id)
        .bind(&remote.href)
        .bind(&remote.etag)
        .bind(&remote.raw)
        .bind(&remote.title)
        .bind(&remote.notes)
        .bind(remote.due)
        .bind(due_at)
        .bind(remind_at)
        .bind(remote.done_at)
        .bind(updated)
        .execute(&mut **tx)
        .await?
        .last_insert_rowid(),
    };

    if let Some(links) = links {
        let mail = links.mail.as_ref();
        let person = links.person.as_ref();
        let event = links.event.as_ref();
        sqlx::query(
            "UPDATE tasks SET mail_account = ?, mail_message_id = ?, mail_folder = ?, mail_uid = ?, mail_subject = ?,
                    mail_from = ?, person = ?, person_name = ?, event_uid = ?, event_title = ?, event_start = ?
             WHERE id = ?",
        )
        .bind(mail.map(|m| m.account))
        .bind(mail.and_then(|m| m.message_id.clone()).filter(|id| !id.trim().is_empty()))
        .bind(mail.and_then(|m| m.folder.clone()))
        .bind(mail.and_then(|m| m.uid).map(i64::from))
        .bind(mail.map(|m| m.subject.clone()))
        .bind(mail.map(|m| m.from.clone()))
        .bind(person.map(|p| p.email.trim().to_lowercase()))
        .bind(person.and_then(|p| p.name.clone()))
        .bind(event.map(|e| e.uid.clone()))
        .bind(event.map(|e| e.title.clone()))
        .bind(event.map(|e| e.start))
        .bind(id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(id)
}

fn list_from_row(row: &SqliteRow) -> ListRow {
    ListRow {
        remote_id: row.get("remote_id"),
        list: TaskList {
            id: row.get("id"),
            source: row.get("source"),
            account: row.get("account_id"),
            caldav: row.get("caldav_id"),
            owner: row.get("owner"),
            name: row.get("name"),
            color: row.get("color"),
            read_only: row.get("read_only"),
            is_default: row.get("is_default"),
            open: row.get("open"),
            synced_at: row.get("synced_at"),
            error: row.get("error"),
        },
    }
}

fn task_from_row(row: &SqliteRow) -> TaskRow {
    let mail = row.get::<Option<i64>, _>("mail_account").map(|account| MailLink {
        account,
        message_id: row.get("mail_message_id"),
        folder: row.get("mail_folder"),
        uid: row.get::<Option<i64>, _>("mail_uid").map(|u| u as u32),
        subject: row.get::<Option<String>, _>("mail_subject").unwrap_or_default(),
        from: row.get::<Option<String>, _>("mail_from").unwrap_or_default(),
    });
    let person = row
        .get::<Option<String>, _>("person")
        .map(|email| PersonLink { email, name: row.get("person_name") });
    let event = row.get::<Option<String>, _>("event_uid").map(|uid| EventLink {
        uid,
        title: row.get::<Option<String>, _>("event_title").unwrap_or_default(),
        start: row.get::<Option<i64>, _>("event_start").unwrap_or_default(),
    });
    TaskRow {
        remote_id: row.get("remote_id"),
        href: row.get("href"),
        etag: row.get("etag"),
        raw: row.get("raw"),
        task: Task {
            id: row.get("id"),
            list: row.get("list_id"),
            title: row.get("title"),
            notes: row.get("notes"),
            due: row.get("due"),
            due_at: row.get("due_at"),
            remind_at: row.get("remind_at"),
            done_at: row.get("done_at"),
            updated_at: row.get("updated_at"),
            read_only: row.get("read_only"),
            links: Links { mail, person, event },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Database;

    const KEEP_ALL: Keeps = Keeps { time: true, reminder: true };

    fn remote(id: &str, title: &str, due: Option<i64>) -> RemoteTask {
        RemoteTask { remote_id: id.into(), title: title.into(), due, ..Default::default() }
    }

    #[tokio::test]
    async fn lists_and_tasks_follow_the_provider() {
        let db = Database::in_memory().await.unwrap();
        let store = db.tasks();
        // Havre's Inbox is there from the start, and is the default.
        let lists = store.lists().await.unwrap();
        assert_eq!(lists.len(), 1);
        assert!(lists[0].is_default && lists[0].source == "local");

        // A CalDAV server with two task lists (CalDAV needs no mail account).
        let caldav = db.agenda().insert_caldav("Cloud", "https://c.example", "me", "pw").await.unwrap();
        let remotes = [
            RemoteList {
                remote_id: "a".into(),
                name: "Work".into(),
                color: None,
                read_only: false,
                is_default: true,
            },
            RemoteList {
                remote_id: "b".into(),
                name: "Home".into(),
                color: None,
                read_only: false,
                is_default: false,
            },
        ];
        store.replace_lists("caldav", None, Some(caldav), &remotes).await.unwrap();
        let lists = store.lists().await.unwrap();
        assert_eq!(lists.len(), 3);
        let work = lists.iter().find(|l| l.name == "Work").unwrap();
        assert!(work.is_default, "the account's main list becomes the default");
        assert_eq!(work.owner, "Cloud");
        assert_eq!(store.default_list().await.unwrap(), work.id);

        // Tasks arrive; one has links (made from an email).
        let day = 20_000 * DAY;
        store
            .replace_tasks(
                work.id,
                &[remote("1", "Call Thomas", Some(day)), remote("2", "Pay", None)],
                KEEP_ALL,
            )
            .await
            .unwrap();
        let links = Links {
            mail: Some(MailLink {
                account: 1,
                message_id: Some("m@x".into()),
                subject: "Devis".into(),
                ..Default::default()
            }),
            person: Some(PersonLink { email: "Thomas@Renard.fr".into(), name: Some("Thomas".into()) }),
            event: None,
        };
        let mut timed = remote("1", "Call Thomas", Some(day));
        timed.due_at = Some(day + 10 * 3600);
        timed.remind_at = Some(day + 9 * 3600);
        let saved = store.save_task(work.id, &timed, Some(&links), KEEP_ALL).await.unwrap();
        assert_eq!(saved.links.person.as_ref().unwrap().email, "thomas@renard.fr");

        // The next sync knows only the day (Microsoft, Google): the time,
        // the reminder and the links stay.
        store.replace_tasks(work.id, &[remote("1", "Call Thomas back", Some(day))], KEEP_ALL).await.unwrap();
        let all = store.tasks(&TaskQuery::default()).await.unwrap();
        assert_eq!(all.len(), 1, "task 2 was deleted at the provider");
        assert_eq!(all[0].title, "Call Thomas back");
        assert_eq!(all[0].due_at, Some(day + 10 * 3600));
        assert_eq!(all[0].remind_at, Some(day + 9 * 3600));
        assert_eq!(all[0].links, links_lowercase(links.clone()));

        // Moved to another day: the old time no longer applies.
        store
            .replace_tasks(work.id, &[remote("1", "Call Thomas back", Some(day + DAY))], KEEP_ALL)
            .await
            .unwrap();
        assert_eq!(store.task(all[0].id).await.unwrap().due_at, None);

        // Filters.
        let by_person = TaskQuery { person: Some("THOMAS@renard.fr".into()), ..Default::default() };
        assert_eq!(store.tasks(&by_person).await.unwrap().len(), 1);
        let by_day = TaskQuery { from: Some(day), to: Some(day + DAY), ..Default::default() };
        assert_eq!(store.tasks(&by_day).await.unwrap().len(), 0);
        let by_words = TaskQuery { q: Some("thomas".into()), ..Default::default() };
        assert_eq!(store.tasks(&by_words).await.unwrap().len(), 1);
        let both = TaskQuery { q: Some("back THOMAS".into()), ..Default::default() };
        assert_eq!(store.tasks(&both).await.unwrap().len(), 1, "words in any order");
        let missing = TaskQuery { q: Some("thomas tomorrow".into()), ..Default::default() };
        assert!(store.tasks(&missing).await.unwrap().is_empty());

        // Reminders: due ones, once.
        assert_eq!(store.due_reminders(day + 9 * 3600).await.unwrap().len(), 1);
        store.mark_reminded(all[0].id).await.unwrap();
        assert!(store.due_reminders(day + 9 * 3600).await.unwrap().is_empty());

        // The server's lists go: their tasks too; the Inbox is the default again.
        store.remove_lists("caldav", None, Some(caldav)).await.unwrap();
        let lists = store.lists().await.unwrap();
        assert_eq!(lists.len(), 1);
        assert!(lists[0].is_default);
        assert!(store.tasks(&TaskQuery::default()).await.unwrap().is_empty());
    }

    fn links_lowercase(mut links: Links) -> Links {
        if let Some(p) = links.person.as_mut() {
            p.email = p.email.to_lowercase();
        }
        links
    }

    #[tokio::test]
    async fn havre_lists_and_sync_choices() {
        let db = Database::in_memory().await.unwrap();
        let store = db.tasks();
        let home = store.create_local_list("Home").await.unwrap();
        assert_eq!(store.lists().await.unwrap().len(), 2);
        store.update_list(home.id, Some("Maison"), Some("#A9C79C"), Some(true)).await.unwrap();
        let home = store.list_row(home.id).await.unwrap().list;
        assert_eq!((home.name.as_str(), home.color.as_str(), home.is_default), ("Maison", "#a9c79c", true));
        let inbox = store.lists().await.unwrap().into_iter().find(|l| l.name == "Inbox").unwrap();
        assert!(!inbox.is_default);
        store.delete_local_list(home.id).await.unwrap();
        assert!(store.delete_local_list(inbox.id).await.is_err(), "the last Havre list stays");
        assert!(store.list_row(inbox.id).await.unwrap().list.is_default);

        assert!(store.sync_enabled("account:1").await.unwrap());
        assert!(!store.sync_enabled("caldav:1").await.unwrap());
        store.set_sync_enabled("account:1", false).await.unwrap();
        assert!(!store.sync_enabled("account:1").await.unwrap());
    }
}
