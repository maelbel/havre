//! Backups: a copy of Havre's database, made every night (or when you ask),
//! the last few kept.
//!
//! What a backup is: one SQLite file, `havre-2026-10-15-0300.db`. Open it
//! with any SQLite tool if you're curious. It holds your accounts, labels,
//! drafts, the Havre calendar, settings… Two things are left out:
//!
//! - the **mail cache** (downloaded messages, the search-by-meaning index):
//!   it's a copy of what's on your mail servers, Havre downloads it again;
//! - the **encryption key** (`secret.key`). Mail passwords and sign-ins in
//!   the database are encrypted with it, so a stolen backup alone gives them
//!   away to no one. Keep the key somewhere safe (Settings › Backups shows it
//!   once you type your password): restoring on a new server needs it.
//!
//! How: `VACUUM INTO` writes a consistent copy while Havre keeps running
//! (SQLite reads a snapshot), then the copy is trimmed and checked.
//!
//! **Off-site copies** (step 18): each backup can also be sealed (encrypted,
//! `sealed.rs`) and sent to a WebDAV folder or an S3 bucket (`remote.rs`),
//! the last few kept there. A copy that fails is tried again later.
//!
//! What it teaches: SQLite's online copy, files and permissions, a
//! scheduler loop, and restoring safely (keep the old file until the new
//! one is known good).

pub mod remote;
pub mod sealed;
mod sigv4;

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use sqlx::{
    Row,
    sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions},
};

use crate::{Error, Result, secrets::Secrets, store::Database};
use remote::{Kind, Remote, RemoteFile};

/// Settings › Backups.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupSettings {
    /// Every night, at `time`.
    pub enabled: bool,
    /// "03:00", the server's local time.
    pub time: String,
    /// How many backups to keep (older ones are deleted).
    pub keep: u32,
}

/// One backup file.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BackupFile {
    pub name: String,
    pub size: u64,
    /// When it was made (unix seconds).
    pub created_at: i64,
}

/// What the Backups page and the App library's status card show.
#[derive(Debug, Clone, Serialize)]
pub struct BackupStatus {
    pub settings: BackupSettings,
    /// Where backups are written (`HAVRE_BACKUP_DIR`).
    pub dir: String,
    pub backups: Vec<BackupFile>,
    pub last_at: Option<i64>,
    pub last_error: Option<String>,
    /// The next scheduled one (unix seconds), if on.
    pub next_at: Option<i64>,
    /// Which encryption key restoring needs (a fingerprint, not the key).
    pub key_fingerprint: String,
    /// Off-site copies.
    pub remote: RemoteStatus,
}

/// Settings › Backups › Off-site copy. The secret goes one way only: the page
/// may send a new one (`secret`), but only learns whether one is saved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteSettings {
    pub enabled: bool,
    pub kind: Kind,
    pub url: String,
    #[serde(default)]
    pub bucket: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default = "yes")]
    pub path_style: bool,
    #[serde(default)]
    pub username: String,
    /// A new password / secret key; `None` or empty keeps the saved one.
    #[serde(default, skip_serializing)]
    pub secret: Option<String>,
    #[serde(default)]
    pub has_secret: bool,
    pub keep: u32,
}

fn yes() -> bool {
    true
}

/// What the page shows about off-site copies.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteStatus {
    pub settings: RemoteSettings,
    pub last_at: Option<i64>,
    pub last_name: Option<String>,
    pub last_size: Option<i64>,
    pub last_error: Option<String>,
    /// Copies have been failing since then (unix seconds).
    pub failing_since: Option<i64>,
    /// A local backup waiting to be sent, and when Havre tries next.
    pub pending: Option<String>,
    pub next_try_at: Option<i64>,
    /// On, and failing for 24 hours or more (the status card turns orange).
    pub failing_for_a_day: bool,
}

/// Backups of one Havre: the database, the folder to write to, the key.
#[derive(Clone)]
pub struct Backups {
    db: Database,
    dir: PathBuf,
    secrets: Secrets,
    /// One off-site copy at a time (the scheduler and "Copy off-site").
    sending: std::sync::Arc<tokio::sync::Mutex<()>>,
}

/// The name of a backup's off-site copy: `havre-….db` → `havre-….havre`.
pub fn copy_name(backup: &str) -> String {
    format!("{}.{}", backup.trim_end_matches(".db"), sealed::EXTENSION)
}

/// File names Havre writes, and the only ones it serves or deletes.
pub fn is_backup_name(name: &str) -> bool {
    name.starts_with("havre-")
        && name.ends_with(".db")
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !name.contains("..")
}

impl Backups {
    pub fn new(db: Database, dir: PathBuf, secrets: Secrets) -> Self {
        Self { db, dir, secrets, sending: Default::default() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub async fn settings(&self) -> Result<BackupSettings> {
        let row = sqlx::query("SELECT enabled, time, keep FROM backup_settings WHERE id = 1")
            .fetch_one(&self.db.pool())
            .await?;
        Ok(BackupSettings {
            enabled: row.get("enabled"),
            time: row.get("time"),
            keep: row.get::<i64, _>("keep").clamp(1, 365) as u32,
        })
    }

    pub async fn save_settings(&self, s: &BackupSettings) -> Result<BackupSettings> {
        if NaiveTime::parse_from_str(&s.time, "%H:%M").is_err() {
            return Err(Error::Config("The time looks like 03:00.".into()));
        }
        if !(1..=365).contains(&s.keep) {
            return Err(Error::Config("Keep between 1 and 365 backups.".into()));
        }
        sqlx::query("UPDATE backup_settings SET enabled = ?, time = ?, keep = ? WHERE id = 1")
            .bind(s.enabled)
            .bind(&s.time)
            .bind(s.keep as i64)
            .execute(&self.db.pool())
            .await?;
        self.settings().await
    }

    /// The backups in the folder, newest first.
    pub fn list(&self) -> Result<Vec<BackupFile>> {
        let mut files = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(files),
            Err(err) => return Err(err.into()),
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_backup_name(&name) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            files.push(BackupFile { name, size: meta.len(), created_at: modified_at(&meta) });
        }
        files.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.name.cmp(&a.name)));
        Ok(files)
    }

    /// The full path of a backup, if `name` is one (never anything else).
    pub fn path_of(&self, name: &str) -> Result<PathBuf> {
        if !is_backup_name(name) {
            return Err(Error::NotFound(format!("backup {name}")));
        }
        let path = self.dir.join(name);
        if !path.is_file() {
            return Err(Error::NotFound(format!("backup {name}")));
        }
        Ok(path)
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        std::fs::remove_file(self.path_of(name)?)?;
        Ok(())
    }

    pub async fn status(&self) -> Result<BackupStatus> {
        let settings = self.settings().await?;
        let row = sqlx::query("SELECT last_at, last_error FROM backup_settings WHERE id = 1")
            .fetch_one(&self.db.pool())
            .await?;
        let last_at: Option<i64> = row.get("last_at");
        let next_at =
            settings.enabled.then(|| next_run(&settings.time, last_at, Local::now().timestamp())).flatten();
        Ok(BackupStatus {
            dir: self.dir.display().to_string(),
            backups: self.list()?,
            last_at,
            last_error: row.get("last_error"),
            next_at,
            key_fingerprint: self.secrets.fingerprint(),
            remote: self.remote_status().await?,
            settings,
        })
    }

    /// Makes a backup now. Returns the new file.
    pub async fn run(&self) -> Result<BackupFile> {
        let result = self.make().await;
        let (at, error) = match &result {
            Ok(_) => (Some(chrono::Utc::now().timestamp()), None),
            Err(err) => (None, Some(err.to_string())),
        };
        sqlx::query("UPDATE backup_settings SET last_at = COALESCE(?, last_at), last_error = ? WHERE id = 1")
            .bind(at)
            .bind(error)
            .execute(&self.db.pool())
            .await?;
        if let Ok(file) = &result {
            // Off-site: the scheduler sends it within a minute (a newer backup
            // replaces one still waiting).
            sqlx::query("UPDATE backup_remote SET pending = ?, next_try_at = ? WHERE id = 1 AND enabled = 1")
                .bind(&file.name)
                .bind(chrono::Utc::now().timestamp())
                .execute(&self.db.pool())
                .await?;
            let keep = self.settings().await?.keep as usize;
            let pending = self.remote_status().await?.pending;
            for old in self.list()?.into_iter().skip(keep) {
                // Not the one still waiting to be sent off-site.
                if pending.as_deref() == Some(&old.name) {
                    continue;
                }
                if let Err(err) = std::fs::remove_file(self.dir.join(&old.name)) {
                    tracing::warn!(%err, file = old.name, "removing an old backup");
                }
            }
        }
        result
    }

    // ---- Off-site copies ---------------------------------------------------------------

    pub async fn remote_settings(&self) -> Result<RemoteSettings> {
        let row = sqlx::query(
            "SELECT enabled, kind, url, bucket, region, prefix, path_style, username, secret IS NOT NULL AS has_secret, keep \
             FROM backup_remote WHERE id = 1",
        )
        .fetch_one(&self.db.pool())
        .await?;
        Ok(RemoteSettings {
            enabled: row.get("enabled"),
            kind: Kind::parse(row.get("kind")),
            url: row.get("url"),
            bucket: row.get("bucket"),
            region: row.get("region"),
            prefix: row.get("prefix"),
            path_style: row.get("path_style"),
            username: row.get("username"),
            secret: None,
            has_secret: row.get("has_secret"),
            keep: row.get::<i64, _>("keep").clamp(1, 365) as u32,
        })
    }

    pub async fn remote_status(&self) -> Result<RemoteStatus> {
        let row = sqlx::query(
            "SELECT last_at, last_name, last_size, last_error, failing_since, pending, next_try_at FROM backup_remote WHERE id = 1",
        )
        .fetch_one(&self.db.pool())
        .await?;
        let settings = self.remote_settings().await?;
        let failing_since: Option<i64> = row.get("failing_since");
        let failing_for_a_day =
            settings.enabled && failing_since.is_some_and(|t| chrono::Utc::now().timestamp() - t >= 86_400);
        Ok(RemoteStatus {
            failing_for_a_day,
            settings,
            last_at: row.get("last_at"),
            last_name: row.get("last_name"),
            last_size: row.get("last_size"),
            last_error: row.get("last_error"),
            failing_since: row.get("failing_since"),
            pending: row.get("pending"),
            next_try_at: row.get("next_try_at"),
        })
    }

    /// The destination these settings describe, with the secret to use: the
    /// one typed (if any), else the saved one.
    async fn remote_from(&self, s: &RemoteSettings) -> Result<Remote> {
        let url = remote::check_url(&s.url)?;
        if s.kind == Kind::S3 && s.bucket.trim().is_empty() {
            return Err(Error::Config("Give the bucket's name.".into()));
        }
        if s.username.trim().is_empty() {
            return Err(Error::Config(match s.kind {
                Kind::Webdav => "Give the user name.".into(),
                Kind::S3 => "Give the access key (its ID).".into(),
            }));
        }
        let secret = match s.secret.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            Some(typed) => typed.to_string(),
            None => {
                let saved: Option<Vec<u8>> =
                    sqlx::query_scalar("SELECT secret FROM backup_remote WHERE id = 1")
                        .fetch_one(&self.db.pool())
                        .await?;
                match saved {
                    Some(bytes) => self.secrets.decrypt(&bytes)?,
                    None => {
                        return Err(Error::Config(match s.kind {
                            Kind::Webdav => "Give the password (for Nextcloud, an app password).".into(),
                            Kind::S3 => "Give the secret key.".into(),
                        }));
                    }
                }
            }
        };
        Ok(Remote {
            kind: s.kind,
            url,
            bucket: s.bucket.trim().to_string(),
            region: s.region.trim().to_string(),
            prefix: s.prefix.trim().to_string(),
            path_style: s.path_style,
            username: s.username.trim().to_string(),
            secret,
        })
    }

    /// Saves the off-site settings (checked first when turned on).
    pub async fn save_remote(&self, s: &RemoteSettings) -> Result<RemoteStatus> {
        if !(1..=365).contains(&s.keep) {
            return Err(Error::Config("Keep between 1 and 365 copies.".into()));
        }
        if s.enabled {
            self.remote_from(s).await?;
        } else if !s.url.trim().is_empty() {
            remote::check_url(&s.url)?;
        }
        let secret = match s.secret.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            Some(typed) => Some(self.secrets.encrypt(typed)?),
            None => None,
        };
        sqlx::query(
            "UPDATE backup_remote SET enabled = ?, kind = ?, url = ?, bucket = ?, region = ?, prefix = ?, path_style = ?, \
             username = ?, keep = ?, secret = COALESCE(?, secret) WHERE id = 1",
        )
        .bind(s.enabled)
        .bind(s.kind.as_str())
        .bind(s.url.trim())
        .bind(s.bucket.trim())
        .bind(s.region.trim())
        .bind(s.prefix.trim())
        .bind(s.path_style)
        .bind(s.username.trim())
        .bind(s.keep as i64)
        .bind(secret)
        .execute(&self.db.pool())
        .await?;
        if !s.enabled {
            sqlx::query("UPDATE backup_remote SET pending = NULL, next_try_at = NULL WHERE id = 1")
                .execute(&self.db.pool())
                .await?;
        }
        self.remote_status().await
    }

    /// Turns off-site copies off and forgets the destination and its secret
    /// (the copies already sent stay where they are).
    pub async fn forget_remote(&self) -> Result<RemoteStatus> {
        sqlx::query("DELETE FROM backup_remote").execute(&self.db.pool()).await?;
        sqlx::query("INSERT INTO backup_remote (id) VALUES (1)").execute(&self.db.pool()).await?;
        self.remote_status().await
    }

    /// Tries a destination (the typed settings, else the saved secret).
    pub async fn test_remote(&self, s: &RemoteSettings) -> Result<String> {
        let remote = self.remote_from(s).await?;
        remote.test(&remote::client()?).await
    }

    /// Seals one local backup and sends it now; then trims old copies there.
    pub async fn send_offsite(&self, name: &str) -> Result<RemoteFile> {
        let _one_at_a_time = self.sending.lock().await;
        let result = self.send_locked(name).await;
        let now = chrono::Utc::now().timestamp();
        match &result {
            Ok(file) => {
                sqlx::query(
                    "UPDATE backup_remote SET last_at = ?, last_name = ?, last_size = ?, last_error = NULL, \
                     failing_since = NULL, pending = CASE WHEN pending = ? THEN NULL ELSE pending END, \
                     next_try_at = CASE WHEN pending = ? THEN NULL ELSE next_try_at END WHERE id = 1",
                )
                .bind(now)
                .bind(&file.name)
                .bind(file.size as i64)
                .bind(name)
                .bind(name)
                .execute(&self.db.pool())
                .await?;
                tracing::info!(copy = file.name, size = file.size, "off-site copy sent");
            }
            Err(err) => {
                // Every 15 minutes during the first hour of failures, then hourly.
                let since: Option<i64> =
                    sqlx::query_scalar("SELECT failing_since FROM backup_remote WHERE id = 1")
                        .fetch_one(&self.db.pool())
                        .await?;
                let wait = if now - since.unwrap_or(now) < 3600 { 15 * 60 } else { 3600 };
                sqlx::query(
                    "UPDATE backup_remote SET last_error = ?, failing_since = COALESCE(failing_since, ?), \
                     next_try_at = CASE WHEN pending IS NOT NULL THEN ? ELSE NULL END WHERE id = 1",
                )
                // The sentence itself, without "configuration error:" in front.
                .bind(match err {
                    Error::Config(message) => message.clone(),
                    other => other.to_string(),
                })
                .bind(now)
                .bind(now + wait)
                .execute(&self.db.pool())
                .await?;
                tracing::warn!(%err, backup = name, "off-site copy failed");
            }
        }
        result
    }

    async fn send_locked(&self, name: &str) -> Result<RemoteFile> {
        let settings = self.remote_settings().await?;
        let remote = self.remote_from(&settings).await?;
        let local = self.path_of(name)?;
        let copy = copy_name(name);
        // Sealed next to the backups, under a hidden name, removed afterwards.
        let sealed_path = self.dir.join(format!(".offsite-{copy}"));
        let created_at = std::fs::metadata(&local).map(|m| modified_at(&m)).unwrap_or_default();
        let secrets = self.secrets.clone();
        let (src, dst) = (local.clone(), sealed_path.clone());
        tokio::task::spawn_blocking(move || sealed::seal(&src, &dst, &secrets, created_at))
            .await
            .map_err(|e| Error::Config(format!("Sealing the backup stopped: {e}")))??;
        let result = async {
            let bytes = tokio::fs::read(&sealed_path).await?;
            let size = bytes.len() as u64;
            let http = remote::client()?;
            remote.send(&http, &copy, bytes).await?;
            // Keep the newest `keep` copies there (names sort by date).
            let copies = remote.list(&http).await?;
            let extra = copies.len().saturating_sub(settings.keep as usize);
            for old in copies.iter().take(extra) {
                if let Err(err) = remote.delete(&http, &old.name).await {
                    tracing::warn!(%err, copy = old.name, "removing an old off-site copy");
                }
            }
            Ok(RemoteFile { name: copy.clone(), size })
        }
        .await;
        let _ = std::fs::remove_file(&sealed_path);
        result
    }

    /// The scheduler's part: sends the waiting backup when it's time.
    async fn send_pending(&self) -> Result<()> {
        let row = sqlx::query("SELECT enabled, pending, next_try_at FROM backup_remote WHERE id = 1")
            .fetch_one(&self.db.pool())
            .await?;
        let (enabled, pending, next): (bool, Option<String>, Option<i64>) =
            (row.get("enabled"), row.get("pending"), row.get("next_try_at"));
        let now = chrono::Utc::now().timestamp();
        if let (true, Some(name)) = (enabled, pending)
            && next.is_none_or(|at| at <= now)
        {
            if self.path_of(&name).map(|p| p.exists()).unwrap_or(false) {
                let _ = self.send_offsite(&name).await;
            } else {
                // The backup was deleted meanwhile: nothing left to send.
                sqlx::query("UPDATE backup_remote SET pending = NULL, next_try_at = NULL WHERE id = 1")
                    .execute(&self.db.pool())
                    .await?;
            }
        }
        Ok(())
    }

    async fn make(&self) -> Result<BackupFile> {
        std::fs::create_dir_all(&self.dir).map_err(|e| {
            Error::Config(format!("Can't create the backup folder {}: {e}", self.dir.display()))
        })?;
        let name = format!("havre-{}.db", Local::now().format("%Y-%m-%d-%H%M%S"));
        let path = self.dir.join(&name);
        // Written under a temporary name: a half-made file is never taken
        // for a backup.
        let partial = self.dir.join(format!(".{name}.partial"));
        let _ = std::fs::remove_file(&partial);

        // 1. A consistent copy, while Havre keeps running.
        sqlx::query("VACUUM INTO ?")
            .bind(partial.to_string_lossy().into_owned())
            .execute(&self.db.pool())
            .await?;

        // 2. Leave out what Havre can download again, note what the backup
        //    needs, and make the file compact.
        let copy = open_file(&partial, false).await?;
        for table in ["bodies", "mail_vectors", "ai_cache", "sessions"] {
            sqlx::query(sql!("DELETE FROM {table}")).execute(&copy).await?;
        }
        sqlx::query("CREATE TABLE havre_backup (created_at INTEGER NOT NULL, version TEXT NOT NULL, key_fingerprint TEXT NOT NULL)")
            .execute(&copy)
            .await?;
        sqlx::query("INSERT INTO havre_backup VALUES (?, ?, ?)")
            .bind(chrono::Utc::now().timestamp())
            .bind(env!("CARGO_PKG_VERSION"))
            .bind(self.secrets.fingerprint())
            .execute(&copy)
            .await?;
        sqlx::query("VACUUM").execute(&copy).await?;
        copy.close().await;

        // 3. Check it before calling it a backup.
        check(&partial).await?;
        crate::secrets::restrict_permissions(&partial);
        std::fs::rename(&partial, &path)?;
        let meta = std::fs::metadata(&path)?;
        let size = meta.len();
        tracing::info!(file = %path.display(), size, "backup made");
        // The file's own time, as `list()` reads it: "now" could already be a
        // second later, and the two would disagree about the same backup.
        Ok(BackupFile { name, size, created_at: modified_at(&meta) })
    }

    /// Runs forever: makes the nightly backup when it's time.
    pub async fn scheduler(self) {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let due = async {
                let settings = self.settings().await?;
                if !settings.enabled {
                    return Ok::<bool, Error>(false);
                }
                let last: Option<i64> =
                    sqlx::query_scalar("SELECT last_at FROM backup_settings WHERE id = 1")
                        .fetch_one(&self.db.pool())
                        .await?;
                let now = Local::now().timestamp();
                Ok(next_run(&settings.time, last, now).is_some_and(|at| at <= now))
            };
            match due.await {
                Ok(true) => {
                    if let Err(err) = self.run().await {
                        tracing::warn!(%err, "nightly backup failed");
                    }
                }
                Ok(false) => {}
                Err(err) => tracing::warn!(%err, "checking the backup schedule"),
            }
            if let Err(err) = self.send_pending().await {
                tracing::warn!(%err, "checking off-site copies");
            }
        }
    }
}

/// When the next scheduled backup is due: the latest `time` slot (today or
/// yesterday) not covered by `last_at`, else tomorrow's.
pub fn next_run(time: &str, last_at: Option<i64>, now: i64) -> Option<i64> {
    let t = NaiveTime::parse_from_str(time, "%H:%M").ok()?;
    let now_local = Local.timestamp_opt(now, 0).single()?;
    let today = now_local.date_naive();
    let slot = |day: chrono::NaiveDate| {
        Local.from_local_datetime(&day.and_time(t)).earliest().map(|d| d.timestamp())
    };
    let today_slot = slot(today)?;
    let latest = if today_slot <= now { today_slot } else { slot(today.pred_opt()?)? };
    match last_at {
        // Already done since the latest slot: the next one.
        Some(last) if last >= latest => {
            slot(Local.timestamp_opt(latest, 0).single()?.date_naive().succ_opt()?)
        }
        _ => Some(latest),
    }
}

async fn open_file(path: &Path, read_only: bool) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::new().filename(path).read_only(read_only).foreign_keys(false);
    Ok(SqlitePoolOptions::new().max_connections(1).connect_with(options).await?)
}

/// What a backup file says about itself.
#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    pub created_at: i64,
    pub version: String,
    pub key_fingerprint: String,
    pub accounts: i64,
}

impl BackupInfo {
    /// "2026-10-15 03:00" in the server's local time.
    pub fn made_at(&self) -> String {
        Local
            .timestamp_opt(self.created_at, 0)
            .single()
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default()
    }
}

/// Checks that `path` is a sound Havre backup and says what's in it.
pub async fn check(path: &Path) -> Result<BackupInfo> {
    let pool = open_file(path, true)
        .await
        .map_err(|e| Error::Config(format!("{} isn't a readable SQLite file: {e}", path.display())))?;
    let result = async {
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&pool)
            .await
            .map_err(|_| Error::Config(format!("{} isn't a Havre backup.", path.display())))?;
        if integrity != "ok" {
            return Err(Error::Config(format!("This backup is damaged ({integrity}).")));
        }
        let row = sqlx::query("SELECT created_at, version, key_fingerprint FROM havre_backup")
            .fetch_one(&pool)
            .await
            .map_err(|_| Error::Config("This file isn't a Havre backup.".into()))?;
        let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts").fetch_one(&pool).await?;
        Ok(BackupInfo {
            created_at: row.get("created_at"),
            version: row.get("version"),
            key_fingerprint: row.get("key_fingerprint"),
            accounts,
        })
    }
    .await;
    pool.close().await;
    result
}

/// Puts a backup in place of `data_dir/havre.db` (Havre must be stopped).
///
/// The current database is kept as `havre.db.before-restore-<time>`; the
/// key too if a new one is given. Without `key`, the existing `secret.key`
/// (or `HAVRE_SECRET_KEY`) must be the one the backup was made with.
///
/// A sealed off-site copy (`.havre`) is decrypted first, with the same key.
pub async fn restore(data_dir: &Path, backup: &Path, key: Option<&str>) -> Result<BackupInfo> {
    if sealed::is_sealed(backup) {
        let secrets = key_for(data_dir, key).map_err(|_| {
            Error::Config(
                "An off-site copy (.havre) is encrypted: give its recovery key with --key (it's the one Settings › \
                 Backups shows; restoring without it isn't possible)."
                    .into(),
            )
        })?;
        std::fs::create_dir_all(data_dir)?;
        let plain = data_dir.join(format!(".restore-{}.db", Local::now().format("%Y%m%d%H%M%S")));
        let (src, dst) = (backup.to_path_buf(), plain.clone());
        tokio::task::spawn_blocking(move || sealed::unseal(&src, &dst, &secrets))
            .await
            .map_err(|e| Error::Config(format!("Decrypting the copy stopped: {e}")))??;
        let result = restore_plain(data_dir, &plain, key).await;
        let _ = std::fs::remove_file(&plain);
        return result;
    }
    restore_plain(data_dir, backup, key).await
}

/// The recovery key given with `--key`, else the one already in `data_dir`.
fn key_for(data_dir: &Path, key: Option<&str>) -> Result<Secrets> {
    match key {
        Some(hex) => Secrets::from_hex(hex.trim()),
        None => Secrets::load_existing(data_dir)?.ok_or_else(|| {
            Error::Config(format!(
                "There's no encryption key in {}: give the recovery key with --key (Settings › Backups says \
                 how to find it), or restore with --key-mismatch-ok and enter your mail passwords again.",
                data_dir.display()
            ))
        }),
    }
}

async fn restore_plain(data_dir: &Path, backup: &Path, key: Option<&str>) -> Result<BackupInfo> {
    let info = check(backup).await?;
    let secrets = key_for(data_dir, key)?;
    if secrets.fingerprint() != info.key_fingerprint {
        return Err(Error::Config(format!(
            "This backup was made with another encryption key (fingerprint {}, this server's is {}). \
             Give the recovery key from Settings › Backups with --key, or restore anyway with --key-mismatch-ok: \
             mail passwords and sign-ins would then have to be entered again.",
            info.key_fingerprint,
            secrets.fingerprint()
        )));
    }
    replace(data_dir, backup, key)?;
    Ok(info)
}

/// Restores even though the key doesn't match (passwords to re-enter).
pub async fn restore_without_key(data_dir: &Path, backup: &Path) -> Result<BackupInfo> {
    if sealed::is_sealed(backup) {
        return Err(Error::Config(
            "An off-site copy (.havre) is encrypted: it opens only with its recovery key. Give it with --key.".into(),
        ));
    }
    let info = check(backup).await?;
    replace(data_dir, backup, None)?;
    Ok(info)
}

fn replace(data_dir: &Path, backup: &Path, key: Option<&str>) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let stamp = Local::now().format("%Y-%m-%d-%H%M%S");
    let db = data_dir.join("havre.db");
    if db.exists() {
        std::fs::rename(&db, data_dir.join(format!("havre.db.before-restore-{stamp}")))?;
    }
    // A WAL left by the old database must not be applied to the new one.
    for extra in ["havre.db-wal", "havre.db-shm"] {
        let _ = std::fs::remove_file(data_dir.join(extra));
    }
    std::fs::copy(backup, &db)?;
    crate::secrets::restrict_permissions(&db);
    if let Some(hex) = key {
        let key_file = data_dir.join("secret.key");
        if key_file.exists() {
            std::fs::rename(&key_file, data_dir.join(format!("secret.key.before-restore-{stamp}")))?;
        }
        std::fs::write(&key_file, hex.trim())?;
        crate::secrets::restrict_permissions(&key_file);
    }
    Ok(())
}

/// When a file was last written, in unix seconds (0 if the system can't say).
fn modified_at(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_backup_names_are_served() {
        assert!(is_backup_name("havre-2026-10-15-030000.db"));
        assert!(!is_backup_name("havre.db"));
        assert!(!is_backup_name("../havre-x.db"));
        assert!(!is_backup_name("havre-x/../../secret.key.db"));
        assert!(!is_backup_name(".havre-2026.db.partial"));
    }

    #[test]
    fn schedules_the_next_backup() {
        let at = |s: &str| {
            Local
                .from_local_datetime(&chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap())
                .unwrap()
                .timestamp()
        };
        let now = at("2026-10-15 10:00");
        // Never made: today 03:00 is due already.
        assert_eq!(next_run("03:00", None, now), Some(at("2026-10-15 03:00")));
        // Made at 03:01 today: tomorrow.
        assert_eq!(next_run("03:00", Some(at("2026-10-15 03:01")), now), Some(at("2026-10-16 03:00")));
        // Made yesterday: today's slot (already past) is due.
        assert_eq!(next_run("03:00", Some(at("2026-10-14 03:01")), now), Some(at("2026-10-15 03:00")));
        // Before today's slot, done last night: tonight.
        assert_eq!(next_run("23:00", Some(at("2026-10-14 23:00")), now), Some(at("2026-10-15 23:00")));
        assert_eq!(next_run("3h", None, now), None);
    }

    /// A fresh folder under the system's temp dir (removed at the end).
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("havre-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn backup_then_restore_on_a_new_server() {
        let root = temp_dir("backup");
        let data = root.join("data");
        let secrets = Secrets::random();
        let db = Database::open(&data.join("havre.db"), secrets.clone()).await.unwrap();
        let account = db
            .insert_account(&crate::store::NewAccount {
                name: "Work".into(),
                email: "me@x.test".into(),
                color: "work".into(),
                imap_host: "imap.x.test".into(),
                imap_port: 993,
                imap_user: String::new(),
                password: "s3cret".into(),
                sender_name: String::new(),
                smtp_host: None,
                smtp_port: None,
                smtp_security: None,
            })
            .await
            .unwrap();
        db.create_label("Clients", "work").await.unwrap();

        let backups = Backups::new(db.clone(), root.join("backups"), secrets.clone());
        let file = backups.run().await.unwrap();
        assert_eq!(backups.list().unwrap(), std::slice::from_ref(&file));
        let status = backups.status().await.unwrap();
        assert!(status.last_at.is_some() && status.last_error.is_none());

        // A new server: restoring needs the key the backup was made with.
        let new_data = root.join("new");
        let path = backups.path_of(&file.name).unwrap();
        let wrong = Secrets::random().export_hex();
        assert!(restore(&new_data, &path, Some(&wrong)).await.is_err(), "wrong key refused");
        let info = restore(&new_data, &path, Some(&secrets.export_hex())).await.unwrap();
        assert_eq!(info.accounts, 1);

        let restored =
            Database::open(&new_data.join("havre.db"), Secrets::load_or_create(&new_data).unwrap())
                .await
                .unwrap();
        assert_eq!(restored.labels().await.unwrap()[0].name, "Clients");
        // The password decrypts with the restored key.
        assert!(matches!(
            restored.imap_config(account.id).await.unwrap().auth,
            crate::config::ImapAuth::Password(p) if p == "s3cret"
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn restores_a_sealed_copy_with_the_recovery_key() {
        let source = temp_dir("sealed-source");
        let secrets = Secrets::random();
        let db = Database::open(&source.join("havre.db"), secrets.clone()).await.unwrap();
        let backups = Backups::new(db, source.join("backups"), secrets.clone());
        let made = backups.run().await.unwrap();
        let sealed_copy = source.join(copy_name(&made.name));
        sealed::seal(&source.join("backups").join(&made.name), &sealed_copy, &secrets, 0).unwrap();

        // A new server, nothing but the copy and the key.
        let target = temp_dir("sealed-target");
        assert!(restore_without_key(&target, &sealed_copy).await.is_err());
        assert!(restore(&target, &sealed_copy, Some(&Secrets::random().export_hex())).await.is_err());
        let info = restore(&target, &sealed_copy, Some(&secrets.export_hex())).await.unwrap();
        assert_eq!(info.key_fingerprint, secrets.fingerprint());
        assert!(target.join("havre.db").exists());
        let leftovers: Vec<_> = std::fs::read_dir(&target)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with(".restore-"))
            .collect();
        assert!(leftovers.is_empty(), "the decrypted temporary file is removed: {leftovers:?}");
    }

    #[test]
    fn names_the_off_site_copy() {
        assert_eq!(copy_name("havre-2026-11-08-030000.db"), "havre-2026-11-08-030000.havre");
        assert!(remote::is_copy_name(&copy_name("havre-2026-11-08-030000.db")));
    }

    #[tokio::test]
    async fn keeps_only_the_newest() {
        let root = temp_dir("rotate");
        let db = Database::open(&root.join("havre.db"), Secrets::random()).await.unwrap();
        let backups = Backups::new(db, root.join("backups"), Secrets::random());
        backups
            .save_settings(&BackupSettings { enabled: true, time: "03:00".into(), keep: 2 })
            .await
            .unwrap();
        for _ in 0..3 {
            backups.run().await.unwrap();
            // Names have seconds: wait for the next one.
            tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        }
        assert_eq!(backups.list().unwrap().len(), 2);
        assert!(
            backups
                .save_settings(&BackupSettings { enabled: true, time: "25:00".into(), keep: 2 })
                .await
                .is_err()
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
