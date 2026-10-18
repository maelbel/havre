//! The Agenda's tables: calendars, events, CalDAV servers.

use serde::Serialize;
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use super::{Attendee, COLORS, Calendar, Event, RemoteCalendar, RemoteEvent};
use crate::{Error, Result, secrets::Secrets};

#[derive(Clone)]
pub struct AgendaStore {
    pool: SqlitePool,
    secrets: Secrets,
}

/// A calendar plus what syncing needs.
#[derive(Debug, Clone)]
pub(crate) struct CalendarRow {
    pub calendar: Calendar,
    pub remote_id: String,
    pub sync_from: Option<i64>,
    pub sync_to: Option<i64>,
}

/// An event plus what writing it back needs.
#[derive(Debug, Clone)]
pub(crate) struct EventRow {
    pub event: Event,
    pub remote_id: String,
    pub href: Option<String>,
    pub etag: Option<String>,
    pub raw: Option<String>,
}

/// A CalDAV server, as listed in Settings (no password).
#[derive(Debug, Clone, Serialize)]
pub struct CalDavAccount {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub username: String,
}

pub(crate) struct CalDavLogin {
    pub url: String,
    pub username: String,
    pub password: String,
}

const CALENDAR_COLUMNS: &str = "id, source, account_id, caldav_id, remote_id, name, color, read_only, enabled, \
                                is_default, sync_from, sync_to, synced_at, error";

impl AgendaStore {
    pub(crate) fn new(pool: SqlitePool, secrets: Secrets) -> Self {
        Self { pool, secrets }
    }

    // ---- Calendars ----------------------------------------------------------------

    pub async fn calendars(&self) -> Result<Vec<Calendar>> {
        Ok(self.calendar_rows().await?.into_iter().map(|r| r.calendar).collect())
    }

    pub(crate) async fn calendar_rows(&self) -> Result<Vec<CalendarRow>> {
        let rows = sqlx::query(&format!(
            "SELECT {CALENDAR_COLUMNS} FROM calendars
             ORDER BY source = 'local' DESC, account_id, caldav_id, is_default DESC, name COLLATE NOCASE"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(calendar_from_row).collect())
    }

    pub(crate) async fn calendar_row(&self, id: i64) -> Result<CalendarRow> {
        let row = sqlx::query(&format!("SELECT {CALENDAR_COLUMNS} FROM calendars WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("calendar {id}")))?;
        Ok(calendar_from_row(&row))
    }

    /// Makes the stored calendars of one source match what it lists.
    pub(crate) async fn replace_calendars(
        &self,
        source: &str,
        account: Option<i64>,
        caldav: Option<i64>,
        remotes: &[RemoteCalendar],
    ) -> Result<()> {
        let existing: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, remote_id FROM calendars WHERE account_id IS ? AND caldav_id IS ? AND source = ?",
        )
        .bind(account)
        .bind(caldav)
        .bind(source)
        .fetch_all(&self.pool)
        .await?;
        let has_default: bool = sqlx::query_scalar(
            "SELECT COUNT(*) > 0 FROM calendars WHERE is_default = 1 AND source <> 'local'",
        )
        .fetch_one(&self.pool)
        .await?;
        let mut tx = self.pool.begin().await?;
        for (i, remote) in remotes.iter().enumerate() {
            match existing.iter().find(|(_, r)| *r == remote.remote_id) {
                Some((id, _)) => {
                    sqlx::query("UPDATE calendars SET name = ?, read_only = ? WHERE id = ?")
                        .bind(&remote.name)
                        .bind(remote.read_only)
                        .bind(id)
                        .execute(&mut *tx)
                        .await?;
                }
                None => {
                    let color = remote.color.clone().unwrap_or_else(|| COLORS[i % COLORS.len()].to_owned());
                    // The first account's main calendar becomes the default
                    // for new events (instead of Havre's local one).
                    let default = remote.is_default && !has_default && !remote.read_only;
                    if default {
                        sqlx::query("UPDATE calendars SET is_default = 0").execute(&mut *tx).await?;
                    }
                    sqlx::query(
                        "INSERT INTO calendars (source, account_id, caldav_id, remote_id, name, color, read_only, is_default)
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
                sqlx::query("DELETE FROM calendars WHERE id = ?").bind(id).execute(&mut *tx).await?;
            }
        }
        tx.commit().await?;
        self.ensure_default().await
    }

    /// Exactly one calendar receives new events: Havre's own if needed.
    async fn ensure_default(&self) -> Result<()> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM calendars WHERE is_default = 1")
            .fetch_one(&self.pool)
            .await?;
        if count == 0 {
            sqlx::query("UPDATE calendars SET is_default = 1 WHERE source = 'local'")
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// Show/hide, recolor, make default.
    pub async fn update_calendar(
        &self,
        id: i64,
        enabled: Option<bool>,
        color: Option<&str>,
        is_default: Option<bool>,
    ) -> Result<Calendar> {
        let row = self.calendar_row(id).await?;
        if let Some(enabled) = enabled {
            sqlx::query("UPDATE calendars SET enabled = ? WHERE id = ?")
                .bind(enabled)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        if let Some(color) = color {
            let valid = color.len() == 7
                && color.starts_with('#')
                && color[1..].chars().all(|c| c.is_ascii_hexdigit());
            if !valid {
                return Err(Error::Config("colors look like #e3bc6a".into()));
            }
            sqlx::query("UPDATE calendars SET color = ? WHERE id = ?")
                .bind(color)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        if is_default == Some(true) {
            if row.calendar.read_only {
                return Err(Error::Config("New events can't go into a read-only calendar.".into()));
            }
            sqlx::query("UPDATE calendars SET is_default = (id = ?)").bind(id).execute(&self.pool).await?;
        }
        Ok(self.calendar_row(id).await?.calendar)
    }

    pub(crate) async fn set_error(&self, calendar: i64, error: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE calendars SET error = ? WHERE id = ?")
            .bind(error)
            .bind(calendar)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---- Events ---------------------------------------------------------------------

    /// Events of the shown calendars overlapping `[from, to)`.
    pub async fn events(&self, from: i64, to: i64) -> Result<Vec<Event>> {
        let rows = sqlx::query(
            "SELECT e.* FROM events e JOIN calendars c ON c.id = e.calendar_id
             WHERE c.enabled = 1 AND e.start_ts < ? AND e.end_ts > ?
             ORDER BY e.all_day DESC, e.start_ts, e.end_ts DESC",
        )
        .bind(to)
        .bind(from)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| event_from_row(r).event).collect())
    }

    pub(crate) async fn event_row(&self, id: i64) -> Result<EventRow> {
        let row = sqlx::query("SELECT * FROM events WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("event {id}")))?;
        Ok(event_from_row(&row))
    }

    pub async fn event(&self, id: i64) -> Result<Event> {
        Ok(self.event_row(id).await?.event)
    }

    /// After fetching `[from, to)` from the provider: keeps local ids for
    /// events that are still there, drops those that are gone.
    pub(crate) async fn replace_window(
        &self,
        calendar: i64,
        from: i64,
        to: i64,
        remotes: &[RemoteEvent],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("CREATE TEMP TABLE IF NOT EXISTS seen (remote_id TEXT PRIMARY KEY)")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM seen").execute(&mut *tx).await?;
        for remote in remotes {
            upsert(&mut tx, calendar, remote).await?;
            sqlx::query("INSERT OR IGNORE INTO seen VALUES (?)")
                .bind(&remote.remote_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query(
            "DELETE FROM events WHERE calendar_id = ? AND start_ts < ? AND end_ts > ?
             AND remote_id NOT IN (SELECT remote_id FROM seen)",
        )
        .bind(calendar)
        .bind(to)
        .bind(from)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE calendars SET sync_from = MIN(COALESCE(sync_from, ?1), ?1), sync_to = MAX(COALESCE(sync_to, ?2), ?2),
                                  synced_at = ?3, error = NULL WHERE id = ?4",
        )
        .bind(from)
        .bind(to)
        .bind(now())
        .bind(calendar)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Saves one event (new or changed); returns it with its local id.
    pub(crate) async fn save_event(&self, calendar: i64, remote: &RemoteEvent) -> Result<Event> {
        let mut tx = self.pool.begin().await?;
        let id = upsert(&mut tx, calendar, remote).await?;
        tx.commit().await?;
        self.event(id).await
    }

    /// Events with this iCalendar UID (a meeting already in a calendar).
    pub async fn events_by_uid(&self, uid: &str) -> Result<Vec<Event>> {
        // The meeting itself, or your copy of it (see `remember_copy`).
        let rows = sqlx::query(
            "SELECT e.* FROM events e WHERE e.uid = ?1 OR EXISTS (
                 SELECT 1 FROM invitation_copies c
                 WHERE c.uid = ?1 AND c.calendar_id = e.calendar_id AND c.remote_id = e.remote_id)
             ORDER BY e.start_ts",
        )
        .bind(uid)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| event_from_row(r).event).collect())
    }

    /// Upcoming meetings you haven't answered yet (shown calendars only).
    pub async fn pending_invitations(&self, from: i64) -> Result<Vec<Event>> {
        let rows = sqlx::query(
            "SELECT e.* FROM events e JOIN calendars c ON c.id = e.calendar_id
             WHERE c.enabled = 1 AND e.my_response = 'needs_action' AND e.end_ts > ?
             ORDER BY e.start_ts LIMIT 50",
        )
        .bind(from)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| event_from_row(r).event).collect())
    }

    /// `id` is your copy of the meeting `uid` (an invitation you added):
    /// remembered by the provider's id, which survives syncs.
    pub(crate) async fn remember_copy(&self, id: i64, uid: &str, organizer: Option<&str>) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO invitation_copies (uid, calendar_id, remote_id, organizer)
             SELECT ?, calendar_id, remote_id, lower(?) FROM events WHERE id = ?",
        )
        .bind(uid)
        .bind(organizer)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Your answer to the invitation with this UID, if you gave one.
    pub async fn answer(&self, uid: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT response FROM invitation_answers WHERE uid = ?")
            .bind(uid)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub(crate) async fn remember_answer(&self, uid: &str, response: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO invitation_answers (uid, response, answered_at) VALUES (?, ?, ?)
             ON CONFLICT (uid) DO UPDATE SET response = excluded.response, answered_at = excluded.answered_at",
        )
        .bind(uid)
        .bind(response)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// After answering an invitation.
    pub(crate) async fn set_my_response(&self, id: i64, response: &str) -> Result<()> {
        sqlx::query("UPDATE events SET my_response = ? WHERE id = ?")
            .bind(response)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Events with this person (as guest or organizer), latest first.
    pub async fn events_with(
        &self,
        email: &str,
        upcoming_from: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Event>> {
        let email = email.to_lowercase();
        let (filter, order) = match upcoming_from {
            Some(_) => ("e.end_ts > ?2", "e.start_ts ASC"),
            None => ("e.end_ts <= ?2", "e.start_ts DESC"),
        };
        let rows = sqlx::query(&format!(
            "SELECT e.* FROM events e JOIN calendars c ON c.id = e.calendar_id
             WHERE c.enabled = 1 AND {filter}
               AND (EXISTS (SELECT 1 FROM json_each(e.attendees) a WHERE lower(json_extract(a.value, '$.email')) = ?1)
                    OR lower(e.organizer) LIKE '%' || ?1 || '%'
                    OR EXISTS (SELECT 1 FROM invitation_copies k WHERE k.organizer = ?1
                               AND k.calendar_id = e.calendar_id AND k.remote_id = e.remote_id))
             ORDER BY {order} LIMIT ?3"
        ))
        .bind(&email)
        .bind(upcoming_from.unwrap_or_else(now))
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| event_from_row(r).event).collect())
    }

    /// Replaces an event whose provider id changed (moved to another calendar…).
    pub(crate) async fn delete_event(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM events WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    // ---- CalDAV servers --------------------------------------------------------------

    pub async fn caldav_accounts(&self) -> Result<Vec<CalDavAccount>> {
        let rows: Vec<(i64, String, String, String)> =
            sqlx::query_as("SELECT id, name, url, username FROM caldav_accounts ORDER BY id")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(id, name, url, username)| CalDavAccount { id, name, url, username })
            .collect())
    }

    pub(crate) async fn insert_caldav(
        &self,
        name: &str,
        url: &str,
        username: &str,
        password: &str,
    ) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "INSERT INTO caldav_accounts (name, url, username, password, created_at) VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(name)
        .bind(url)
        .bind(username)
        .bind(self.secrets.encrypt(password)?)
        .bind(now())
        .fetch_one(&self.pool)
        .await?)
    }

    pub(crate) async fn caldav_login(&self, id: i64) -> Result<CalDavLogin> {
        let (url, username, password): (String, String, Vec<u8>) =
            sqlx::query_as("SELECT url, username, password FROM caldav_accounts WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .ok_or_else(|| Error::NotFound(format!("CalDAV account {id}")))?;
        Ok(CalDavLogin { url, username, password: self.secrets.decrypt(&password)? })
    }

    pub async fn delete_caldav(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM caldav_accounts WHERE id = ?").bind(id).execute(&self.pool).await?;
        self.ensure_default().await
    }

    /// When an account loses calendar access, its calendars go.
    pub(crate) async fn remove_account_calendars(&self, account: i64) -> Result<()> {
        sqlx::query("DELETE FROM calendars WHERE account_id = ?").bind(account).execute(&self.pool).await?;
        self.ensure_default().await
    }
}

async fn upsert(tx: &mut sqlx::SqliteConnection, calendar: i64, e: &RemoteEvent) -> Result<i64> {
    let attendees = serde_json::to_string(&e.attendees).unwrap_or_else(|_| "[]".into());
    Ok(sqlx::query_scalar(
        "INSERT INTO events (calendar_id, remote_id, href, etag, raw, title, location, description, start_ts, end_ts,
                             all_day, recurring, read_only, organizer, attendees, join_url, uid, my_response)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (calendar_id, remote_id) DO UPDATE SET
            href = excluded.href, etag = excluded.etag, raw = excluded.raw, title = excluded.title,
            location = excluded.location, description = excluded.description, start_ts = excluded.start_ts,
            end_ts = excluded.end_ts, all_day = excluded.all_day, recurring = excluded.recurring,
            read_only = excluded.read_only, organizer = excluded.organizer, attendees = excluded.attendees,
            join_url = excluded.join_url, uid = excluded.uid, my_response = excluded.my_response
         RETURNING id",
    )
    .bind(calendar)
    .bind(&e.remote_id)
    .bind(&e.href)
    .bind(&e.etag)
    .bind(&e.raw)
    .bind(&e.title)
    .bind(&e.location)
    .bind(&e.description)
    .bind(e.start)
    .bind(e.end)
    .bind(e.all_day)
    .bind(e.recurring)
    .bind(e.read_only)
    .bind(&e.organizer)
    .bind(attendees)
    .bind(&e.join_url)
    .bind(&e.uid)
    .bind(&e.my_response)
    .fetch_one(&mut *tx)
    .await?)
}

fn calendar_from_row(r: &SqliteRow) -> CalendarRow {
    CalendarRow {
        calendar: Calendar {
            id: r.get("id"),
            source: r.get("source"),
            account: r.get("account_id"),
            caldav: r.get("caldav_id"),
            name: r.get("name"),
            color: r.get("color"),
            read_only: r.get("read_only"),
            enabled: r.get("enabled"),
            is_default: r.get("is_default"),
            synced_at: r.get("synced_at"),
            error: r.get("error"),
        },
        remote_id: r.get("remote_id"),
        sync_from: r.get("sync_from"),
        sync_to: r.get("sync_to"),
    }
}

fn event_from_row(r: &SqliteRow) -> EventRow {
    let attendees: Vec<Attendee> = serde_json::from_str(r.get("attendees")).unwrap_or_default();
    EventRow {
        event: Event {
            id: r.get("id"),
            calendar: r.get("calendar_id"),
            title: r.get("title"),
            location: r.get("location"),
            description: r.get("description"),
            start: r.get("start_ts"),
            end: r.get("end_ts"),
            all_day: r.get("all_day"),
            recurring: r.get("recurring"),
            read_only: r.get("read_only"),
            organizer: r.get("organizer"),
            attendees,
            join_url: r.get("join_url"),
            uid: r.get("uid"),
            my_response: r.get("my_response"),
        },
        remote_id: r.get("remote_id"),
        href: r.get("href"),
        etag: r.get("etag"),
        raw: r.get("raw"),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}
