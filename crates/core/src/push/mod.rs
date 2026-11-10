//! Push notifications to phones and browsers (Web Push), step 20.
//!
//! A browser that turns notifications on gives us a **subscription**: an
//! address at its push service (Google's for Chrome, Apple's for Safari,
//! Mozilla's for Firefox) and two keys. To notify it, we encrypt a small JSON
//! (`{title, body, url}`) for those keys and POST it to the address, signed
//! with Havre's own key ([`crypto`]). The browser wakes Havre's service worker
//! (`web/public/sw.js`), which shows it — even when no Havre page is open.
//!
//! What sends one ([`Push::start`]): a task or event reminder, new mail in an
//! inbox, a new invitation, a scheduled message that couldn't go. A device
//! where Havre is open and in front says so every half minute ([`Push::seen`])
//! and gets nothing: the page shows its own toast.
//!
//! Notifications have buttons (step 26, [`actions`]): *Archive*, *Done*, *In
//! 10 min*, *Accept*… The service worker sends the tapped one back to the
//! server, which does the work like the app's own buttons.

pub mod actions;
pub mod crypto;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tokio::sync::{OnceCell, broadcast};

use crate::{
    Error, Result,
    store::Database,
    sync::{AccountEvent, SyncEvent},
    users::UserId,
};
pub use actions::{Action, Button, Target};
use crypto::{BrowserKeys, Vapid};

/// Who to contact about this server's pushes (the VAPID `sub` claim): the
/// project's page, rather than your address, which push services don't need.
const CONTACT: &str = "https://github.com/maelbel/havre";
/// A device that said "I'm in front" less than this ago gets no push.
const IN_FRONT: Duration = Duration::from_secs(70);
/// More new emails than this at once: one notification for all of them.
const SUMMARY_ABOVE: usize = 3;
/// How long new mail is watched for an invitation (its body is read a moment
/// after its headers: the invitation is only known then).
const INVITATION_WATCH: Duration = Duration::from_secs(3600);
/// How often snoozed reminders are looked at.
const SNOOZE_EVERY: Duration = Duration::from_secs(30);

/// What the service worker shows. `Default` lets a notification without
/// buttons say only what it has (`..Default::default()`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Notification {
    pub title: String,
    pub body: String,
    /// The page to open on click (`/mail?account=…`).
    pub url: String,
    /// A notification with the same tag replaces the previous one.
    pub tag: String,
    /// Its buttons (none: tapping it only opens `url`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Button>,
    /// What the buttons act on; sent back to the server with the tapped one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Target>,
    /// An online meeting's link: the *Join* button opens it (no server needed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub join_url: Option<String>,
    /// Buzz even when it replaces one with the same tag (a reminder coming back).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub renotify: bool,
}

/// A browser subscribing (what `PushSubscription.toJSON()` gives, plus a name).
#[derive(Debug, Clone, Deserialize)]
pub struct NewSubscription {
    pub endpoint: String,
    pub keys: SubscriptionKeys,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionKeys {
    pub p256dh: String,
    pub auth: String,
}

/// A device that gets notifications, for Settings › Notifications.
#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub id: i64,
    pub label: String,
    /// So the page can tell which one it is (the endpoint itself stays here).
    pub this_device: bool,
    pub created_at: i64,
    pub last_ok_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushSettings {
    pub new_mail: bool,
    pub mail_details: bool,
    pub reminders: bool,
    pub event_reminders: bool,
    pub invitations: bool,
}

/// A partial update: only the fields that are present change.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PushSettingsUpdate {
    pub new_mail: Option<bool>,
    pub mail_details: Option<bool>,
    pub reminders: Option<bool>,
    pub event_reminders: Option<bool>,
    pub invitations: Option<bool>,
}

/// An email notified a moment ago, watched until its body is read: it may
/// carry an invitation.
struct Recent {
    account: i64,
    folder: String,
    uid: u32,
    at: Instant,
}

pub struct Push {
    db: Database,
    http: reqwest::Client,
    /// Havre's VAPID key, loaded (or made) the first time it's needed.
    /// `OnceCell`: set once, then read by everyone without a lock.
    vapid: OnceCell<Vapid>,
    /// Endpoint → when that device last said Havre is open and in front.
    in_front: Mutex<HashMap<String, Instant>>,
    /// (account, inbox) → the highest UID already looked at: mail at or
    /// below it is never notified (it was there before, or already sent).
    seen_uids: tokio::sync::Mutex<HashMap<(i64, String), i64>>,
    /// New mail that may still turn out to be an invitation.
    recent: tokio::sync::Mutex<Vec<Recent>>,
    /// Havre's event channel: a snoozed reminder is announced on it again,
    /// so it rings everywhere a reminder rings (here, the open page, Desktop).
    events: broadcast::Sender<AccountEvent>,
}

impl Push {
    /// Starts listening to Havre's events (new mail, reminders) and pushes.
    pub fn start(db: Database, events: &broadcast::Sender<AccountEvent>) -> Arc<Self> {
        let push = Arc::new(Self::new(db, events.clone()));
        // Snoozed reminders: looked at every half minute.
        let me = Arc::clone(&push);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(SNOOZE_EVERY).await;
                if let Err(err) = me.ring_snoozed(now()).await {
                    tracing::warn!("push: snoozed reminders: {err}");
                }
            }
        });
        let mut rx = events.subscribe();
        let me = Arc::clone(&push);
        tokio::spawn(async move {
            // What's in the inboxes now is old news.
            if let Err(err) = me.remember_inboxes().await {
                tracing::warn!("push: {err}");
            }
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if let Err(err) = me.on_event(&event).await {
                            tracing::warn!("push: {err}");
                        }
                    }
                    // Too many events at once: some were skipped, keep going.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        push
    }

    fn new(db: Database, events: broadcast::Sender<AccountEvent>) -> Self {
        Self {
            db,
            http: reqwest::Client::builder().timeout(Duration::from_secs(20)).build().expect("HTTP client"),
            vapid: OnceCell::new(),
            in_front: Mutex::new(HashMap::new()),
            seen_uids: tokio::sync::Mutex::new(HashMap::new()),
            recent: tokio::sync::Mutex::new(Vec::new()),
            events,
        }
    }

    fn pool(&self) -> SqlitePool {
        self.db.pool()
    }

    // ---- Keys and devices -----------------------------------------------------------

    /// Havre's VAPID key: from the database, or made and saved the first time.
    async fn vapid(&self) -> Result<&Vapid> {
        self.vapid
            .get_or_try_init(|| async {
                let saved: Option<Vec<u8>> =
                    sqlx::query_scalar("SELECT vapid_key FROM push_settings WHERE id = 1")
                        .fetch_one(&self.pool())
                        .await?;
                let key = match saved {
                    Some(sealed) => {
                        let hex_key = self.db.secrets().decrypt(&sealed)?;
                        let bytes =
                            hex::decode(hex_key).map_err(|_| Error::Config("bad VAPID key".into()))?;
                        p256::SecretKey::from_slice(&bytes)
                            .map_err(|_| Error::Config("bad VAPID key".into()))?
                    }
                    None => {
                        let key = crypto::random_key();
                        let sealed = self.db.secrets().encrypt(&hex::encode(key.to_bytes()))?;
                        sqlx::query("UPDATE push_settings SET vapid_key = ? WHERE id = 1")
                            .bind(sealed)
                            .execute(&self.pool())
                            .await?;
                        key
                    }
                };
                Ok(Vapid::new(&key, CONTACT))
            })
            .await
    }

    /// The public key a browser subscribes with.
    pub async fn public_key(&self) -> Result<String> {
        Ok(self.vapid().await?.public_key())
    }

    /// Saves a browser's subscription (the same address again just updates it).
    /// A device belongs to whoever turned notifications on there; signing in
    /// as someone else and turning them on again makes it theirs.
    pub async fn subscribe(&self, user: UserId, s: &NewSubscription) -> Result<()> {
        check_endpoint(&s.endpoint)?;
        BrowserKeys::from_base64(&s.keys.p256dh, &s.keys.auth)?;
        let label: String = s.label.trim().chars().take(80).collect();
        sqlx::query(
            "INSERT INTO push_subscriptions (endpoint, p256dh, auth, label, created_at, user_id)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (endpoint) DO UPDATE SET p256dh = excluded.p256dh, auth = excluded.auth,
                 label = excluded.label, last_error = NULL, user_id = excluded.user_id",
        )
        .bind(&s.endpoint)
        .bind(&s.keys.p256dh)
        .bind(&s.keys.auth)
        .bind(label)
        .bind(now())
        .bind(user.0)
        .execute(&self.pool())
        .await?;
        Ok(())
    }

    /// Forgets a browser (it turned notifications off).
    pub async fn unsubscribe(&self, user: UserId, endpoint: &str) -> Result<()> {
        sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = ? AND user_id = ?")
            .bind(endpoint)
            .bind(user.0)
            .execute(&self.pool())
            .await?;
        Ok(())
    }

    /// The push service says the browser is gone: forget it, whoever's it was.
    async fn forget(&self, endpoint: &str) -> Result<()> {
        sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = ?")
            .bind(endpoint)
            .execute(&self.pool())
            .await?;
        Ok(())
    }

    /// `user`'s devices; `this_device` marks the one with `endpoint`.
    pub async fn devices(&self, user: UserId, endpoint: Option<&str>) -> Result<Vec<Device>> {
        let rows = sqlx::query(
            "SELECT id, endpoint, label, created_at, last_ok_at, last_error FROM push_subscriptions
             WHERE user_id = ? ORDER BY id",
        )
        .bind(user.0)
        .fetch_all(&self.pool())
        .await?;
        Ok(rows
            .iter()
            .map(|r| Device {
                id: r.get("id"),
                label: r.get("label"),
                this_device: endpoint == Some(r.get::<&str, _>("endpoint")),
                created_at: r.get("created_at"),
                last_ok_at: r.get("last_ok_at"),
                last_error: r.get("last_error"),
            })
            .collect())
    }

    pub async fn remove_device(&self, user: UserId, id: i64) -> Result<()> {
        let done = sqlx::query("DELETE FROM push_subscriptions WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user.0)
            .execute(&self.pool())
            .await?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound(format!("device {id}")));
        }
        Ok(())
    }

    /// What `user` wants pushed (everything, until they choose).
    pub async fn settings(&self, user: UserId) -> Result<PushSettings> {
        let r = sqlx::query(
            "SELECT new_mail, mail_details, reminders, event_reminders, invitations FROM push_prefs WHERE user_id = ?",
        )
        .bind(user.0)
        .fetch_optional(&self.pool())
        .await?;
        Ok(match r {
            Some(r) => PushSettings {
                new_mail: r.get("new_mail"),
                mail_details: r.get("mail_details"),
                reminders: r.get("reminders"),
                event_reminders: r.get("event_reminders"),
                invitations: r.get("invitations"),
            },
            None => PushSettings {
                new_mail: true,
                mail_details: true,
                reminders: true,
                event_reminders: true,
                invitations: true,
            },
        })
    }

    pub async fn update_settings(&self, user: UserId, u: &PushSettingsUpdate) -> Result<PushSettings> {
        sqlx::query("INSERT OR IGNORE INTO push_prefs (user_id) VALUES (?)")
            .bind(user.0)
            .execute(&self.pool())
            .await?;
        // `COALESCE(?, column)`: a missing field (NULL) keeps what's there.
        sqlx::query(
            "UPDATE push_prefs SET new_mail = COALESCE(?, new_mail), mail_details = COALESCE(?, mail_details),
             reminders = COALESCE(?, reminders), event_reminders = COALESCE(?, event_reminders),
             invitations = COALESCE(?, invitations) WHERE user_id = ?",
        )
        .bind(u.new_mail)
        .bind(u.mail_details)
        .bind(u.reminders)
        .bind(u.event_reminders)
        .bind(u.invitations)
        .bind(user.0)
        .execute(&self.pool())
        .await?;
        self.settings(user).await
    }

    /// A page says Havre is open and in front on this device.
    pub fn seen(&self, endpoint: &str) {
        let mut map = self.in_front.lock().unwrap();
        map.retain(|_, at| at.elapsed() < IN_FRONT);
        map.insert(endpoint.to_owned(), Instant::now());
    }

    fn is_in_front(&self, endpoint: &str) -> bool {
        self.in_front.lock().unwrap().get(endpoint).is_some_and(|at| at.elapsed() < IN_FRONT)
    }

    // ---- Sending ------------------------------------------------------------------

    /// Sends a test to one device (even if Havre is in front there: you asked).
    pub async fn test(&self, user: UserId, endpoint: &str) -> Result<()> {
        let note = Notification {
            title: "Havre".into(),
            body: "Notifications work on this device.".into(),
            url: "/settings/notifications".into(),
            tag: "havre-test".into(),
            ..Default::default()
        };
        let row = sqlx::query(
            "SELECT id, endpoint, p256dh, auth FROM push_subscriptions WHERE endpoint = ? AND user_id = ?",
        )
        .bind(endpoint)
        .bind(user.0)
        .fetch_optional(&self.pool())
        .await?
        .ok_or_else(|| Error::Config("Turn notifications on for this device first.".into()))?;
        self.send(&row, &note).await
    }

    /// Sends `note` to every device of `user` not showing Havre in front.
    /// Returns how many got it.
    pub async fn notify(&self, user: UserId, note: &Notification) -> Result<usize> {
        let rows = sqlx::query("SELECT id, endpoint, p256dh, auth FROM push_subscriptions WHERE user_id = ?")
            .bind(user.0)
            .fetch_all(&self.pool())
            .await?;
        let mut sent = 0;
        for row in &rows {
            if self.is_in_front(row.get("endpoint")) {
                continue;
            }
            match self.send(row, note).await {
                Ok(()) => sent += 1,
                Err(err) => tracing::info!("push to device {} failed: {err}", row.get::<i64, _>("id")),
            }
        }
        Ok(sent)
    }

    /// Encrypts and posts one notification; keeps the device's status.
    async fn send(&self, row: &sqlx::sqlite::SqliteRow, note: &Notification) -> Result<()> {
        let id: i64 = row.get("id");
        let endpoint: String = row.get("endpoint");
        let keys = BrowserKeys::from_base64(row.get("p256dh"), row.get("auth"))?;
        let text = serde_json::to_vec(note).map_err(|e| Error::Config(e.to_string()))?;
        let body = crypto::seal(&text, &keys)?;
        let response = self
            .http
            .post(&endpoint)
            .header("Authorization", self.vapid().await?.authorization(&endpoint, now())?)
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            // Kept by the push service up to a day if the phone is off.
            .header("TTL", "86400")
            .header("Urgency", "normal")
            .body(body)
            .send()
            .await;
        let result = match response {
            Ok(r) if r.status().is_success() => Ok(()),
            // The browser unsubscribed, or the subscription expired: forget it.
            Ok(r) if matches!(r.status().as_u16(), 404 | 410) => {
                self.forget(&endpoint).await?;
                return Err(Error::Config("this device no longer accepts notifications (removed)".into()));
            }
            Ok(r) => {
                let status = r.status();
                let detail: String = r.text().await.unwrap_or_default().chars().take(200).collect();
                Err(Error::Config(format!("the push service answered {status}. {detail}").trim().to_owned()))
            }
            Err(e) => Err(Error::Config(format!("the push service can't be reached: {e}"))),
        };
        let (ok_at, error) = match &result {
            Ok(()) => (Some(now()), None),
            Err(e) => (None, Some(e.to_string())),
        };
        sqlx::query(
            "UPDATE push_subscriptions SET last_ok_at = COALESCE(?, last_ok_at), last_error = ? WHERE id = ?",
        )
        .bind(ok_at)
        .bind(error)
        .bind(id)
        .execute(&self.pool())
        .await?;
        result
    }

    // ---- What sends one -------------------------------------------------------------

    /// Each event goes to the devices of the user it's for (`event.user`).
    async fn on_event(&self, event: &AccountEvent) -> Result<()> {
        let user = event.user;
        match &event.event {
            SyncEvent::Reminder { task, title } => {
                if self.settings(user).await?.reminders {
                    self.notify(user, &task_notification(*task, title)).await?;
                }
            }
            SyncEvent::EventReminder { event: id, title, start, all_day, location, join_url } => {
                if self.settings(user).await?.event_reminders {
                    let tz = self.time_zone(user).await?;
                    let when = When { start: *start, all_day: *all_day, tz };
                    self.notify(user, &event_notification(*id, title, &when, location, join_url.as_deref()))
                        .await?;
                }
            }
            // A scheduled message couldn't go: always said (you may count on it).
            SyncEvent::Outbox { draft, subject, sent: false, error: Some(error), .. } => {
                let subject = if subject.trim().is_empty() { "(no subject)" } else { subject.trim() };
                self.notify(
                    user,
                    &Notification {
                        title: format!("Not sent: {subject}"),
                        body: error.chars().take(160).collect(),
                        url: "/mail?view=scheduled".into(),
                        tag: format!("havre-outbox-{draft}"),
                        ..Default::default()
                    },
                )
                .await?;
            }
            SyncEvent::Folder { path } => {
                let notes = self.new_mail(user, event.account, path).await?;
                let settings = self.settings(user).await?;
                if settings.new_mail {
                    for note in notes {
                        self.notify(user, &note).await?;
                    }
                }
                // Invitations come a moment later (once the body is read), and
                // take the place of the email's notification (the same tag).
                let invitations = self.new_invitations(user, event.account, path).await?;
                if settings.invitations {
                    for note in invitations {
                        self.notify(user, &note).await?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Your time zone, as the Agenda knows it from your browser (UTC until then).
    async fn time_zone(&self, user: UserId) -> Result<chrono_tz::Tz> {
        let zone = self.db.agenda(user).settings().await?.time_zone;
        Ok(zone.and_then(|z| z.parse().ok()).unwrap_or(chrono_tz::Tz::UTC))
    }

    /// Notes the highest UID of every inbox: what's there now is never notified.
    async fn remember_inboxes(&self) -> Result<()> {
        let rows = sqlx::query(
            "SELECT f.account_id, f.path, COALESCE(MAX(m.uid), 0) AS top FROM folders f
             LEFT JOIN messages m ON m.account_id = f.account_id AND m.folder = f.path
             WHERE f.role = 'inbox' GROUP BY f.account_id, f.path",
        )
        .fetch_all(&self.pool())
        .await?;
        let mut seen = self.seen_uids.lock().await;
        for r in rows {
            seen.insert((r.get("account_id"), r.get("path")), r.get("top"));
        }
        Ok(())
    }

    /// The notifications for mail that arrived in `folder` (if it's an
    /// inbox): unread, newer than what we looked at, from the last day, not
    /// snoozed. Moves the mark forward either way.
    async fn new_mail(&self, user: UserId, account: i64, folder: &str) -> Result<Vec<Notification>> {
        let inbox: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM folders WHERE account_id = ? AND path = ? AND role = 'inbox')",
        )
        .bind(account)
        .bind(folder)
        .fetch_one(&self.pool())
        .await?;
        if !inbox {
            return Ok(Vec::new());
        }
        // Held for the whole check, so two events for the same inbox can't both notify a message.
        let mut seen = self.seen_uids.lock().await;
        let key = (account, folder.to_owned());
        let top: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(uid), 0) FROM messages WHERE account_id = ? AND folder = ?",
        )
        .bind(account)
        .bind(folder)
        .fetch_one(&self.pool())
        .await?;
        // An inbox we didn't know (a new account): its mail is old news too.
        let Some(&mark) = seen.get(&key) else {
            seen.insert(key, top);
            return Ok(Vec::new());
        };
        seen.insert(key, top.max(mark));
        let rows = sqlx::query(
            "SELECT m.uid, m.subject, m.from_name, m.from_address FROM messages m
             WHERE m.account_id = ? AND m.folder = ? AND m.uid > ? AND m.unread = 1
               AND datetime(m.date) > datetime('now', '-1 day')
               AND NOT EXISTS (SELECT 1 FROM snoozes s WHERE s.account_id = m.account_id AND s.folder = m.folder
                               AND s.uid = m.uid AND s.back = 0)
             ORDER BY m.uid",
        )
        .bind(account)
        .bind(folder)
        .bind(mark)
        .fetch_all(&self.pool())
        .await?;
        drop(seen);
        // Watched for a while: one of them may be an invitation.
        let mut recent = self.recent.lock().await;
        recent.retain(|r| r.at.elapsed() < INVITATION_WATCH);
        for r in &rows {
            let uid = r.get::<i64, _>("uid") as u32;
            recent.push(Recent { account, folder: folder.to_owned(), uid, at: Instant::now() });
        }
        drop(recent);
        let details = self.settings(user).await?.mail_details;
        let mail: Vec<(i64, String, String)> = rows
            .iter()
            .map(|r| {
                let from = r
                    .get::<Option<String>, _>("from_name")
                    .filter(|n| !n.trim().is_empty())
                    .or_else(|| r.get("from_address"))
                    .unwrap_or_else(|| "Someone".into());
                (r.get("uid"), from, r.get("subject"))
            })
            .collect();
        Ok(mail_notifications(account, folder, &mail, details))
    }

    /// The invitations among the mail notified a moment ago in `folder`, now
    /// that their bodies are read. Each email is looked at once: when its
    /// body is in, it's either an invitation (a notification) or forgotten.
    async fn new_invitations(&self, user: UserId, account: i64, folder: &str) -> Result<Vec<Notification>> {
        let mut recent = self.recent.lock().await;
        let mut notes = Vec::new();
        let mut done: Vec<u32> = Vec::new();
        for r in recent.iter().filter(|r| r.account == account && r.folder == folder) {
            let row = sqlx::query(
                "SELECT files, has_invite FROM messages WHERE account_id = ? AND folder = ? AND uid = ?",
            )
            .bind(account)
            .bind(folder)
            .bind(r.uid as i64)
            .fetch_optional(&self.pool())
            .await?;
            let Some(row) = row else {
                done.push(r.uid); // gone (moved, deleted)
                continue;
            };
            // `files` is filled when the body is read: until then, wait.
            let Some(files) = row.get::<Option<String>, _>("files") else { continue };
            done.push(r.uid);
            if row.get::<bool, _>("has_invite") {
                // An invitation that can't be read keeps the email's notification.
                match self.invitation_notification(user, account, folder, r.uid, &files).await {
                    Ok(Some(note)) => notes.push(note),
                    Ok(None) => {}
                    Err(err) => tracing::debug!("push: invitation in message {}: {err}", r.uid),
                }
            }
        }
        recent.retain(|r| !(r.account == account && r.folder == folder && done.contains(&r.uid)));
        Ok(notes)
    }

    /// The notification for the invitation in an email (read from Havre's
    /// copy of it), or `None` when there's nothing to answer: a cancellation,
    /// an update, your own meeting, or one you already answered.
    async fn invitation_notification(
        &self,
        user: UserId,
        account: i64,
        folder: &str,
        uid: u32,
        files: &str,
    ) -> Result<Option<Notification>> {
        let files: Vec<crate::mail::FileInfo> = serde_json::from_str(files).unwrap_or_default();
        let Some(ics) =
            files.iter().find(|f| f.mime == "text/calendar" || f.name.to_lowercase().ends_with(".ics"))
        else {
            return Ok(None);
        };
        let raw: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT raw FROM bodies WHERE account_id = ? AND folder = ? AND uid = ?")
                .bind(account)
                .bind(folder)
                .bind(uid as i64)
                .fetch_optional(&self.pool())
                .await?;
        let Some(raw) = raw else { return Ok(None) };
        let Some(file) = crate::mail::extract_attachment(&raw, ics.index) else { return Ok(None) };
        let invitation = crate::agenda::ics::read_invitation(&String::from_utf8_lossy(&file.bytes))?;
        let Some(first) = invitation.events.first() else { return Ok(None) };
        let me: String = sqlx::query_scalar("SELECT email FROM accounts WHERE id = ?")
            .bind(account)
            .fetch_one(&self.pool())
            .await?;
        let mine = invitation.organizer_email.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&me));
        if invitation.method != "REQUEST" || mine {
            return Ok(None);
        }
        // Already in your calendars with your answer: an update, nothing to ask.
        if let Some(uid) = &invitation.uid {
            let agenda = self.db.agenda(user);
            if agenda.events_by_uid(uid).await?.iter().any(|e| e.my_response.is_some()) {
                return Ok(None);
            }
        }
        let when = When { start: first.start, all_day: first.all_day, tz: self.time_zone(user).await? };
        Ok(Some(invitation_notification(
            account,
            folder,
            uid,
            ics.index,
            &first.title,
            &when,
            invitation.organizer.as_deref(),
        )))
    }

    // ---- Buttons --------------------------------------------------------------------

    /// What a mail button may act on: the emails still in that folder. An
    /// unknown UID would be ignored by the mail server, and the button would
    /// seem to work; here it's said instead. Other targets are returned as
    /// they are (their own functions look them up).
    pub async fn still_there(&self, user: UserId, target: &Target) -> Result<Target> {
        let gone = || Error::Config("That email isn't there any more.".into());
        match target {
            Target::Mail { account, folder, uid } => {
                if self.has_message(user, *account, folder, *uid).await? {
                    Ok(target.clone())
                } else {
                    Err(gone())
                }
            }
            Target::Inbox { account, folder, uids } => {
                let mut there = Vec::new();
                for uid in uids {
                    if self.has_message(user, *account, folder, *uid).await? {
                        there.push(*uid);
                    }
                }
                if there.is_empty() {
                    return Err(gone());
                }
                Ok(Target::Inbox { account: *account, folder: folder.clone(), uids: there })
            }
            _ => Ok(target.clone()),
        }
    }

    /// In one of `user`'s accounts: someone else's email is never "there".
    async fn has_message(&self, user: UserId, account: i64, folder: &str, uid: u32) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM messages WHERE account_id = ? AND folder = ? AND uid = ?
                              AND account_id IN (SELECT id FROM accounts WHERE user_id = ?))",
        )
        .bind(account)
        .bind(folder)
        .bind(uid as i64)
        .bind(user.0)
        .fetch_one(&self.pool())
        .await?)
    }

    // ---- Snoozing -------------------------------------------------------------------

    /// A reminder again in `minutes` (a notification's *In 10 min*). The task
    /// or event must exist; it's looked up again when the time comes.
    pub async fn snooze(&self, user: UserId, target: &Target, minutes: i64) -> Result<()> {
        let (kind, id, start) = match target {
            Target::Task { task } => {
                self.db.tasks(user).task(*task).await?;
                ("task", *task, None)
            }
            Target::Event { event, start } => {
                self.db.agenda(user).event(*event).await?;
                ("event", *event, Some(*start))
            }
            _ => return Err(Error::Config("Only reminders can come back later.".into())),
        };
        sqlx::query("INSERT INTO push_snoozed (kind, ref_id, start, due_at, user_id) VALUES (?, ?, ?, ?, ?)")
            .bind(kind)
            .bind(id)
            .bind(start)
            .bind(now() + minutes * 60)
            .bind(user.0)
            .execute(&self.pool())
            .await?;
        Ok(())
    }

    /// Announces the snoozed reminders whose time has come, once each: the
    /// row is deleted first, and only the one who deleted it rings.
    async fn ring_snoozed(&self, now: i64) -> Result<()> {
        let due = sqlx::query("SELECT id, kind, ref_id, start, user_id FROM push_snoozed WHERE due_at <= ?")
            .bind(now)
            .fetch_all(&self.pool())
            .await?;
        for row in due {
            let gone = sqlx::query("DELETE FROM push_snoozed WHERE id = ?")
                .bind(row.get::<i64, _>("id"))
                .execute(&self.pool())
                .await?;
            if gone.rows_affected() == 0 {
                continue;
            }
            let id: i64 = row.get("ref_id");
            let user = UserId(row.get("user_id"));
            // Done or deleted since: nothing to remind.
            let event = match row.get::<String, _>("kind").as_str() {
                "task" => match self.db.tasks(user).task(id).await {
                    Ok(task) if task.done_at.is_none() => SyncEvent::Reminder { task: id, title: task.title },
                    _ => continue,
                },
                _ => match self.db.agenda(user).event(id).await {
                    Ok(e) => SyncEvent::EventReminder {
                        event: id,
                        title: e.title,
                        start: row.get::<Option<i64>, _>("start").unwrap_or(e.start),
                        all_day: e.all_day,
                        location: e.location,
                        join_url: e.join_url,
                    },
                    Err(_) => continue,
                },
            };
            // Nobody listening (no page open): fine, `on_event` below still pushes.
            let _ = self.events.send(AccountEvent { account: 0, user, event });
        }
        Ok(())
    }
}

/// When something happens, and the time zone to say it in.
struct When {
    start: i64,
    all_day: bool,
    tz: chrono_tz::Tz,
}

impl When {
    /// The day (an all-day event's is its UTC date; a timed one's, where you are).
    fn date(&self) -> chrono::NaiveDate {
        if self.all_day {
            chrono::DateTime::from_timestamp(self.start, 0).unwrap_or_default().date_naive()
        } else {
            self.local().date_naive()
        }
    }

    fn local(&self) -> chrono::DateTime<chrono_tz::Tz> {
        use chrono::TimeZone;
        self.tz.timestamp_opt(self.start, 0).single().unwrap_or_else(|| self.tz.timestamp_opt(0, 0).unwrap())
    }

    /// "Thu 14 Nov, 10:00", "Thu 14 Nov, all day".
    fn day_and_time(&self) -> String {
        let day = self.date().format("%a %-d %b");
        if self.all_day {
            format!("{day}, all day")
        } else {
            format!("{day}, {}", self.local().format("%H:%M"))
        }
    }
}

/// A task reminder: *Done*, *In 1 hour*.
fn task_notification(task: i64, title: &str) -> Notification {
    Notification {
        title: "Reminder".into(),
        body: title.to_owned(),
        url: format!("/tasks?task={task}"),
        tag: format!("havre-task-{task}"),
        actions: vec![Action::Done.button("Done"), Action::Snooze(60).button(&actions::snooze_title(60))],
        data: Some(Target::Task { task }),
        renotify: true,
        ..Default::default()
    }
}

/// A new invitation: *Accept*, *Decline*. It has the email's tag, so it takes
/// the place of the email's own notification.
fn invitation_notification(
    account: i64,
    folder: &str,
    uid: u32,
    index: usize,
    title: &str,
    when: &When,
    organizer: Option<&str>,
) -> Notification {
    let title = if title.trim().is_empty() { "(no title)" } else { title.trim() };
    // "Claire Dubois <claire@example.org>": the name is enough here.
    let organizer = organizer.map(|o| o.split(" <").next().unwrap_or(o).trim());
    let body = match organizer.filter(|o| !o.is_empty()) {
        Some(o) => format!("{} · from {o}", when.day_and_time()),
        None => when.day_and_time(),
    };
    Notification {
        title: format!("Invitation: {title}"),
        body,
        url: format!("/mail?account={account}&folder={}&uid={uid}", url_part(folder)),
        tag: format!("havre-mail-{account}-{uid}"),
        actions: vec![Action::Accept.button("Accept"), Action::Decline.button("Decline")],
        data: Some(Target::Invitation { account, folder: folder.to_owned(), uid, index }),
        ..Default::default()
    }
}

/// An event reminder: "Kick-off" / "10:00 · Room 3" (your time), opening the
/// event in the Agenda. Buttons: *Join* and *In 10 min* for an online
/// meeting, else *In 5 min* and *In 10 min*.
fn event_notification(
    id: i64,
    title: &str,
    when: &When,
    location: &str,
    join_url: Option<&str>,
) -> Notification {
    let date = when.date();
    let time = if when.all_day {
        format!("All day, {}", date.format("%a %-d %b"))
    } else {
        when.local().format("%H:%M").to_string()
    };
    let body = if location.trim().is_empty() { time } else { format!("{time} · {}", location.trim()) };
    // Only a web address is opened by the service worker.
    let join_url = join_url.filter(|u| u.starts_with("https://")).map(str::to_owned);
    let snooze = |m: i64| Action::Snooze(m).button(&actions::snooze_title(m));
    let actions = match join_url {
        Some(_) => vec![Button { action: "join".into(), title: "Join".into() }, snooze(10)],
        None => vec![snooze(5), snooze(10)],
    };
    Notification {
        title: title.to_owned(),
        body,
        url: format!("/agenda?date={}&event={id}", date.format("%Y-%m-%d")),
        tag: format!("havre-event-{id}-{}", when.start),
        actions,
        data: Some(Target::Event { event: id, start: when.start }),
        join_url,
        renotify: true,
    }
}

/// The notifications for new mail: one per email, or a summary above
/// [`SUMMARY_ABOVE`]. `details: false` leaves the sender and subject out.
fn mail_notifications(
    account: i64,
    folder: &str,
    mail: &[(i64, String, String)],
    details: bool,
) -> Vec<Notification> {
    let inbox_url = format!("/mail?account={account}&folder={}", url_part(folder));
    if mail.len() > SUMMARY_ABOVE {
        let mut senders: Vec<&str> = Vec::new();
        for (_, from, _) in mail {
            if !senders.contains(&from.as_str()) {
                senders.push(from);
            }
        }
        return vec![Notification {
            title: format!("{} new emails", mail.len()),
            body: if details {
                senders.iter().take(4).copied().collect::<Vec<_>>().join(", ")
            } else {
                String::new()
            },
            url: inbox_url,
            tag: format!("havre-mail-{account}"),
            actions: vec![Action::Read.button("Mark all as read")],
            data: Some(Target::Inbox {
                account,
                folder: folder.to_owned(),
                uids: mail.iter().map(|(uid, _, _)| *uid as u32).collect(),
            }),
            ..Default::default()
        }];
    }
    mail.iter()
        .map(|(uid, from, subject)| Notification {
            title: if details { from.clone() } else { "New email".into() },
            body: if details {
                if subject.trim().is_empty() { "(no subject)".into() } else { subject.clone() }
            } else {
                String::new()
            },
            url: format!("{inbox_url}&uid={uid}"),
            tag: format!("havre-mail-{account}-{uid}"),
            actions: vec![Action::Archive.button("Archive"), Action::Read.button("Mark as read")],
            data: Some(Target::Mail { account, folder: folder.to_owned(), uid: *uid as u32 }),
            ..Default::default()
        })
        .collect()
}

/// Percent-encodes a folder name for a URL (`INBOX/Clients` → `INBOX%2FClients`).
fn url_part(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Push services are HTTPS. (Tests use a local plain-HTTP pretend service.)
fn check_endpoint(endpoint: &str) -> Result<()> {
    let url =
        reqwest::Url::parse(endpoint).map_err(|_| Error::Config("That isn't a push address.".into()))?;
    if url.scheme() == "https" || (cfg!(test) && url.host_str() == Some("127.0.0.1")) {
        Ok(())
    } else {
        Err(Error::Config("Push addresses must use HTTPS.".into()))
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::users::UserId;
    use crate::{
        mail::{FolderRole, MessageSummary, Person},
        store::tests::{folder, new_account},
    };
    use crypto::tests::{AUTH, UA_PRIVATE, UA_PUBLIC, b64, open};

    type Requests = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

    /// A `Push` with an event channel nobody else listens to.
    fn new_push(db: Database) -> Push {
        Push::new(db, broadcast::channel(16).0)
    }

    /// A pretend push service: answers each request with the next status of
    /// `statuses`, and keeps what it received (headers, body).
    async fn push_service(statuses: Vec<u16>) -> (String, Requests) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/send/device-1", listener.local_addr().unwrap());
        let got: Requests = Arc::default();
        let keep = Arc::clone(&got);
        tokio::spawn(async move {
            for status in statuses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buf = [0u8; 8192];
                // The headers, then as many bytes as Content-Length says.
                let (head, body) = loop {
                    let n = socket.read(&mut buf).await.unwrap();
                    request.extend_from_slice(&buf[..n]);
                    let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                    let head = String::from_utf8_lossy(&request[..end]).to_string();
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_owned())
                        })
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break (head, request[end + 4..end + 4 + length].to_vec());
                    }
                };
                keep.lock().unwrap().push((head, body));
                let reply = format!("HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        (url, got)
    }

    fn mail(uid: u32, from: &str, subject: &str, unread: bool, hours_ago: i64) -> MessageSummary {
        let date = chrono::Utc::now() - chrono::Duration::hours(hours_ago);
        MessageSummary {
            uid,
            from: Some(Person { name: Some(from.into()), address: Some(format!("{uid}@x.test")) }),
            subject: subject.into(),
            date: Some(date.to_rfc3339()),
            unread,
            ..Default::default()
        }
    }

    async fn subscribe(push: &Push, endpoint: &str) {
        let sub = NewSubscription {
            endpoint: endpoint.into(),
            keys: SubscriptionKeys { p256dh: UA_PUBLIC.into(), auth: AUTH.into() },
            label: "Chrome on Android".into(),
        };
        push.subscribe(UserId::ADMIN, &sub).await.unwrap();
    }

    /// What the phone would show: the request's body, decrypted.
    fn shown(body: &[u8]) -> serde_json::Value {
        let ua = p256::SecretKey::from_slice(&b64(UA_PRIVATE)).unwrap();
        serde_json::from_slice(&open(body, &ua, &b64(AUTH))).unwrap()
    }

    fn inbox_event(account: i64) -> AccountEvent {
        AccountEvent { account, user: UserId::ADMIN, event: SyncEvent::Folder { path: "INBOX".into() } }
    }

    #[tokio::test]
    async fn new_mail_is_pushed_once_and_old_mail_never() {
        let db = Database::in_memory().await.unwrap();
        let account = db.insert_account(UserId::ADMIN, &new_account("me@x.test")).await.unwrap().id;
        let store = db.store(account);
        store
            .replace_folders(&[folder("INBOX", FolderRole::Inbox), folder("Archive", FolderRole::Archive)])
            .await
            .unwrap();
        store.upsert_messages("INBOX", &[mail(1, "Old", "Already there", true, 1)]).await.unwrap();

        let push = new_push(db.clone());
        push.remember_inboxes().await.unwrap();
        let (endpoint, got) = push_service(vec![201, 201]).await;
        subscribe(&push, &endpoint).await;

        store
            .upsert_messages(
                "INBOX",
                &[
                    mail(2, "Claire Dubois", "Kick-off planning", true, 0),
                    mail(3, "Read already", "Seen elsewhere", false, 0),
                    mail(4, "Backfill", "Two days old", true, 48),
                ],
            )
            .await
            .unwrap();
        store.upsert_messages("Archive", &[mail(9, "Elsewhere", "Not an inbox", true, 0)]).await.unwrap();
        push.on_event(&inbox_event(account)).await.unwrap();
        push.on_event(&AccountEvent {
            account,
            user: UserId::ADMIN,
            event: SyncEvent::Folder { path: "Archive".into() },
        })
        .await
        .unwrap();
        push.on_event(&inbox_event(account)).await.unwrap(); // the same mail again: nothing new

        let requests = got.lock().unwrap().clone();
        assert_eq!(requests.len(), 1, "only the new, unread, recent email of the inbox");
        let (head, body) = &requests[0];
        let head = head.to_lowercase();
        assert!(head.contains("content-encoding: aes128gcm") && head.contains("ttl: 86400"), "{head}");
        assert!(head.contains("authorization: vapid t="), "{head}");
        let note = shown(body);
        assert_eq!(note["title"], "Claire Dubois");
        assert_eq!(note["body"], "Kick-off planning");
        assert_eq!(note["url"], format!("/mail?account={account}&folder=INBOX&uid=2"));
        // Its buttons, and what they act on.
        assert_eq!(note["actions"][0], serde_json::json!({"action": "archive", "title": "Archive"}));
        assert_eq!(note["actions"][1]["action"], "read");
        assert_eq!(
            note["data"],
            serde_json::json!({"kind": "mail", "account": account, "folder": "INBOX", "uid": 2})
        );
        assert!(push.devices(UserId::ADMIN, Some(&endpoint)).await.unwrap()[0].last_ok_at.is_some());
    }

    #[test]
    fn each_notification_has_its_buttons() {
        let names = |n: &Notification| n.actions.iter().map(|b| b.action.clone()).collect::<Vec<_>>();
        let task = task_notification(7, "Call Thomas");
        assert_eq!(names(&task), ["done", "snooze-60"]);
        assert_eq!(task.actions[1].title, "In 1 hour");
        assert_eq!(task.data, Some(Target::Task { task: 7 }));
        assert!(task.renotify);

        let when = When { start: 1_791_230_400, all_day: false, tz: chrono_tz::Tz::UTC };
        let online = event_notification(4, "Kick-off", &when, "", Some("https://meet.example/abc"));
        assert_eq!(names(&online), ["join", "snooze-10"]);
        assert_eq!(online.join_url.as_deref(), Some("https://meet.example/abc"));
        assert_eq!(online.data, Some(Target::Event { event: 4, start: 1_791_230_400 }));
        // No link, or not a web page: no *Join*.
        for link in [None, Some("javascript:alert(1)"), Some("http://meet.example/abc")] {
            let note = event_notification(4, "Kick-off", &when, "", link);
            assert_eq!(names(&note), ["snooze-5", "snooze-10"]);
            assert_eq!(note.join_url, None);
        }

        let mail: Vec<(i64, String, String)> =
            (1..=5).map(|i| (i, "Claire".to_owned(), "s".to_owned())).collect();
        let summary = &mail_notifications(1, "INBOX", &mail, true)[0];
        assert_eq!(names(summary), ["read"]);
        assert_eq!(summary.actions[0].title, "Mark all as read");
        assert_eq!(
            summary.data,
            Some(Target::Inbox { account: 1, folder: "INBOX".into(), uids: vec![1, 2, 3, 4, 5] })
        );

        // Without buttons, the JSON stays as it was.
        let plain =
            Notification { title: "t".into(), url: "/".into(), tag: "x".into(), ..Default::default() };
        let json = serde_json::to_value(&plain).unwrap();
        assert_eq!(json, serde_json::json!({"title": "t", "body": "", "url": "/", "tag": "x"}));
    }

    #[tokio::test]
    async fn mail_buttons_only_act_on_mail_still_there() {
        let db = Database::in_memory().await.unwrap();
        let account = db.insert_account(UserId::ADMIN, &new_account("me@x.test")).await.unwrap().id;
        let store = db.store(account);
        store.replace_folders(&[folder("INBOX", FolderRole::Inbox)]).await.unwrap();
        store
            .upsert_messages("INBOX", &[mail(1, "A", "a", true, 0), mail(3, "C", "c", true, 0)])
            .await
            .unwrap();
        let push = new_push(db);
        let one = |uid| Target::Mail { account, folder: "INBOX".into(), uid };
        assert_eq!(push.still_there(UserId::ADMIN, &one(1)).await.unwrap(), one(1));
        assert!(push.still_there(UserId::ADMIN, &one(2)).await.is_err(), "archived elsewhere since");
        let all = Target::Inbox { account, folder: "INBOX".into(), uids: vec![1, 2, 3] };
        assert_eq!(
            push.still_there(UserId::ADMIN, &all).await.unwrap(),
            Target::Inbox { account, folder: "INBOX".into(), uids: vec![1, 3] }
        );
        let none = Target::Inbox { account, folder: "INBOX".into(), uids: vec![2] };
        assert!(push.still_there(UserId::ADMIN, &none).await.is_err());
        assert_eq!(
            push.still_there(UserId::ADMIN, &Target::Task { task: 9 }).await.unwrap(),
            Target::Task { task: 9 }
        );
    }

    #[tokio::test]
    async fn snoozed_reminders_come_back_once_after_their_time() {
        let db = Database::in_memory().await.unwrap();
        let list: i64 =
            sqlx::query_scalar("SELECT id FROM task_lists LIMIT 1").fetch_one(&db.pool()).await.unwrap();
        let task: i64 = sqlx::query_scalar(
            "INSERT INTO tasks (list_id, remote_id, title, updated_at) VALUES (?, 'r1', 'Call Thomas', 0) RETURNING id",
        )
        .bind(list)
        .fetch_one(&db.pool())
        .await
        .unwrap();
        let calendar: i64 =
            sqlx::query_scalar("SELECT id FROM calendars LIMIT 1").fetch_one(&db.pool()).await.unwrap();
        let event = db
            .agenda(UserId::ADMIN)
            .save_event(
                calendar,
                &crate::agenda::RemoteEvent {
                    remote_id: "e1".into(),
                    title: "Kick-off".into(),
                    start: 1_791_230_400,
                    end: 1_791_234_000,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        let push = Push::new(db.clone(), tx);

        push.snooze(UserId::ADMIN, &Target::Task { task }, 60).await.unwrap();
        push.snooze(UserId::ADMIN, &Target::Event { event: event.id, start: event.start }, 10).await.unwrap();
        // Only reminders, and only things that exist.
        assert!(push.snooze(UserId::ADMIN, &Target::Task { task: 999 }, 10).await.is_err());
        assert!(
            push.snooze(UserId::ADMIN, &Target::Mail { account: 1, folder: "INBOX".into(), uid: 1 }, 10)
                .await
                .is_err()
        );

        let t = now();
        push.ring_snoozed(t + 5 * 60).await.unwrap();
        assert!(rx.try_recv().is_err(), "not yet");
        push.ring_snoozed(t + 10 * 60).await.unwrap();
        match rx.try_recv().unwrap().event {
            SyncEvent::EventReminder { event: id, title, start, .. } => {
                assert_eq!((id, title.as_str(), start), (event.id, "Kick-off", event.start));
            }
            other => panic!("{other:?}"),
        }
        assert!(rx.try_recv().is_err(), "the task's hour isn't over");
        push.ring_snoozed(t + 10 * 60).await.unwrap();
        assert!(rx.try_recv().is_err(), "once");

        // Done in the meantime: nothing to remind.
        sqlx::query("UPDATE tasks SET done_at = 1 WHERE id = ?")
            .bind(task)
            .execute(&db.pool())
            .await
            .unwrap();
        push.ring_snoozed(t + 61 * 60).await.unwrap();
        assert!(rx.try_recv().is_err());
        let left: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM push_snoozed").fetch_one(&db.pool()).await.unwrap();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn a_new_invitation_takes_the_place_of_its_email() {
        let db = Database::in_memory().await.unwrap();
        sqlx::query("UPDATE agenda_settings SET time_zone = 'Europe/Paris'")
            .execute(&db.pool())
            .await
            .unwrap();
        // The invitation is for mael@example.net, from Claire.
        let account = db.insert_account(UserId::ADMIN, &new_account("mael@example.net")).await.unwrap().id;
        let store = db.store(account);
        store.replace_folders(&[folder("INBOX", FolderRole::Inbox)]).await.unwrap();
        let push = new_push(db.clone());
        push.remember_inboxes().await.unwrap();
        let (endpoint, got) = push_service(vec![201, 201, 201]).await;
        subscribe(&push, &endpoint).await;

        // 1. Its headers arrive: an email notification.
        store
            .upsert_messages(
                "INBOX",
                &[mail(5, "Claire Dubois", "Kick-off", true, 0), mail(6, "Thomas", "Hi", true, 0)],
            )
            .await
            .unwrap();
        push.on_event(&inbox_event(account)).await.unwrap();
        assert_eq!(got.lock().unwrap().len(), 2);
        // 2. Their bodies are read: one is an invitation.
        let invite = include_bytes!("../mail/testdata/5-invitation.eml");
        store.store_body("INBOX", 5, invite).await.unwrap();
        store.store_body("INBOX", 6, b"Subject: Hi\r\n\r\nJust a note.\r\n").await.unwrap();
        push.on_event(&inbox_event(account)).await.unwrap();
        push.on_event(&inbox_event(account)).await.unwrap(); // once
        let requests = got.lock().unwrap().clone();
        assert_eq!(requests.len(), 3, "the invitation, and nothing for the plain email");
        let note = shown(&requests[2].1);
        assert_eq!(note["title"], "Invitation: Kick-off atelier Lyon");
        assert_eq!(note["body"], "Tue 20 Oct, 10:00 · from Claire Dubois");
        assert_eq!(note["tag"], format!("havre-mail-{account}-5"), "replaces the email's notification");
        assert_eq!(note["actions"][0]["action"], "accept");
        assert_eq!(note["actions"][1]["action"], "decline");
        assert_eq!(note["data"]["kind"], "invitation");
        assert_eq!(note["data"]["uid"], 5);
    }

    #[tokio::test]
    async fn invitations_follow_their_setting_and_skip_your_own() {
        let db = Database::in_memory().await.unwrap();
        // This time the organizer is you: nothing to answer.
        let account = db.insert_account(UserId::ADMIN, &new_account("claire@example.org")).await.unwrap().id;
        let store = db.store(account);
        store.replace_folders(&[folder("INBOX", FolderRole::Inbox)]).await.unwrap();
        let push = new_push(db.clone());
        push.remember_inboxes().await.unwrap();
        push.update_settings(
            UserId::ADMIN,
            &PushSettingsUpdate { new_mail: Some(false), ..Default::default() },
        )
        .await
        .unwrap();
        let (endpoint, got) = push_service(vec![201]).await;
        subscribe(&push, &endpoint).await;
        store.upsert_messages("INBOX", &[mail(5, "Claire", "Kick-off", true, 0)]).await.unwrap();
        push.on_event(&inbox_event(account)).await.unwrap();
        store.store_body("INBOX", 5, include_bytes!("../mail/testdata/5-invitation.eml")).await.unwrap();
        push.on_event(&inbox_event(account)).await.unwrap();
        assert!(got.lock().unwrap().is_empty(), "new mail off, and your own meeting");
        assert!(push.settings(UserId::ADMIN).await.unwrap().invitations, "on by default");
    }

    #[tokio::test]
    async fn reminders_follow_the_setting() {
        let db = Database::in_memory().await.unwrap();
        let push = new_push(db);
        let (endpoint, got) = push_service(vec![201]).await;
        subscribe(&push, &endpoint).await;
        let ring = AccountEvent {
            account: 0,
            user: UserId::ADMIN,
            event: SyncEvent::Reminder { task: 7, title: "Call Thomas".into() },
        };
        push.on_event(&ring).await.unwrap();
        push.update_settings(
            UserId::ADMIN,
            &PushSettingsUpdate { reminders: Some(false), ..Default::default() },
        )
        .await
        .unwrap();
        push.on_event(&ring).await.unwrap();
        let requests = got.lock().unwrap().clone();
        assert_eq!(requests.len(), 1, "the second one is off");
        assert_eq!(shown(&requests[0].1)["url"], "/tasks?task=7");
    }

    #[tokio::test]
    async fn a_failed_scheduled_message_is_pushed() {
        let db = Database::in_memory().await.unwrap();
        let push = new_push(db);
        let (endpoint, got) = push_service(vec![201]).await;
        subscribe(&push, &endpoint).await;
        let outbox = |sent: bool, error: Option<&str>| AccountEvent {
            account: 0,
            user: UserId::ADMIN,
            event: SyncEvent::Outbox {
                draft: 3,
                subject: "Quote".into(),
                sent,
                late: false,
                error: error.map(str::to_owned),
            },
        };
        // Sent, and scheduled: nothing to say.
        push.on_event(&outbox(true, None)).await.unwrap();
        push.on_event(&outbox(false, None)).await.unwrap();
        push.on_event(&outbox(false, Some("The server refused the password."))).await.unwrap();
        let requests = got.lock().unwrap().clone();
        assert_eq!(requests.len(), 1);
        let note = shown(&requests[0].1);
        assert_eq!(note["title"], "Not sent: Quote");
        assert_eq!(note["body"], "The server refused the password.");
    }

    #[tokio::test]
    async fn a_device_in_front_gets_nothing_and_a_gone_one_is_removed() {
        let db = Database::in_memory().await.unwrap();
        let push = new_push(db);
        let (endpoint, got) = push_service(vec![410]).await;
        subscribe(&push, &endpoint).await;
        let note = Notification { title: "t".into(), url: "/".into(), tag: "x".into(), ..Default::default() };

        push.seen(&endpoint);
        assert_eq!(push.notify(UserId::ADMIN, &note).await.unwrap(), 0);
        assert!(got.lock().unwrap().is_empty(), "Havre is open there: no push");

        push.in_front.lock().unwrap().clear();
        assert_eq!(push.notify(UserId::ADMIN, &note).await.unwrap(), 0);
        assert_eq!(got.lock().unwrap().len(), 1);
        assert!(push.devices(UserId::ADMIN, None).await.unwrap().is_empty(), "410: the subscription is gone");
    }

    #[tokio::test]
    async fn keeps_one_vapid_key_and_checks_subscriptions() {
        let db = Database::in_memory().await.unwrap();
        let first = new_push(db.clone()).public_key().await.unwrap();
        assert_eq!(new_push(db.clone()).public_key().await.unwrap(), first, "saved, encrypted, and reused");
        let push = new_push(db);
        let bad = |endpoint: &str, p256dh: &str| NewSubscription {
            endpoint: endpoint.into(),
            keys: SubscriptionKeys { p256dh: p256dh.into(), auth: AUTH.into() },
            label: String::new(),
        };
        assert!(
            push.subscribe(UserId::ADMIN, &bad("http://push.example/x", UA_PUBLIC)).await.is_err(),
            "HTTPS only"
        );
        assert!(
            push.subscribe(UserId::ADMIN, &bad("https://push.example/x", "nope")).await.is_err(),
            "a real key"
        );
        assert!(push.subscribe(UserId::ADMIN, &bad("https://push.example/x", UA_PUBLIC)).await.is_ok());
    }

    #[tokio::test]
    async fn event_reminders_follow_their_setting() {
        let db = Database::in_memory().await.unwrap();
        sqlx::query("UPDATE agenda_settings SET time_zone = 'Europe/Paris'")
            .execute(&db.pool())
            .await
            .unwrap();
        let push = new_push(db);
        let (endpoint, got) = push_service(vec![201]).await;
        subscribe(&push, &endpoint).await;
        let ring = AccountEvent {
            account: 0,
            user: UserId::ADMIN,
            event: SyncEvent::EventReminder {
                event: 4,
                title: "Kick-off".into(),
                start: 1_791_230_400, // Mon 5 Oct 2026, 20:00 UTC = 22:00 in Paris
                all_day: false,
                location: "Room 3".into(),
                join_url: None,
            },
        };
        push.on_event(&ring).await.unwrap();
        push.update_settings(
            UserId::ADMIN,
            &PushSettingsUpdate { event_reminders: Some(false), ..Default::default() },
        )
        .await
        .unwrap();
        push.on_event(&ring).await.unwrap();
        let requests = got.lock().unwrap().clone();
        assert_eq!(requests.len(), 1, "the second one is off");
        let note = shown(&requests[0].1);
        assert_eq!(
            (note["title"].as_str(), note["body"].as_str()),
            (Some("Kick-off"), Some("22:00 · Room 3"))
        );
        assert_eq!(note["url"], "/agenda?date=2026-10-05&event=4");
    }

    #[test]
    fn many_emails_make_one_notification() {
        let mail: Vec<(i64, String, String)> = (1..=5)
            .map(|i| (i, if i % 2 == 0 { "Claire" } else { "Thomas" }.to_owned(), format!("s{i}")))
            .collect();
        let notes = mail_notifications(1, "INBOX/Clients", &mail, true);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "5 new emails");
        assert_eq!(notes[0].body, "Thomas, Claire");
        assert_eq!(notes[0].url, "/mail?account=1&folder=INBOX%2FClients");
        let private = mail_notifications(1, "INBOX", &mail[..1], false);
        assert_eq!((private[0].title.as_str(), private[0].body.as_str()), ("New email", ""));
    }
}
