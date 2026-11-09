//! Push notifications to phones and browsers (Web Push), step 20.
//!
//! A browser that turns notifications on gives us a **subscription**: an
//! address at its push service (Google's for Chrome, Apple's for Safari,
//! Mozilla's for Firefox) and two keys. To notify it, we encrypt a small JSON
//! (`{title, body, url}`) for those keys and POST it to the address, signed
//! with Havre's own key ([`crypto`]). The browser wakes Havre's service worker
//! (`web/public/sw.js`), which shows it — even when no Havre page is open.
//!
//! What sends one ([`Push::start`]): a task reminder, and new mail in an inbox.
//! A device where Havre is open and in front says so every half minute
//! ([`Push::seen`]) and gets nothing: the page shows its own toast.

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
};
use crypto::{BrowserKeys, Vapid};

/// Who to contact about this server's pushes (the VAPID `sub` claim): the
/// project's page, rather than your address, which push services don't need.
const CONTACT: &str = "https://github.com/maelbel/havre";
/// A device that said "I'm in front" less than this ago gets no push.
const IN_FRONT: Duration = Duration::from_secs(70);
/// More new emails than this at once: one notification for all of them.
const SUMMARY_ABOVE: usize = 3;

/// What the service worker shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Notification {
    pub title: String,
    pub body: String,
    /// The page to open on click (`/mail?account=…`).
    pub url: String,
    /// A notification with the same tag replaces the previous one.
    pub tag: String,
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
}

/// A partial update: only the fields that are present change.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PushSettingsUpdate {
    pub new_mail: Option<bool>,
    pub mail_details: Option<bool>,
    pub reminders: Option<bool>,
    pub event_reminders: Option<bool>,
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
}

impl Push {
    /// Starts listening to Havre's events (new mail, reminders) and pushes.
    pub fn start(db: Database, events: &broadcast::Sender<AccountEvent>) -> Arc<Self> {
        let push = Arc::new(Self::new(db));
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

    fn new(db: Database) -> Self {
        Self {
            db,
            http: reqwest::Client::builder().timeout(Duration::from_secs(20)).build().expect("HTTP client"),
            vapid: OnceCell::new(),
            in_front: Mutex::new(HashMap::new()),
            seen_uids: tokio::sync::Mutex::new(HashMap::new()),
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
    pub async fn subscribe(&self, s: &NewSubscription) -> Result<()> {
        check_endpoint(&s.endpoint)?;
        BrowserKeys::from_base64(&s.keys.p256dh, &s.keys.auth)?;
        let label: String = s.label.trim().chars().take(80).collect();
        sqlx::query(
            "INSERT INTO push_subscriptions (endpoint, p256dh, auth, label, created_at) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (endpoint) DO UPDATE SET p256dh = excluded.p256dh, auth = excluded.auth,
                 label = excluded.label, last_error = NULL",
        )
        .bind(&s.endpoint)
        .bind(&s.keys.p256dh)
        .bind(&s.keys.auth)
        .bind(label)
        .bind(now())
        .execute(&self.pool())
        .await?;
        Ok(())
    }

    /// Forgets a browser (it turned notifications off).
    pub async fn unsubscribe(&self, endpoint: &str) -> Result<()> {
        sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = ?")
            .bind(endpoint)
            .execute(&self.pool())
            .await?;
        Ok(())
    }

    /// Every device; `this_device` marks the one with `endpoint`.
    pub async fn devices(&self, endpoint: Option<&str>) -> Result<Vec<Device>> {
        let rows = sqlx::query(
            "SELECT id, endpoint, label, created_at, last_ok_at, last_error FROM push_subscriptions ORDER BY id",
        )
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

    pub async fn remove_device(&self, id: i64) -> Result<()> {
        let done =
            sqlx::query("DELETE FROM push_subscriptions WHERE id = ?").bind(id).execute(&self.pool()).await?;
        if done.rows_affected() == 0 {
            return Err(Error::NotFound(format!("device {id}")));
        }
        Ok(())
    }

    pub async fn settings(&self) -> Result<PushSettings> {
        let r = sqlx::query(
            "SELECT new_mail, mail_details, reminders, event_reminders FROM push_settings WHERE id = 1",
        )
        .fetch_one(&self.pool())
        .await?;
        Ok(PushSettings {
            new_mail: r.get("new_mail"),
            mail_details: r.get("mail_details"),
            reminders: r.get("reminders"),
            event_reminders: r.get("event_reminders"),
        })
    }

    pub async fn update_settings(&self, u: &PushSettingsUpdate) -> Result<PushSettings> {
        // `COALESCE(?, column)`: a missing field (NULL) keeps what's there.
        sqlx::query(
            "UPDATE push_settings SET new_mail = COALESCE(?, new_mail), mail_details = COALESCE(?, mail_details),
             reminders = COALESCE(?, reminders), event_reminders = COALESCE(?, event_reminders) WHERE id = 1",
        )
        .bind(u.new_mail)
        .bind(u.mail_details)
        .bind(u.reminders)
        .bind(u.event_reminders)
        .execute(&self.pool())
        .await?;
        self.settings().await
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
    pub async fn test(&self, endpoint: &str) -> Result<()> {
        let note = Notification {
            title: "Havre".into(),
            body: "Notifications work on this device.".into(),
            url: "/settings/notifications".into(),
            tag: "havre-test".into(),
        };
        let row = sqlx::query("SELECT id, endpoint, p256dh, auth FROM push_subscriptions WHERE endpoint = ?")
            .bind(endpoint)
            .fetch_optional(&self.pool())
            .await?
            .ok_or_else(|| Error::Config("Turn notifications on for this device first.".into()))?;
        self.send(&row, &note).await
    }

    /// Sends `note` to every device not showing Havre in front. Returns how
    /// many got it.
    pub async fn notify(&self, note: &Notification) -> Result<usize> {
        let rows = sqlx::query("SELECT id, endpoint, p256dh, auth FROM push_subscriptions")
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
                self.unsubscribe(&endpoint).await?;
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

    async fn on_event(&self, event: &AccountEvent) -> Result<()> {
        match &event.event {
            SyncEvent::Reminder { task, title } => {
                if self.settings().await?.reminders {
                    self.notify(&Notification {
                        title: "Reminder".into(),
                        body: title.clone(),
                        url: format!("/tasks?task={task}"),
                        tag: format!("havre-task-{task}"),
                    })
                    .await?;
                }
            }
            SyncEvent::EventReminder { event: id, title, start, all_day, location, .. } => {
                if self.settings().await?.event_reminders {
                    let tz = self.time_zone().await?;
                    self.notify(&event_notification(*id, title, *start, *all_day, location, tz)).await?;
                }
            }
            // A scheduled message couldn't go: always said (you may count on it).
            SyncEvent::Outbox { draft, subject, sent: false, error: Some(error), .. } => {
                let subject = if subject.trim().is_empty() { "(no subject)" } else { subject.trim() };
                self.notify(&Notification {
                    title: format!("Not sent: {subject}"),
                    body: error.chars().take(160).collect(),
                    url: "/mail?view=scheduled".into(),
                    tag: format!("havre-outbox-{draft}"),
                })
                .await?;
            }
            SyncEvent::Folder { path } => {
                let notes = self.new_mail(event.account, path).await?;
                if self.settings().await?.new_mail {
                    for note in notes {
                        self.notify(&note).await?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Your time zone, as the Agenda knows it from your browser (UTC until then).
    async fn time_zone(&self) -> Result<chrono_tz::Tz> {
        let zone: Option<String> = sqlx::query_scalar("SELECT time_zone FROM agenda_settings WHERE id = 1")
            .fetch_one(&self.pool())
            .await?;
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
    async fn new_mail(&self, account: i64, folder: &str) -> Result<Vec<Notification>> {
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
        let details = self.settings().await?.mail_details;
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
}

/// An event reminder: "Kick-off" / "10:00 · Room 3" (your time), opening the
/// event in the Agenda.
fn event_notification(
    id: i64,
    title: &str,
    start: i64,
    all_day: bool,
    location: &str,
    tz: chrono_tz::Tz,
) -> Notification {
    use chrono::TimeZone;
    // An all-day event's date is its UTC date; a timed one's, the date where you are.
    let (date, when) = if all_day {
        let day = chrono::DateTime::from_timestamp(start, 0).unwrap_or_default().date_naive();
        (day, format!("All day, {}", day.format("%a %-d %b")))
    } else {
        let local = tz.timestamp_opt(start, 0).single().unwrap_or_else(|| tz.timestamp_opt(0, 0).unwrap());
        (local.date_naive(), local.format("%H:%M").to_string())
    };
    let body = if location.trim().is_empty() { when } else { format!("{when} · {}", location.trim()) };
    Notification {
        title: title.to_owned(),
        body,
        url: format!("/agenda?date={}&event={id}", date.format("%Y-%m-%d")),
        tag: format!("havre-event-{id}-{start}"),
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
    use crate::{
        mail::{FolderRole, MessageSummary, Person},
        store::tests::{folder, new_account},
    };
    use crypto::tests::{AUTH, UA_PRIVATE, UA_PUBLIC, b64, open};

    type Requests = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

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
        push.subscribe(&sub).await.unwrap();
    }

    /// What the phone would show: the request's body, decrypted.
    fn shown(body: &[u8]) -> serde_json::Value {
        let ua = p256::SecretKey::from_slice(&b64(UA_PRIVATE)).unwrap();
        serde_json::from_slice(&open(body, &ua, &b64(AUTH))).unwrap()
    }

    fn inbox_event(account: i64) -> AccountEvent {
        AccountEvent { account, event: SyncEvent::Folder { path: "INBOX".into() } }
    }

    #[tokio::test]
    async fn new_mail_is_pushed_once_and_old_mail_never() {
        let db = Database::in_memory().await.unwrap();
        let account = db.insert_account(&new_account("me@x.test")).await.unwrap().id;
        let store = db.store(account);
        store
            .replace_folders(&[folder("INBOX", FolderRole::Inbox), folder("Archive", FolderRole::Archive)])
            .await
            .unwrap();
        store.upsert_messages("INBOX", &[mail(1, "Old", "Already there", true, 1)]).await.unwrap();

        let push = Push::new(db.clone());
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
        push.on_event(&AccountEvent { account, event: SyncEvent::Folder { path: "Archive".into() } })
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
        assert!(push.devices(Some(&endpoint)).await.unwrap()[0].last_ok_at.is_some());
    }

    #[tokio::test]
    async fn reminders_follow_the_setting() {
        let db = Database::in_memory().await.unwrap();
        let push = Push::new(db);
        let (endpoint, got) = push_service(vec![201]).await;
        subscribe(&push, &endpoint).await;
        let ring =
            AccountEvent { account: 0, event: SyncEvent::Reminder { task: 7, title: "Call Thomas".into() } };
        push.on_event(&ring).await.unwrap();
        push.update_settings(&PushSettingsUpdate { reminders: Some(false), ..Default::default() })
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
        let push = Push::new(db);
        let (endpoint, got) = push_service(vec![201]).await;
        subscribe(&push, &endpoint).await;
        let outbox = |sent: bool, error: Option<&str>| AccountEvent {
            account: 0,
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
        let push = Push::new(db);
        let (endpoint, got) = push_service(vec![410]).await;
        subscribe(&push, &endpoint).await;
        let note = Notification { title: "t".into(), body: "b".into(), url: "/".into(), tag: "x".into() };

        push.seen(&endpoint);
        assert_eq!(push.notify(&note).await.unwrap(), 0);
        assert!(got.lock().unwrap().is_empty(), "Havre is open there: no push");

        push.in_front.lock().unwrap().clear();
        assert_eq!(push.notify(&note).await.unwrap(), 0);
        assert_eq!(got.lock().unwrap().len(), 1);
        assert!(push.devices(None).await.unwrap().is_empty(), "410: the subscription is gone");
    }

    #[tokio::test]
    async fn keeps_one_vapid_key_and_checks_subscriptions() {
        let db = Database::in_memory().await.unwrap();
        let first = Push::new(db.clone()).public_key().await.unwrap();
        assert_eq!(Push::new(db.clone()).public_key().await.unwrap(), first, "saved, encrypted, and reused");
        let push = Push::new(db);
        let bad = |endpoint: &str, p256dh: &str| NewSubscription {
            endpoint: endpoint.into(),
            keys: SubscriptionKeys { p256dh: p256dh.into(), auth: AUTH.into() },
            label: String::new(),
        };
        assert!(push.subscribe(&bad("http://push.example/x", UA_PUBLIC)).await.is_err(), "HTTPS only");
        assert!(push.subscribe(&bad("https://push.example/x", "nope")).await.is_err(), "a real key");
        assert!(push.subscribe(&bad("https://push.example/x", UA_PUBLIC)).await.is_ok());
    }

    #[tokio::test]
    async fn event_reminders_follow_their_setting() {
        let db = Database::in_memory().await.unwrap();
        sqlx::query("UPDATE agenda_settings SET time_zone = 'Europe/Paris'")
            .execute(&db.pool())
            .await
            .unwrap();
        let push = Push::new(db);
        let (endpoint, got) = push_service(vec![201]).await;
        subscribe(&push, &endpoint).await;
        let ring = AccountEvent {
            account: 0,
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
        push.update_settings(&PushSettingsUpdate { event_reminders: Some(false), ..Default::default() })
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
