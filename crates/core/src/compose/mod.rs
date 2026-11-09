//! Writing mail: drafts, replies, forwards, and sending over SMTP.
//!
//! - A message being written is a **draft**, saved in the database as you
//!   type (nothing is lost if the page closes). Replies and forwards start
//!   as a draft filled from the original message.
//! - **Sending** builds a standard MIME message with `lettre` (plain text +
//!   a simple HTML version, plus attachments) and hands it to the account's
//!   outgoing server (SMTP), logging in like IMAP does: password, or OAuth2
//!   token (`XOAUTH2`).
//! - Then a copy goes into the "Sent" folder (`APPEND`) — except for Gmail
//!   and Microsoft 365, whose servers file sent mail themselves.
//! - **On the server** (step 17): each draft also has a copy in its account's
//!   Drafts folder, so other mail apps see it. A background loop writes it
//!   15 s after the last change (closing the window writes it at once); each
//!   version replaces the previous one, found by the draft's own
//!   `Message-ID`. Havre's copy stays the reference: if the server fails,
//!   nothing is lost and Havre tries again later.
//! - **Signatures** (step 24): each account's, put in new drafts (and
//!   replies), swapped when "From" changes.
//! - **Undo send, send later** (step 24): see [`outbox`].

pub mod html;
pub mod outbox;

use std::{str::FromStr, time::Duration};

use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Attachment as MimeAttachment, Mailbox, Mailboxes, MultiPart, SinglePart, header::ContentType},
    transport::smtp::{
        authentication::{Credentials, Mechanism},
        client::{Certificate, Tls, TlsParameters},
    },
};
use mail_parser::{AddressList, MessageParser, TextList};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use crate::{
    Error, Result,
    accounts::AccountManager,
    config::ImapAuth,
    mail::{self, AttachmentData, FolderRole, MessageDetail, RenderOptions},
};

// ---- Outgoing server ----------------------------------------------------------------

/// How the connection to the SMTP server is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// TLS from the first byte (port 465).
    Tls,
    /// Plain connection upgraded with `STARTTLS` (port 587). Havre refuses
    /// to continue if the server can't upgrade.
    StartTls,
}

impl Security {
    pub fn parse(value: &str) -> Self {
        if value == "starttls" { Self::StartTls } else { Self::Tls }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::StartTls => "starttls",
        }
    }
}

/// The usual outgoing server for an incoming one.
pub fn smtp_defaults(imap_host: &str) -> (String, u16, Security) {
    let host = imap_host.trim().to_lowercase();
    match host.as_str() {
        "outlook.office365.com" | "imap-mail.outlook.com" => {
            ("smtp.office365.com".into(), 587, Security::StartTls)
        }
        "imap.gmail.com" => ("smtp.gmail.com".into(), 465, Security::Tls),
        "imap.mail.me.com" => ("smtp.mail.me.com".into(), 587, Security::StartTls),
        "imap.mail.yahoo.com" => ("smtp.mail.yahoo.com".into(), 465, Security::Tls),
        "imap.gmx.com" | "imap.gmx.net" => ("mail.gmx.com".into(), 465, Security::Tls),
        "127.0.0.1" | "localhost" => (host.clone(), 1025, Security::StartTls), // Proton Mail Bridge
        _ => match host.strip_prefix("imap.") {
            Some(domain) => (format!("smtp.{domain}"), 465, Security::Tls),
            // e.g. OVH's ssl0.ovh.net: same server for both.
            None => (host.clone(), 465, Security::Tls),
        },
    }
}

/// Everything needed to send through one account.
#[derive(Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    pub auth: ImapAuth,
    pub ca_file: Option<String>,
}

/// Hands a message to the outgoing server.
pub async fn send(config: &SmtpConfig, message: &Message) -> Result<()> {
    let mut tls = TlsParameters::builder(config.host.clone());
    if let Some(path) = &config.ca_file {
        let pem = std::fs::read(path)?;
        tls = tls.add_root_certificate(
            Certificate::from_pem(&pem).map_err(|e| Error::Config(format!("{path}: {e}")))?,
        );
    }
    let tls = tls.build().map_err(|e| Error::Config(format!("TLS: {e}")))?;
    let (secret, mechanisms) = match &config.auth {
        ImapAuth::Password(password) => (password.clone(), vec![Mechanism::Plain, Mechanism::Login]),
        ImapAuth::Bearer(token) => (token.clone(), vec![Mechanism::Xoauth2]),
        ImapAuth::OAuth(session) => (session.access_token().await?, vec![Mechanism::Xoauth2]),
    };
    let transport = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
        .port(config.port)
        .tls(match config.security {
            Security::Tls => Tls::Wrapper(tls),
            Security::StartTls => Tls::Required(tls),
        })
        .credentials(Credentials::new(config.username.clone(), secret))
        .authentication(mechanisms)
        .timeout(Some(Duration::from_secs(30)))
        .build();
    transport.send(message.clone()).await.map(|_| ()).map_err(|err| explain_smtp(config, &err))
}

/// SMTP errors, for humans.
fn explain_smtp(config: &SmtpConfig, err: &lettre::transport::smtp::Error) -> Error {
    let text = err.to_string();
    let lower = text.to_lowercase();
    let server = format!("{}:{}", config.host, config.port);
    let microsoft = config.host.ends_with("office365.com") || config.host.ends_with("outlook.com");
    let message = if lower.contains("no compatible authentication mechanism") {
        format!(
            "{server} doesn't offer a login method Havre supports. If it's port 25 or 587 without \
             encryption, use 465 (TLS) or 587 with STARTTLS."
        )
    } else if lower.contains("535") || lower.contains("authentication") || lower.contains("auth") {
        if microsoft && matches!(config.auth, ImapAuth::OAuth(_)) {
            "Microsoft refused to send: “Authenticated SMTP” is probably turned off for this mailbox. Your \
             admin can turn it on (Microsoft 365 admin center › Users › the user › Mail › Manage email apps \
             › Authenticated SMTP)."
                .to_owned()
        } else if microsoft {
            "Microsoft refused the password for sending: use Authentication: OAuth2 for this account."
                .to_owned()
        } else {
            format!("{server} refused the login for sending. Check the outgoing server settings ({text}).")
        }
    } else if lower.contains("failed to lookup") || lower.contains("dns") || lower.contains("resolve") {
        format!(
            "Outgoing server “{}” not found: check it in Settings › the account › Server settings.",
            config.host
        )
    } else if lower.contains("connection refused") || lower.contains("timed out") || lower.contains("timeout")
    {
        format!(
            "Can't reach the outgoing server {server}. Check the port (465 with TLS, 587 with STARTTLS) and \
             that your network allows it."
        )
    } else if lower.contains("certificate") || lower.contains("invalid peer") {
        format!("The certificate of {} is not trusted: {text}", config.host)
    } else if err.is_permanent() {
        format!("The server refused the message: {text}")
    } else {
        format!("Couldn't send: {text}")
    };
    Error::Config(message)
}

// ---- Drafts --------------------------------------------------------------------------

/// What a draft is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftKind {
    New,
    Reply,
    ReplyAll,
    Forward,
}

impl DraftKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Reply => "reply",
            Self::ReplyAll => "reply_all",
            Self::Forward => "forward",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "reply" => Self::Reply,
            "reply_all" => Self::ReplyAll,
            "forward" => Self::Forward,
            _ => Self::New,
        }
    }
}

/// A message being written, as the UI sees it.
#[derive(Debug, Clone, Serialize)]
pub struct Draft {
    pub id: i64,
    /// Sent from this account.
    pub account: i64,
    pub kind: DraftKind,
    /// Comma-separated, e.g. `Anna <anna@example.com>, bob@example.com`.
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    /// Plain text. Replies start with the quoted original (`> …`). For a
    /// formatted draft, made from `body_html` by Havre.
    pub body: String,
    /// The formatted text (clean HTML); `None` for a plain-text draft.
    pub body_html: Option<String>,
    pub attachments: Vec<DraftAttachment>,
    pub updated_at: i64,
    /// When its copy was last saved in the server's Drafts folder.
    pub server_saved_at: Option<i64>,
    /// Changed since then: the server copy is behind.
    pub server_pending: bool,
    /// Why the last save to the server failed.
    pub server_error: Option<String>,
    /// The signature as it was put in (to swap it when "From" changes).
    pub signature: String,
    /// When it will be sent: soon (undo send) or later (send later).
    pub send_at: Option<i64>,
    /// `waiting`, `sending` or `failed` (with `send_error`), when `send_at` is set.
    pub send_state: Option<String>,
    pub send_error: Option<String>,
    #[serde(skip)]
    in_reply_to: Option<String>,
    #[serde(skip)]
    references: Option<String>,
    #[serde(skip)]
    source: Option<(String, u32)>,
    #[serde(skip)]
    server: ServerCopy,
}

impl Draft {
    /// Nothing written yet (the signature Havre put in doesn't count).
    fn is_empty(&self) -> bool {
        let body = match find_signature(&self.body, &self.signature) {
            Some(at) => {
                format!("{}{}", &self.body[..at], &self.body[at + signature_block(&self.signature).len()..])
            }
            None => self.body.clone(),
        };
        [&self.to, &self.cc, &self.bcc, &self.subject, &body].iter().all(|f| f.trim().is_empty())
            && self.attachments.is_empty()
    }
}

/// Where a draft's copy on the mail server is, and which version it holds.
#[derive(Debug, Clone, Default, PartialEq)]
struct ServerCopy {
    message_id: Option<String>,
    version: i64,
    saved_version: Option<i64>,
    account: Option<i64>,
    folder: Option<String>,
    uid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DraftAttachment {
    pub id: i64,
    pub filename: String,
    pub mime: String,
    pub size: i64,
}

/// What the UI changes while you type. Only present fields change.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DraftUpdate {
    pub account: Option<i64>,
    pub to: Option<String>,
    pub cc: Option<String>,
    pub bcc: Option<String>,
    pub subject: Option<String>,
    /// Plain text (a plain-text draft).
    pub body: Option<String>,
    /// Formatted text (a rich draft): cleaned, and its plain text made from it.
    pub body_html: Option<String>,
    /// Turn the draft into a rich one (`true`, its text as paragraphs) or a
    /// plain one (`false`, its formatting dropped).
    pub rich: Option<bool>,
}

/// The largest attachment total accepted (most servers refuse ~25 MB).
pub const MAX_ATTACHMENTS: usize = 25 * 1024 * 1024;

/// A draft is saved on the server once it hasn't changed for this long…
pub(crate) const SERVER_QUIET: i64 = 15;
/// …and after a failure, tried again this much later.
pub(crate) const SERVER_RETRY: i64 = 120;

/// Drafts in the database.
#[derive(Clone)]
pub struct Drafts {
    pool: SqlitePool,
}

/// A new draft before it's saved: empty, or filled from a message.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Prefill {
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
    /// The same, formatted (replies and forwards: the original's HTML, cleaned).
    pub body_html: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    /// A draft finished from another app: taken as it is (no signature added).
    pub as_is: bool,
}

impl Drafts {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn insert(
        &self,
        account: i64,
        kind: DraftKind,
        prefill: &Prefill,
        source: Option<(&str, u32)>,
    ) -> Result<i64> {
        let now = now();
        // The account's signature, where it goes for this kind of message.
        let (text, html, in_replies) = self.signature_of(account).await?;
        let signature = if prefill.as_is { String::new() } else { signature_for(kind, &text, in_replies) };
        let rich = self.mail_format_rich().await?;
        let (body, body_html) = if rich {
            // Two empty paragraphs to write in? One is enough in an editor.
            let quote = prefill.body_html.clone().unwrap_or_else(|| html::from_text(&prefill.body));
            let sig = if signature.is_empty() { String::new() } else { html::signature_block(&html) };
            let body_html = if prefill.as_is { quote } else { format!("<p></p>{sig}{quote}") };
            (html::to_text(&body_html), Some(body_html))
        } else {
            (with_signature(&prefill.body, &signature), None)
        };
        Ok(sqlx::query_scalar(
            "INSERT INTO drafts (account_id, kind, to_addrs, cc_addrs, subject, body, in_reply_to, refs,
                                 source_folder, source_uid, created_at, updated_at, signature, body_html)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(account)
        .bind(kind.as_str())
        .bind(&prefill.to)
        .bind(&prefill.cc)
        .bind(&prefill.subject)
        .bind(&body)
        .bind(&prefill.in_reply_to)
        .bind(&prefill.references)
        .bind(source.map(|s| s.0))
        .bind(source.map(|s| s.1 as i64))
        .bind(now)
        .bind(now)
        .bind(&signature)
        .bind(&body_html)
        .fetch_one(&self.pool)
        .await?)
    }

    /// An account's signature: plain text, formatted, and whether replies get it.
    async fn signature_of(&self, account: i64) -> Result<(String, String, bool)> {
        let row: Option<(String, String, bool)> = sqlx::query_as(
            "SELECT signature, signature_html, signature_in_replies FROM accounts WHERE id = ?",
        )
        .bind(account)
        .fetch_optional(&self.pool)
        .await?;
        let (text, html, in_replies) = row.unwrap_or_default();
        // Written before rich text: its HTML is made from the text.
        let html = if html.is_empty() && !text.is_empty() { html::from_text(&text) } else { html };
        Ok((text, html, in_replies))
    }

    /// New messages are written formatted (the default) or as plain text.
    async fn mail_format_rich(&self) -> Result<bool> {
        let format: String = sqlx::query_scalar("SELECT compose_format FROM mail_settings WHERE id = 1")
            .fetch_one(&self.pool)
            .await?;
        Ok(format != "plain")
    }

    pub async fn get(&self, id: i64) -> Result<Draft> {
        let row = sqlx::query("SELECT * FROM drafts WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| Error::NotFound(format!("draft {id}")))?;
        let mut draft = draft_from_row(&row);
        draft.attachments = self.attachments(id).await?;
        Ok(draft)
    }

    /// Every draft, most recent first.
    pub async fn list(&self) -> Result<Vec<Draft>> {
        let rows = sqlx::query("SELECT * FROM drafts ORDER BY updated_at DESC").fetch_all(&self.pool).await?;
        let mut drafts: Vec<Draft> = rows.iter().map(draft_from_row).collect();
        for draft in &mut drafts {
            draft.attachments = self.attachments(draft.id).await?;
        }
        Ok(drafts)
    }

    pub async fn update(&self, id: i64, update: &DraftUpdate) -> Result<Draft> {
        let mut body = update.body.clone();
        // Formatted text: cleaned (never trust what a page sends), its plain text made from it.
        let mut body_html: Option<Option<String>> = update.body_html.as_deref().map(|h| Some(html::clean(h)));
        let needs_current = update.account.is_some() || update.rich.is_some();
        let current = if needs_current { Some(self.get(id).await?) } else { None };
        // Rich or plain, from now on.
        if let (Some(rich), Some(current)) = (update.rich, &current) {
            match (rich, &current.body_html) {
                (true, None) => {
                    let text = body.as_deref().unwrap_or(&current.body);
                    body_html = Some(Some(html::from_text(text)));
                }
                (false, Some(_)) => body_html = Some(None),
                _ => {}
            }
        }
        // "From" changes: the signature follows, unless you edited it.
        let mut signature = None;
        if let (Some(account), Some(current)) = (update.account, &current)
            && current.account != account
        {
            let (text, sig_html, in_replies) = self.signature_of(account).await?;
            let new = signature_for(current.kind, &text, in_replies);
            match body_html.clone().unwrap_or(current.body_html.clone()) {
                Some(rich) => {
                    let new_html = if new.is_empty() { String::new() } else { sig_html };
                    let (swapped, kept) =
                        swap_signature_html(&rich, current.kind, &current.signature, &new, &new_html);
                    body_html = Some(Some(swapped));
                    signature = Some(kept);
                }
                None => {
                    let text = body.as_deref().unwrap_or(&current.body);
                    let (swapped, kept) = swap_signature(text, current.kind, &current.signature, &new);
                    body = Some(swapped);
                    signature = Some(kept);
                }
            }
        }
        // A formatted draft's plain text always comes from its HTML.
        if let Some(Some(h)) = &body_html {
            body = Some(html::to_text(h));
        }
        // Each change counts (`version`): the server copy is behind until
        // it holds the newest one.
        let mut sql =
            sqlx::QueryBuilder::<sqlx::Sqlite>::new("UPDATE drafts SET version = version + 1, updated_at = ");
        sql.push_bind(now());
        let fields = [
            ("to_addrs", &update.to),
            ("cc_addrs", &update.cc),
            ("bcc_addrs", &update.bcc),
            ("subject", &update.subject),
            ("body", &body),
            ("signature", &signature),
        ];
        let body_html_set = body_html.is_some();
        let body_html = body_html.flatten();
        for (column, value) in fields {
            if let Some(value) = value {
                sql.push(format!(", {column} = ")).push_bind(value.clone());
            }
        }
        if body_html_set {
            sql.push(", body_html = ").push_bind(body_html);
        }
        if let Some(account) = update.account {
            sql.push(", account_id = ").push_bind(account);
        }
        sql.push(" WHERE id = ").push_bind(id);
        if sql.build().execute(&self.pool).await?.rows_affected() == 0 {
            return Err(Error::NotFound(format!("draft {id}")));
        }
        self.get(id).await
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM drafts WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    async fn attachments(&self, draft: i64) -> Result<Vec<DraftAttachment>> {
        let rows: Vec<(i64, String, String, i64)> = sqlx::query_as(
            "SELECT id, filename, mime, length(data) FROM draft_attachments WHERE draft_id = ? ORDER BY id",
        )
        .bind(draft)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, filename, mime, size)| DraftAttachment { id, filename, mime, size })
            .collect())
    }

    pub async fn add_attachment(&self, draft: i64, file: &AttachmentData) -> Result<DraftAttachment> {
        let used: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(length(data)), 0) FROM draft_attachments WHERE draft_id = ?",
        )
        .bind(draft)
        .fetch_one(&self.pool)
        .await?;
        if used as usize + file.bytes.len() > MAX_ATTACHMENTS {
            return Err(Error::Config("Attachments are limited to 25 MB per message.".into()));
        }
        let filename = sanitize_filename(&file.filename);
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO draft_attachments (draft_id, filename, mime, data) VALUES (?, ?, ?, ?) RETURNING id",
        )
        .bind(draft)
        .bind(&filename)
        .bind(&file.mime)
        .bind(&file.bytes)
        .fetch_one(&self.pool)
        .await?;
        self.touch(draft).await?;
        Ok(DraftAttachment { id, filename, mime: file.mime.clone(), size: file.bytes.len() as i64 })
    }

    pub async fn remove_attachment(&self, draft: i64, attachment: i64) -> Result<()> {
        sqlx::query("DELETE FROM draft_attachments WHERE id = ? AND draft_id = ?")
            .bind(attachment)
            .bind(draft)
            .execute(&self.pool)
            .await?;
        self.touch(draft).await
    }

    /// A change that isn't a field (an attachment added or removed).
    async fn touch(&self, draft: i64) -> Result<()> {
        sqlx::query("UPDATE drafts SET version = version + 1, updated_at = ? WHERE id = ?")
            .bind(now())
            .bind(draft)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---- The copy on the server ----------------------------------------------------------

    /// Drafts whose server copy is behind, quiet for `SERVER_QUIET` seconds
    /// and not tried in the last `SERVER_RETRY` (oldest first). An empty one
    /// (a window just opened) isn't worth a copy.
    pub(crate) async fn due_on_server(&self, now: i64) -> Result<Vec<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM drafts
             WHERE (server_version IS NULL OR server_version <> version OR server_account <> account_id)
               AND updated_at <= ? AND (server_tried_at IS NULL OR server_tried_at <= ?)
               -- Waiting to be sent: kept out of the Drafts folder (see `outbox`).
               AND send_at IS NULL
               AND (to_addrs <> '' OR cc_addrs <> '' OR bcc_addrs <> '' OR trim(subject) <> ''
                    -- Only the signature Havre put in isn't something written. (`trim`'s
                    -- second argument: what to strip, here line breaks and tabs too.)
                    OR (trim(body, ' ' || char(9, 10, 13)) <> ''
                        AND trim(body, ' ' || char(9, 10, 13)) <> '-- ' || char(10) || signature)
                    OR EXISTS (SELECT 1 FROM draft_attachments a WHERE a.draft_id = drafts.id))
             ORDER BY updated_at LIMIT 20",
        )
        .bind(now - SERVER_QUIET)
        .bind(now - SERVER_RETRY)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The draft (from a server copy) with this `Message-ID`, in this account.
    async fn with_message_id(&self, account: i64, message_id: &str) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar("SELECT id FROM drafts WHERE account_id = ? AND message_id = ? LIMIT 1")
            .bind(account)
            .bind(message_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    async fn set_message_id(&self, draft: i64, message_id: &str) -> Result<()> {
        sqlx::query("UPDATE drafts SET message_id = ? WHERE id = ?")
            .bind(message_id)
            .bind(draft)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// `version` is now on the server, in `folder` (as `uid`).
    async fn saved_on_server(
        &self,
        draft: i64,
        version: i64,
        account: i64,
        folder: &str,
        uid: Option<u32>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE drafts SET server_version = ?, server_account = ?, server_folder = ?, server_uid = ?,
                               server_saved_at = ?, server_tried_at = ?, server_error = NULL
             WHERE id = ?",
        )
        .bind(version)
        .bind(account)
        .bind(folder)
        .bind(uid.map(i64::from))
        .bind(now())
        .bind(now())
        .bind(draft)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn server_failed(&self, draft: i64, error: &str) -> Result<()> {
        sqlx::query("UPDATE drafts SET server_error = ?, server_tried_at = ? WHERE id = ?")
            .bind(error)
            .bind(now())
            .bind(draft)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn attachment_data(&self, draft: i64) -> Result<Vec<AttachmentData>> {
        let rows: Vec<(String, String, Vec<u8>)> = sqlx::query_as(
            "SELECT filename, mime, data FROM draft_attachments WHERE draft_id = ? ORDER BY id",
        )
        .bind(draft)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(filename, mime, bytes)| AttachmentData { filename, mime, bytes }).collect())
    }
}

fn draft_from_row(r: &SqliteRow) -> Draft {
    let source = match (r.get::<Option<String>, _>("source_folder"), r.get::<Option<i64>, _>("source_uid")) {
        (Some(folder), Some(uid)) => Some((folder, uid as u32)),
        _ => None,
    };
    let server = ServerCopy {
        message_id: r.get("message_id"),
        version: r.get("version"),
        saved_version: r.get("server_version"),
        account: r.get("server_account"),
        folder: r.get("server_folder"),
        uid: r.get::<Option<i64>, _>("server_uid").map(|uid| uid as u32),
    };
    let account: i64 = r.get("account_id");
    Draft {
        id: r.get("id"),
        account,
        kind: DraftKind::parse(r.get("kind")),
        to: r.get("to_addrs"),
        cc: r.get("cc_addrs"),
        bcc: r.get("bcc_addrs"),
        subject: r.get("subject"),
        body: r.get("body"),
        body_html: r.get("body_html"),
        attachments: Vec::new(),
        updated_at: r.get("updated_at"),
        server_saved_at: r.get("server_saved_at"),
        server_pending: server.saved_version != Some(server.version) || server.account != Some(account),
        server_error: r.get("server_error"),
        signature: r.get("signature"),
        send_at: r.get("send_at"),
        send_state: r.get("send_state"),
        send_error: r.get("send_error"),
        in_reply_to: r.get("in_reply_to"),
        references: r.get("refs"),
        source,
        server,
    }
}

// ---- Signatures ----------------------------------------------------------------------

/// The line before a signature (RFC 3676: "dash dash space"). Mail apps
/// recognise it, fold the signature away, and leave it out of quotes.
const SIGNATURE_LINE: &str = "-- \n";

/// The account's signature, if this kind of message gets it.
fn signature_for(kind: DraftKind, signature: &str, in_replies: bool) -> String {
    match kind {
        DraftKind::New => signature.to_owned(),
        _ if in_replies => signature.to_owned(),
        _ => String::new(),
    }
}

/// The signature block, as it sits in a body.
fn signature_block(signature: &str) -> String {
    format!("{SIGNATURE_LINE}{signature}")
}

/// Where the signature block is in a body, as Havre put it: whole, ending
/// with the text or a line ("Maël B." isn't "Maël").
fn find_signature(body: &str, signature: &str) -> Option<usize> {
    if signature.is_empty() {
        return None;
    }
    let block = signature_block(signature);
    body.match_indices(&block)
        .map(|(at, _)| at)
        .find(|at| matches!(body[at + block.len()..].chars().next(), None | Some('\n')))
}

/// A new body with the signature: after two empty lines (where you'll
/// write), and above the quoted message of a reply or a forward.
fn with_signature(body: &str, signature: &str) -> String {
    if signature.is_empty() {
        return body.to_owned();
    }
    // `body` is empty (a new message) or starts with "\n\nOn … wrote:" (a reply).
    format!("\n\n{}{body}", signature_block(signature))
}

/// "From" changed: the old signature block is replaced by the new one (or
/// removed), if it's still as Havre put it. Returns the body and the
/// signature it now has (empty: none Havre knows of, so nothing to swap
/// next time).
fn swap_signature(body: &str, kind: DraftKind, old: &str, new: &str) -> (String, String) {
    if !old.is_empty() {
        let block = signature_block(old);
        let Some(at) = find_signature(body, old) else {
            // You edited it: it's yours now.
            return (body.to_owned(), String::new());
        };
        let swapped = if new.is_empty() {
            // The block and the empty lines before it go.
            format!("{}{}", body[..at].trim_end_matches('\n'), &body[at + block.len()..])
        } else {
            format!("{}{}{}", &body[..at], signature_block(new), &body[at + block.len()..])
        };
        return (swapped, new.to_owned());
    }
    // No signature before: a new message gets one at the end (a reply's
    // place, above the quote, can't be told reliably once you've written).
    if !new.is_empty() && kind == DraftKind::New && !body.contains(SIGNATURE_LINE) {
        return (format!("{}\n\n{}", body.trim_end(), signature_block(new)), new.to_owned());
    }
    (body.to_owned(), String::new())
}

/// [`swap_signature`] for a formatted draft: the `div data-signature` block
/// is replaced (or removed), if its text is still the one Havre put in.
fn swap_signature_html(
    body: &str,
    kind: DraftKind,
    old: &str,
    new_text: &str,
    new_html: &str,
) -> (String, String) {
    if let Some((start, end)) = html::find_signature(body) {
        let ours = !old.is_empty() && html::to_text(&body[start..end]) == format!("{SIGNATURE_LINE}{old}");
        if !ours {
            // You edited it (or it's not one Havre knows): yours now.
            return (body.to_owned(), String::new());
        }
        let block = if new_text.is_empty() { String::new() } else { html::signature_block(new_html) };
        return (format!("{}{block}{}", &body[..start], &body[end..]), new_text.to_owned());
    }
    // None before: a new message gets one at the end.
    if !new_text.is_empty() && kind == DraftKind::New {
        return (format!("{body}{}", html::signature_block(new_html)), new_text.to_owned());
    }
    (body.to_owned(), String::new())
}

// ---- Replies and forwards ------------------------------------------------------------

/// Fills a reply or a forward from the original message. `own` are your
/// addresses, left out of "reply all".
pub fn prefill(kind: DraftKind, raw: &[u8], own: &[String]) -> Prefill {
    let Some(message) = MessageParser::default().parse(raw) else { return Prefill::default() };
    let subject = message.subject().unwrap_or_default().trim().to_owned();
    let from = message.from().map(people).unwrap_or_default();
    let to = message.to().map(people).unwrap_or_default();
    let cc = message.cc().map(people).unwrap_or_default();
    let date = message.date().map(|d| d.to_rfc822()).unwrap_or_default();
    let text = message.body_text(0).map(|t| t.into_owned()).unwrap_or_default();
    // The original, formatted (or its text as paragraphs), kept to the allowed
    // tags. Its own signature block isn't one Havre could swap.
    let original_html = match message.body_html(0) {
        Some(h) => html::clean(&h),
        _ => html::from_text(text.trim_end()),
    }
    .replace("<div data-signature=\"\">", "<div>");
    let message_id = message.message_id().map(|id| format!("<{id}>"));
    let mut references = ids(message.references());
    if let Some(id) = &message_id {
        references.push(id.clone());
    }

    let is_own = |p: &(Option<String>, String)| own.iter().any(|o| o.eq_ignore_ascii_case(&p.1));
    let sender_name = from.first().map(|p| p.0.clone().unwrap_or_else(|| p.1.clone())).unwrap_or_default();

    match kind {
        DraftKind::Forward => Prefill {
            body_html: Some(format!(
                "<p>---------- Forwarded message ----------<br>From: {}<br>Date: {}<br>Subject: {}<br>To: {}{}</p>{original_html}",
                html::escape(&join(&from)),
                html::escape(&date),
                html::escape(&subject),
                html::escape(&join(&to)),
                if cc.is_empty() { String::new() } else { format!("<br>Cc: {}", html::escape(&join(&cc))) },
            )),
            subject: prefixed("Fwd: ", &subject, &["fwd:", "fw:", "tr:"]),
            body: format!(
                "\n\n---------- Forwarded message ----------\nFrom: {}\nDate: {date}\nSubject: {subject}\nTo: {}\n{}\n{}",
                join(&from),
                join(&to),
                if cc.is_empty() { String::new() } else { format!("Cc: {}\n", join(&cc)) },
                text.trim_end()
            ),
            ..Prefill::default()
        },
        DraftKind::Reply | DraftKind::ReplyAll => {
            // Reply-To wins over From (mailing lists, no-reply senders).
            let reply_to = message.reply_to().map(people).filter(|r| !r.is_empty()).unwrap_or(from.clone());
            // Replying to your own message goes to its recipients instead.
            let (main, rest) = if !reply_to.is_empty() && reply_to.iter().all(is_own) {
                (to.clone(), Vec::new())
            } else {
                (
                    reply_to,
                    if kind == DraftKind::ReplyAll { [to.clone(), cc.clone()].concat() } else { Vec::new() },
                )
            };
            let mut seen: Vec<String> = Vec::new();
            let mut keep = |list: Vec<(Option<String>, String)>| {
                list.into_iter()
                    .filter(|p| !is_own(p) || kind == DraftKind::Reply)
                    .filter(|p| {
                        let key = p.1.to_lowercase();
                        let new = !seen.contains(&key);
                        seen.push(key);
                        new
                    })
                    .collect::<Vec<_>>()
            };
            let main = keep(main);
            let rest = keep(rest);
            let quoted: String = text
                .trim_end()
                .lines()
                .map(|l| if l.starts_with('>') { format!(">{l}\n") } else { format!("> {l}\n") })
                .collect();
            Prefill {
                to: join(&main),
                cc: join(&rest),
                subject: prefixed("Re: ", &subject, &["re:", "aw:", "sv:"]),
                body: if date.is_empty() {
                    format!("\n\n{sender_name} wrote:\n{quoted}")
                } else {
                    format!("\n\nOn {date}, {sender_name} wrote:\n{quoted}")
                },
                body_html: Some(format!(
                    "<p>{}</p><blockquote>{original_html}</blockquote>",
                    html::escape(&if date.is_empty() {
                        format!("{sender_name} wrote:")
                    } else {
                        format!("On {date}, {sender_name} wrote:")
                    })
                )),
                in_reply_to: message_id,
                references: (!references.is_empty()).then(|| references.join(" ")),
                as_is: false,
            }
        }
        DraftKind::New => Prefill::default(),
    }
}

/// "Re: Lunch", without stacking "Re: Re: Re:".
fn prefixed(prefix: &str, subject: &str, already: &[&str]) -> String {
    let lower = subject.to_lowercase();
    if already.iter().any(|p| lower.starts_with(p)) {
        subject.to_owned()
    } else {
        format!("{prefix}{subject}")
    }
}

/// Message ids (`References:`, one or several) as `<id>` strings, ready to send back.
fn ids(list: Option<TextList<'_>>) -> Vec<String> {
    list.map(|list| list.iter().map(|id| format!("<{id}>")).collect()).unwrap_or_default()
}

fn people(address: AddressList<'_>) -> Vec<(Option<String>, String)> {
    address
        .mailboxes()
        .filter_map(|a| {
            let email = a.address()?.to_owned();
            Some((a.name().map(str::to_owned).filter(|n| !n.is_empty() && *n != email), email))
        })
        .collect()
}

/// `Anna Martin <anna@x.fr>, "Dupont, Jean" <jean@y.fr>`
fn join(people: &[(Option<String>, String)]) -> String {
    people
        .iter()
        .map(|(name, email)| match name {
            Some(name) if name.contains([',', ';', '"', '<', '>', '@']) => {
                format!("\"{}\" <{email}>", name.replace('"', ""))
            }
            Some(name) => format!("{name} <{email}>"),
            None => email.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ---- Building the message --------------------------------------------------------------

/// Parses a recipient field. Empty is fine; a wrong address is reported.
fn recipients(field: &str, label: &str) -> Result<Mailboxes> {
    let field = field.trim().trim_end_matches([',', ';']).replace(';', ",");
    if field.is_empty() {
        return Ok(Mailboxes::new());
    }
    Mailboxes::from_str(&field)
        .map_err(|_| Error::Config(format!("Check the addresses in “{label}”: {field}")))
}

/// The addresses typed so far that are valid (a draft may hold `anna@` while
/// you type: the server copy leaves that one out, Havre's keeps it).
fn valid_recipients(field: &str) -> Mailboxes {
    field
        .replace(';', ",")
        .split(',')
        .filter_map(|one| Mailbox::from_str(one.trim()).ok())
        .fold(Mailboxes::new(), |list, mailbox| list.with(mailbox))
}

/// The MIME message for a draft, to send.
pub fn build_message(
    from_name: &str,
    from_email: &str,
    draft: &Draft,
    attachments: Vec<AttachmentData>,
) -> Result<Message> {
    build(from_name, from_email, draft, attachments, None)
}

/// The copy of a draft kept in the Drafts folder: the same message, with the
/// draft's own `Message-ID` (to find it again), the `Bcc` kept (as every mail
/// app does for drafts) and only the recipients typed so far.
pub fn build_draft_message(
    from_name: &str,
    from_email: &str,
    draft: &Draft,
    message_id: &str,
    attachments: Vec<AttachmentData>,
) -> Result<Message> {
    build(from_name, from_email, draft, attachments, Some(message_id))
}

/// `draft_id`: building the Drafts copy (`Some`, with its `Message-ID`) or
/// the message to send (`None`).
fn build(
    from_name: &str,
    from_email: &str,
    draft: &Draft,
    attachments: Vec<AttachmentData>,
    draft_id: Option<&str>,
) -> Result<Message> {
    let address: lettre::Address = from_email
        .parse()
        .map_err(|_| Error::Config(format!("“{from_email}” isn't a valid sender address")))?;
    let from = Mailbox::new(Some(from_name.to_owned()).filter(|n| !n.trim().is_empty()), address.clone());
    let (to, cc, bcc) = match draft_id {
        Some(_) => (valid_recipients(&draft.to), valid_recipients(&draft.cc), valid_recipients(&draft.bcc)),
        None => (recipients(&draft.to, "To")?, recipients(&draft.cc, "Cc")?, recipients(&draft.bcc, "Bcc")?),
    };
    if draft_id.is_none() && to.iter().chain(cc.iter()).chain(bcc.iter()).next().is_none() {
        return Err(Error::Config("Add at least one recipient.".into()));
    }
    let domain = from_email.rsplit_once('@').map_or("havre.local", |(_, d)| d);
    let message_id = match draft_id {
        Some(id) => id.to_owned(),
        None => format!("<{}@{domain}>", random_hex()),
    };
    let mut builder = Message::builder()
        .from(from)
        .subject(draft.subject.trim())
        .date_now()
        .message_id(Some(message_id))
        .user_agent(concat!("Havre/", env!("CARGO_PKG_VERSION")).to_owned());
    if draft_id.is_some() {
        // Never sent: no real envelope (a draft may have no recipient yet).
        builder = builder.keep_bcc().envelope(
            lettre::address::Envelope::new(Some(address.clone()), vec![address])
                .map_err(|e| Error::Config(e.to_string()))?,
        );
    }
    for mailbox in to {
        builder = builder.to(mailbox);
    }
    for mailbox in cc {
        builder = builder.cc(mailbox);
    }
    for mailbox in bcc {
        builder = builder.bcc(mailbox);
    }
    if let Some(id) = &draft.in_reply_to {
        builder = builder.in_reply_to(id.clone());
    }
    if let Some(refs) = &draft.references {
        builder = builder.references(refs.clone());
    }
    let text = draft.body.replace("\r\n", "\n");
    let html = match &draft.body_html {
        // A formatted draft: its own HTML (cleaned again: it may come from anywhere).
        Some(rich) => html::for_sending(&html::clean(rich)),
        None => text_to_html_signed(&text, &draft.signature),
    };
    let alternative = MultiPart::alternative_plain_html(text.clone(), html);
    let message = if attachments.is_empty() {
        builder.multipart(alternative)
    } else {
        let mut mixed = MultiPart::mixed().multipart(alternative);
        for file in attachments {
            let content_type = ContentType::parse(&file.mime)
                .unwrap_or_else(|_| ContentType::parse("application/octet-stream").unwrap());
            mixed = mixed.singlepart(MimeAttachment::new(file.filename).body(file.bytes, content_type));
        }
        builder.multipart(mixed)
    };
    message.map_err(|e| Error::Config(format!("Can't build the message: {e}")))
}

/// The HTML version of a plain-text body: paragraphs, line breaks, links
/// left as text, and `> quoted` lines as blockquotes.
pub fn text_to_html(text: &str) -> String {
    text_to_html_signed(text, "")
}

/// [`text_to_html`], with the signature (when it's still in the text) in
/// grey and a little smaller, as mail apps show them.
pub fn text_to_html_signed(text: &str, signature: &str) -> String {
    let mut html = String::from(
        "<!doctype html><html><body><div style=\"font-family: -apple-system, 'Segoe UI', sans-serif; font-size: 14px; line-height: 1.5\">",
    );
    let block = signature_block(signature);
    match find_signature(text, signature) {
        Some(at) => {
            lines_to_html(&text[..at], &mut html);
            html.push_str("<div style=\"color: #777; font-size: 13px\">");
            lines_to_html(&block, &mut html);
            html.push_str("</div>");
            lines_to_html(&text[at + block.len()..], &mut html);
        }
        None => lines_to_html(text, &mut html),
    }
    html.push_str("</div></body></html>");
    html
}

/// One part of the body, line by line (its blockquotes closed at its end).
fn lines_to_html(text: &str, html: &mut String) {
    let mut depth = 0;
    for line in text.lines() {
        let level = line.chars().take_while(|c| *c == '>').count();
        let content = line[level..].strip_prefix(' ').unwrap_or(&line[level..]);
        while depth < level {
            html.push_str("<blockquote style=\"margin: 0 0 0 4px; padding-left: 10px; border-left: 2px solid #ccc; color: #555\">");
            depth += 1;
        }
        while depth > level {
            html.push_str("</blockquote>");
            depth -= 1;
        }
        html.push_str(&escape(content));
        html.push_str("<br>");
    }
    for _ in 0..depth {
        html.push_str("</blockquote>");
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    let clean: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    if clean.is_empty() { "attachment".into() } else { clean }
}

fn random_hex() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

// ---- Putting it together ----------------------------------------------------------------

/// What happened after sending.
#[derive(Debug, Clone, Serialize)]
pub struct SendOutcome {
    /// A copy is in the Sent folder (by Havre or by the server itself).
    pub saved_to_sent: bool,
    /// Sent, but something after it failed (shown, not an error).
    pub warning: Option<String>,
}

/// Where to start a draft from.
#[derive(Debug, Clone, Deserialize)]
pub struct NewDraft {
    pub account: i64,
    #[serde(default = "new_kind")]
    pub kind: DraftKind,
    /// The message replied to or forwarded.
    pub folder: Option<String>,
    pub uid: Option<u32>,
    /// A `mailto:`-style start: `to`, `subject`.
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub subject: String,
}

fn new_kind() -> DraftKind {
    DraftKind::New
}

/// Servers that file sent mail themselves (an `APPEND` would make a double).
fn saves_sent_itself(smtp_host: &str) -> bool {
    let host = smtp_host.to_lowercase();
    host.ends_with("gmail.com") || host.ends_with("office365.com") || host.ends_with("outlook.com")
}

impl AccountManager {
    /// Answers a meeting invitation by email (iTIP `REPLY`): a short text
    /// for people, and the `.ics` answer for the organizer's calendar.
    pub async fn send_calendar_reply(
        &self,
        account: i64,
        to: &str,
        subject: &str,
        text: &str,
        ics: &str,
    ) -> Result<()> {
        let account = self.database().account(account).await?;
        let smtp = self.database().smtp_config(account.id).await?;
        let from = Mailbox::new(
            Some(account.sender_name.clone()).filter(|n| !n.trim().is_empty()),
            account
                .email
                .parse()
                .map_err(|_| Error::Config(format!("“{}” isn't a valid address", account.email)))?,
        );
        let to: Mailbox = to.parse().map_err(|_| Error::Config(format!("“{to}” isn't a valid address")))?;
        let domain = account.email.rsplit_once('@').map_or("havre.local", |(_, d)| d);
        let calendar = SinglePart::builder()
            .header(
                ContentType::parse("text/calendar; charset=utf-8; method=REPLY")
                    .map_err(|e| Error::Config(e.to_string()))?,
            )
            .body(ics.to_owned());
        let message = Message::builder()
            .from(from)
            .to(to)
            .subject(subject)
            .date_now()
            .message_id(Some(format!("<{}@{domain}>", random_hex())))
            .user_agent(concat!("Havre/", env!("CARGO_PKG_VERSION")).to_owned())
            .multipart(
                MultiPart::alternative().singlepart(SinglePart::plain(text.to_owned())).singlepart(calendar),
            )
            .map_err(|e| Error::Config(format!("Can't build the answer: {e}")))?;
        send(&smtp, &message).await
    }

    pub fn drafts(&self) -> Drafts {
        self.database().drafts()
    }

    /// Starts a draft: empty, or a reply / forward of a message (with the
    /// forwarded message's attachments).
    pub async fn new_draft(&self, start: NewDraft) -> Result<Draft> {
        let account = self.database().account(start.account).await?;
        let drafts = self.drafts();
        let source = match (start.kind, &start.folder, start.uid) {
            (DraftKind::New, _, _) => None,
            (_, Some(folder), Some(uid)) => Some((folder.clone(), uid)),
            _ => return Err(Error::Config("which message? (folder and uid)".into())),
        };
        let Some((folder, uid)) = source else {
            let prefill = Prefill { to: start.to, subject: start.subject, ..Prefill::default() };
            let id = drafts.insert(account.id, DraftKind::New, &prefill, None).await?;
            return drafts.get(id).await;
        };
        let raw = self.engine(account.id).await?.raw(&folder, uid).await?;
        let own = vec![account.email.clone(), account.imap_user.clone()];
        let prefill = prefill(start.kind, &raw, &own);
        let id = drafts.insert(account.id, start.kind, &prefill, Some((&folder, uid))).await?;
        if start.kind == DraftKind::Forward {
            let detail = MessageDetail::parse(
                uid,
                &folder,
                false,
                &raw,
                &RenderOptions { allow_remote_images: false, attachment_url: &|_| String::new() },
            );
            for attachment in detail.attachments.iter().filter(|a| !a.inline) {
                if let Some(data) = mail::extract_attachment(&raw, attachment.index) {
                    drafts.add_attachment(id, &data).await?;
                }
            }
        }
        drafts.get(id).await
    }

    // ---- Drafts on the server ---------------------------------------------------------

    /// Saves a draft's newest version in its account's Drafts folder, if the
    /// copy there is behind. A failure is recorded on the draft (shown in the
    /// compose window, tried again later), not returned: Havre's copy is safe.
    pub async fn save_draft_to_server(&self, id: i64) -> Result<Draft> {
        let _one_at_a_time = self.drafts_lock.lock().await;
        let drafts = self.drafts();
        let draft = drafts.get(id).await?;
        // Waiting to be sent: kept out of the Drafts folder (see `outbox`).
        if !draft.server_pending || draft.is_empty() || draft.send_at.is_some() {
            return Ok(draft);
        }
        match self.write_server_copy(&draft).await {
            Ok((folder, uid)) => {
                drafts.saved_on_server(id, draft.server.version, draft.account, &folder, uid).await?;
            }
            Err(err) => {
                tracing::warn!(draft = id, "couldn't save the draft on the server: {err}");
                drafts.server_failed(id, &server_error(&err)).await?;
            }
        }
        drafts.get(id).await
    }

    /// Writes the copy; returns where it went.
    async fn write_server_copy(&self, draft: &Draft) -> Result<(String, Option<u32>)> {
        let account = self.database().account(draft.account).await?;
        let engine = self.engine(account.id).await?;
        let folder = engine
            .store()
            .folders()
            .await?
            .into_iter()
            .find(|f| f.folder.role == FolderRole::Drafts)
            .map(|f| f.folder.path)
            .ok_or_else(|| Error::Config(NO_DRAFTS_FOLDER.into()))?;
        let drafts = self.drafts();
        let message_id = match &draft.server.message_id {
            Some(id) => id.clone(),
            None => {
                let domain = account.email.rsplit_once('@').map_or("havre.local", |(_, d)| d);
                let id = format!("<draft-{}@{domain}>", random_hex());
                drafts.set_message_id(draft.id, &id).await?;
                id
            }
        };
        let attachments = drafts.attachment_data(draft.id).await?;
        let raw = build_draft_message(&account.sender_name, &account.email, draft, &message_id, attachments)?
            .formatted();
        let imap = self.database().imap_config(account.id).await?;
        // The UID Havre had for its copy, when it's in this same folder.
        let same_place =
            draft.server.account == Some(account.id) && draft.server.folder.as_deref() == Some(&folder);
        let old_uid = if same_place { draft.server.uid } else { None };
        let uid = mail::save_draft(&imap, &folder, &raw, &message_id, old_uid).await?;
        // Moved to another account (or its Drafts folder changed): the old copy goes.
        if let (Some(old_account), Some(old_folder), false) =
            (draft.server.account, &draft.server.folder, same_place)
        {
            self.remove_copy(old_account, old_folder, &message_id, draft.server.uid).await;
        }
        engine.request_sync(Some(folder.clone()));
        Ok((folder, uid))
    }

    /// Removes a draft's copy from a server (best effort: logged if it fails).
    async fn remove_copy(&self, account: i64, folder: &str, message_id: &str, uid: Option<u32>) -> bool {
        let removed = match self.database().imap_config(account).await {
            Ok(imap) => mail::remove_draft(&imap, folder, message_id, uid).await,
            Err(err) => Err(err),
        };
        match removed {
            Ok(()) => {
                if let Ok(engine) = self.engine(account).await {
                    engine.request_sync(Some(folder.to_owned()));
                }
                true
            }
            Err(err) => {
                tracing::warn!(account, "couldn't remove a draft from the server: {err}");
                false
            }
        }
    }

    /// The draft's copy on the server, if it has one.
    async fn remove_server_copy(&self, draft: &Draft) -> bool {
        match (draft.server.account, &draft.server.folder, &draft.server.message_id) {
            (Some(account), Some(folder), Some(message_id)) => {
                self.remove_copy(account, folder, message_id, draft.server.uid).await
            }
            _ => true,
        }
    }

    /// Discards a draft: in Havre and on the server.
    pub async fn discard_draft(&self, id: i64) -> Result<()> {
        let _one_at_a_time = self.drafts_lock.lock().await;
        let draft = self.drafts().get(id).await?;
        self.remove_server_copy(&draft).await;
        self.drafts().delete(id).await
    }

    /// A draft from the server's Drafts folder (started in another app),
    /// opened to finish it in Havre: recipients, subject, text and
    /// attachments, linked to that copy so saving replaces it. Opening the
    /// same one again gives the same Havre draft.
    pub async fn draft_from_message(&self, account: i64, folder: &str, uid: u32) -> Result<Draft> {
        let _one_at_a_time = self.drafts_lock.lock().await;
        let drafts = self.drafts();
        let raw = self.engine(account).await?.raw(folder, uid).await?;
        let message = MessageParser::default()
            .parse(&raw)
            .ok_or_else(|| Error::Config("This draft can't be read.".into()))?;
        let message_id = message.message_id().map(|id| format!("<{id}>"));
        if let Some(id) = &message_id
            && let Some(existing) = drafts.with_message_id(account, id).await?
        {
            return drafts.get(existing).await;
        }
        // Mail is untrusted: only its text is taken (HTML turned into text by
        // the parser), addresses re-joined, attachments within the limit.
        let addresses = |a: Option<AddressList<'_>>| a.map(people).map(|p| join(&p)).unwrap_or_default();
        let references = ids(message.references());
        let prefill = Prefill {
            to: addresses(message.to()),
            cc: addresses(message.cc()),
            subject: message.subject().unwrap_or_default().trim().to_owned(),
            body: message.body_text(0).map(|t| t.trim_end().to_owned()).unwrap_or_default(),
            // Its formatting, kept to what Havre allows.
            body_html: message.body_html(0).map(|h| html::clean(&h)),
            as_is: true,
            in_reply_to: message.in_reply_to().and_then(|list| list.first()).map(|id| format!("<{id}>")),
            references: (!references.is_empty()).then(|| references.join(" ")),
        };
        let bcc = addresses(message.bcc());
        let id = drafts.insert(account, DraftKind::New, &prefill, None).await?;
        if !bcc.is_empty() {
            drafts.update(id, &DraftUpdate { bcc: Some(bcc), ..DraftUpdate::default() }).await?;
        }
        let detail = MessageDetail::parse(
            uid,
            folder,
            false,
            &raw,
            &RenderOptions { allow_remote_images: false, attachment_url: &|_| String::new() },
        );
        for attachment in detail.attachments.iter().filter(|a| !a.inline) {
            if let Some(data) = mail::extract_attachment(&raw, attachment.index)
                && let Err(err) = drafts.add_attachment(id, &data).await
            {
                tracing::warn!("a draft's attachment was left out: {err}");
            }
        }
        // Linked to that copy: it's on the server as it is now.
        let message_id = match message_id {
            Some(id) => id,
            None => format!("<draft-{}@havre.local>", random_hex()),
        };
        drafts.set_message_id(id, &message_id).await?;
        let version = drafts.get(id).await?.server.version;
        drafts.saved_on_server(id, version, account, folder, Some(uid)).await?;
        drafts.get(id).await
    }

    /// Sends a draft, files a copy in Sent, marks the original as answered,
    /// and deletes the draft (and its copy in the Drafts folder).
    pub async fn send_draft(&self, id: i64) -> Result<SendOutcome> {
        let drafts = self.drafts();
        let draft = drafts.get(id).await?;
        let account = self.database().account(draft.account).await?;
        let smtp = self.database().smtp_config(account.id).await?;
        let message =
            build_message(&account.sender_name, &account.email, &draft, drafts.attachment_data(id).await?)?;
        send(&smtp, &message).await?;
        tracing::info!(account = account.email, "message sent");

        let mut outcome = SendOutcome { saved_to_sent: true, warning: None };
        let engine = self.engine(account.id).await.ok();
        let sent_folder = match &engine {
            Some(engine) => engine
                .store()
                .folders()
                .await
                .ok()
                .and_then(|f| f.into_iter().find(|f| f.folder.role == FolderRole::Sent))
                .map(|f| f.folder.path),
            None => None,
        };
        if !saves_sent_itself(&smtp.host) {
            match &sent_folder {
                Some(folder) => {
                    let imap = self.database().imap_config(account.id).await?;
                    if let Err(err) = mail::append_message(&imap, folder, &message.formatted()).await {
                        tracing::warn!("couldn't save to Sent: {err}");
                        outcome = SendOutcome {
                            saved_to_sent: false,
                            warning: Some(format!("Sent, but no copy could be saved in “{folder}”: {err}")),
                        };
                    }
                }
                None => {
                    outcome = SendOutcome {
                        saved_to_sent: false,
                        warning: Some("Sent. This account has no Sent folder, so no copy was saved.".into()),
                    }
                }
            }
        }
        if let (Some((folder, uid)), DraftKind::Reply | DraftKind::ReplyAll) = (&draft.source, draft.kind) {
            let imap = self.database().imap_config(account.id).await?;
            if let Err(err) = mail::set_answered(&imap, folder, *uid).await {
                tracing::warn!("couldn't mark as answered: {err}");
            }
        }
        {
            let _one_at_a_time = self.drafts_lock.lock().await;
            let draft = drafts.get(id).await?;
            if !self.remove_server_copy(&draft).await && outcome.warning.is_none() {
                outcome.warning = Some("Sent. Its draft is still in the Drafts folder on the server.".into());
            }
            drafts.delete(id).await?;
        }
        if let Some(engine) = engine {
            engine.request_sync(sent_folder);
        }
        Ok(outcome)
    }
}

const NO_DRAFTS_FOLDER: &str = "This account has no Drafts folder on the server: the draft stays in Havre.";

/// A server failure, as the compose window shows it.
fn server_error(err: &Error) -> String {
    match err {
        Error::Config(message) if message == NO_DRAFTS_FOLDER => message.clone(),
        Error::Config(message) | Error::SignIn(message) => {
            format!("Not saved on the server: {message} Havre tries again in a few minutes.")
        }
        // The details are in the log; here, what it means for you.
        _ => "The mail server didn't answer: saved in Havre, tried again in a few minutes.".into(),
    }
}

/// Every 10 s: saves the drafts whose server copy is behind (see
/// `Drafts::due_on_server`). A `Weak` pointer, like the snooze loop: it
/// mustn't keep the manager alive.
pub(crate) async fn drafts_loop(manager: std::sync::Weak<AccountManager>) {
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let Some(manager) = manager.upgrade() else { return };
        let due = match manager.drafts().due_on_server(now()).await {
            Ok(due) => due,
            Err(err) => {
                tracing::warn!(%err, "reading drafts to save");
                continue;
            }
        };
        for id in due {
            if let Err(err) = manager.save_draft_to_server(id).await {
                tracing::warn!(draft = id, "{err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn two_accounts() -> (crate::store::Database, i64, i64) {
        let db = crate::store::Database::in_memory().await.unwrap();
        let mut ids = Vec::new();
        for (email, signature) in [("me@work.org", "Maël Belliard\nAcme · +33 1 23"), ("me@home.org", "Maël")]
        {
            let account = db
                .insert_account(&crate::store::NewAccount {
                    name: email.into(),
                    email: email.into(),
                    imap_host: "mail.example.org".into(),
                    imap_port: 993,
                    imap_user: email.into(),
                    color: "work".into(),
                    password: "x".into(),
                    sender_name: String::new(),
                    smtp_host: None,
                    smtp_port: None,
                    smtp_security: None,
                })
                .await
                .unwrap();
            let update =
                crate::store::AccountUpdate { signature: Some(signature.into()), ..Default::default() };
            db.update_account(account.id, &update).await.unwrap();
            ids.push(account.id);
        }
        (db, ids[0], ids[1])
    }

    #[tokio::test]
    async fn signatures_in_new_messages_replies_and_when_from_changes() {
        let (db, work, home) = two_accounts().await;
        let drafts = db.drafts();
        // Plain-text messages here (rich ones: the next test).
        let plain = outbox::MailSettings { undo_seconds: 10, compose_format: "plain".into() };
        drafts.set_mail_settings(&plain).await.unwrap();
        // A new message: two empty lines to write in, then the signature.
        let id = drafts.insert(work, DraftKind::New, &Prefill::default(), None).await.unwrap();
        let draft = drafts.get(id).await.unwrap();
        assert_eq!(draft.body, "\n\n-- \nMaël Belliard\nAcme · +33 1 23");
        // You write, then switch to the other account: its signature replaces this one.
        let body = format!("Hello Anna,{}", draft.body);
        drafts.update(id, &DraftUpdate { body: Some(body), ..Default::default() }).await.unwrap();
        let moved =
            drafts.update(id, &DraftUpdate { account: Some(home), ..Default::default() }).await.unwrap();
        assert_eq!(moved.body, "Hello Anna,\n\n-- \nMaël");
        assert_eq!(moved.signature, "Maël");

        // A reply: above the quote.
        let reply = Prefill { body: "\n\nOn Mon, Anna wrote:\n> Hi\n".into(), ..Prefill::default() };
        let id = drafts.insert(work, DraftKind::Reply, &reply, None).await.unwrap();
        let body = drafts.get(id).await.unwrap().body;
        assert!(body.starts_with("\n\n-- \nMaël Belliard\nAcme · +33 1 23\n\nOn Mon, Anna wrote:"), "{body}");
        // Not in replies when you said so.
        let off = crate::store::AccountUpdate { signature_in_replies: Some(false), ..Default::default() };
        db.update_account(work, &off).await.unwrap();
        let id = drafts.insert(work, DraftKind::Reply, &reply, None).await.unwrap();
        assert_eq!(drafts.get(id).await.unwrap().body, reply.body);
    }

    #[tokio::test]
    async fn rich_drafts_signatures_and_replies() {
        let (db, work, home) = two_accounts().await;
        let drafts = db.drafts();
        // A formatted signature for work.
        let sig = crate::store::AccountUpdate {
            signature_html: Some(
                "<p>Maël <strong>Belliard</strong><br><a href=\"https://havre.app\">havre.app</a></p>".into(),
            ),
            ..Default::default()
        };
        let account = db.update_account(work, &sig).await.unwrap();
        assert_eq!(
            account.signature, "Maël Belliard\nhavre.app <https://havre.app>",
            "its plain text, made from it"
        );

        // A new message: an empty paragraph, then the signature block.
        let id = drafts.insert(work, DraftKind::New, &Prefill::default(), None).await.unwrap();
        let draft = drafts.get(id).await.unwrap();
        let html = draft.body_html.clone().unwrap();
        assert!(html.starts_with("<p></p><div data-signature=\"\"><p>-- </p><p>Maël <strong>"), "{html}");
        assert!(draft.is_empty(), "only the signature: nothing written");

        // You write (the page sends HTML, maybe with things it shouldn't).
        let written = format!("<p>Hello <em>Anna</em><script>x()</script></p>{}", &html[7..]);
        let draft =
            drafts.update(id, &DraftUpdate { body_html: Some(written), ..Default::default() }).await.unwrap();
        assert!(!draft.body_html.as_deref().unwrap().contains("script"));
        assert!(draft.body.starts_with("Hello Anna\n\n-- \nMaël Belliard"), "{}", draft.body);
        // From home: the signature block follows.
        let draft =
            drafts.update(id, &DraftUpdate { account: Some(home), ..Default::default() }).await.unwrap();
        assert_eq!(
            draft.body_html.as_deref().unwrap(),
            "<p>Hello <em>Anna</em></p><div data-signature=\"\"><p>-- </p><p>Maël</p></div>"
        );
        assert_eq!(draft.signature, "Maël");
        // Edited by you: not swapped back.
        let edited = "<p>Hi</p><div data-signature=\"\"><p>-- </p><p>Maël B.</p></div>";
        drafts
            .update(id, &DraftUpdate { body_html: Some(edited.into()), ..Default::default() })
            .await
            .unwrap();
        let back =
            drafts.update(id, &DraftUpdate { account: Some(work), ..Default::default() }).await.unwrap();
        assert_eq!(back.body_html.as_deref(), Some(edited));

        // Sending: the HTML part is the draft's, styled; the text part made from it.
        assert!(build_message("", "me@work.org", &back, vec![]).is_err(), "no recipient yet");
        let to = DraftUpdate { to: Some("anna@example.com".into()), ..Default::default() };
        let ready = drafts.update(id, &to).await.unwrap();
        let raw =
            String::from_utf8(build_message("", "me@work.org", &ready, vec![]).unwrap().formatted()).unwrap();
        assert!(raw.contains("text/plain") && raw.contains("text/html"), "{raw}");
        assert!(raw.contains("color: #777"), "the signature in grey: {raw}");

        // A reply quotes the original, formatted, in a quote.
        let reply = prefill(DraftKind::Reply, ORIGINAL_HTML, &[]);
        let quoted = reply.body_html.clone().unwrap();
        assert!(
            quoted.contains("wrote:</p><blockquote>") && quoted.contains("<strong>Friday</strong>"),
            "{quoted}"
        );
        assert!(!quoted.contains("data-signature") && !quoted.contains("script"), "{quoted}");
        let id = drafts.insert(work, DraftKind::Reply, &reply, None).await.unwrap();
        let html = drafts.get(id).await.unwrap().body_html.unwrap();
        let (sig, quote) = (html.find("data-signature").unwrap(), html.find("<blockquote>").unwrap());
        assert!(sig < quote, "the signature above the quote");

        // Plain ↔ rich on a draft.
        let plain =
            drafts.update(id, &DraftUpdate { rich: Some(false), ..Default::default() }).await.unwrap();
        assert!(plain.body_html.is_none() && plain.body.contains("> "));
        let again = drafts.update(id, &DraftUpdate { rich: Some(true), ..Default::default() }).await.unwrap();
        assert!(again.body_html.unwrap().contains("<blockquote>"));
    }

    const ORIGINAL_HTML: &[u8] = b"From: Anna Martin <anna@example.com>\r\n\
To: me@work.org\r\nSubject: Lunch\r\nDate: Mon, 2 Nov 2026 10:00:00 +0100\r\nMessage-ID: <x@example.com>\r\n\
MIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<html><body><p style=\"color:red\">Lunch on <strong>Friday</strong>?</p>\
<div data-signature=\"\"><p>-- </p><p>Anna</p></div><script>x()</script></body></html>\r\n";

    #[test]
    fn swapping_html_signatures_from_the_editor() {
        // The editor drops the space after "--" (HTML collapses it): still ours.
        let body = "<p>Hi</p><div data-signature=\"\"><p>--</p><p>Maël</p></div><p></p>";
        let (swapped, sig) = swap_signature_html(body, DraftKind::New, "Maël", "Other", "<p>Other</p>");
        assert_eq!(swapped, "<p>Hi</p><div data-signature=\"\"><p>-- </p><p>Other</p></div><p></p>");
        assert_eq!(sig, "Other");
        // Edited inside the block: yours, left alone.
        let edited = body.replace("Maël", "Maël B.");
        assert_eq!(swap_signature_html(&edited, DraftKind::New, "Maël", "Other", "<p>Other</p>").0, edited);
    }

    #[test]
    fn swapping_and_showing_signatures() {
        // Edited by you: left alone (and no longer Havre's to swap).
        let edited = "Hi\n\n-- \nMaël B.";
        assert_eq!(
            swap_signature(edited, DraftKind::New, "Maël", "Other"),
            (edited.to_owned(), String::new())
        );
        // To an account without one: the block goes, the quote stays.
        let reply = "Thanks!\n\n-- \nMaël\n\nOn Mon, Anna wrote:\n> Hi";
        assert_eq!(
            swap_signature(reply, DraftKind::Reply, "Maël", "").0,
            "Thanks!\n\nOn Mon, Anna wrote:\n> Hi"
        );
        // From none to one: a new message gets it at the end, a reply doesn't.
        assert_eq!(swap_signature("Hi", DraftKind::New, "", "Maël").0, "Hi\n\n-- \nMaël");
        assert_eq!(swap_signature("Hi\n> q", DraftKind::Reply, "", "Maël").0, "Hi\n> q");

        // The HTML version: the signature in grey, the quote still a blockquote.
        let html = text_to_html_signed(reply, "Maël");
        assert!(html.contains("<div style=\"color: #777; font-size: 13px\">-- <br>Maël<br></div>"), "{html}");
        assert!(html.contains("<blockquote"));
        assert!(!text_to_html(reply).contains("#777"));
        // A signature can't hold its own "-- " line.
        assert_eq!(crate::store::clean_signature_for_tests("A\n-- \nB  ").unwrap(), "A\nB");
    }

    const ORIGINAL: &[u8] = b"From: Anna Martin <anna@example.com>\r\n\
To: mael@example.org, Bob <bob@example.com>\r\n\
Cc: carol@example.com\r\n\
Subject: Lunch on Friday\r\n\
Date: Tue, 13 Oct 2026 10:00:00 +0200\r\n\
Message-ID: <abc@example.com>\r\n\
References: <root@example.com>\r\n\
\r\n\
Are you free?\r\n> earlier\r\n";

    #[test]
    fn reply_quotes_and_threads() {
        let p = prefill(DraftKind::Reply, ORIGINAL, &["mael@example.org".into()]);
        assert_eq!(p.to, "Anna Martin <anna@example.com>");
        assert_eq!(p.cc, "");
        assert_eq!(p.subject, "Re: Lunch on Friday");
        assert!(p.body.contains("Anna Martin wrote:\n> Are you free?\n>> earlier"));
        assert_eq!(p.in_reply_to.as_deref(), Some("<abc@example.com>"));
        assert_eq!(p.references.as_deref(), Some("<root@example.com> <abc@example.com>"));
    }

    #[test]
    fn reply_all_leaves_me_out() {
        let p = prefill(DraftKind::ReplyAll, ORIGINAL, &["MAEL@example.org".into()]);
        assert_eq!(p.to, "Anna Martin <anna@example.com>");
        assert_eq!(p.cc, "Bob <bob@example.com>, carol@example.com");
    }

    #[test]
    fn forward_keeps_the_original() {
        let p = prefill(DraftKind::Forward, ORIGINAL, &[]);
        assert_eq!(p.to, "");
        assert_eq!(p.subject, "Fwd: Lunch on Friday");
        assert!(p.body.contains("Forwarded message"));
        assert!(p.body.contains("From: Anna Martin <anna@example.com>"));
        assert_eq!(prefixed("Re: ", "RE: hi", &["re:"]), "RE: hi");
    }

    #[test]
    fn builds_a_mime_message() {
        let draft = Draft {
            id: 1,
            account: 1,
            kind: DraftKind::Reply,
            to: "Anna <anna@example.com>; bob@example.com".into(),
            cc: String::new(),
            bcc: "secret@example.com".into(),
            subject: "Re: Lunch".into(),
            body: "Yes!\n\n> Are you free?".into(),
            body_html: None,
            attachments: vec![],
            updated_at: 0,
            server_saved_at: None,
            server_pending: true,
            server_error: None,
            signature: String::new(),
            send_at: None,
            send_state: None,
            send_error: None,
            in_reply_to: Some("<abc@example.com>".into()),
            references: Some("<abc@example.com>".into()),
            source: None,
            server: ServerCopy::default(),
        };
        let file = AttachmentData {
            filename: "menu.txt".into(),
            mime: "text/plain".into(),
            bytes: b"soup".to_vec(),
        };
        let message = build_message("Maël", "mael@example.org", &draft, vec![file]).unwrap();
        let raw = String::from_utf8(message.formatted()).unwrap();
        assert!(raw.contains("To: Anna <anna@example.com>, bob@example.com"));
        assert!(!raw.contains("secret@example.com"), "Bcc must not be in the headers");
        assert!(raw.contains("In-Reply-To: <abc@example.com>"));
        assert!(raw.contains("multipart/mixed"));
        assert!(raw.contains("menu.txt"));
        assert_eq!(message.envelope().to().len(), 3);

        let mut empty = draft.clone();
        empty.to.clear();
        empty.bcc.clear();
        assert!(build_message("", "mael@example.org", &empty, vec![]).is_err());
        let mut wrong = draft.clone();
        wrong.to = "not an address".into();
        assert!(build_message("", "mael@example.org", &wrong, vec![]).is_err());

        // The Drafts copy: its own Message-ID, the Bcc kept, whatever is
        // typed so far (an unfinished address is left out, not an error).
        let mut unfinished = draft.clone();
        unfinished.to = "Anna <anna@example.com>, bob@".into();
        let copy =
            build_draft_message("Maël", "mael@example.org", &unfinished, "<draft-1@example.org>", vec![])
                .unwrap();
        let raw = String::from_utf8(copy.formatted()).unwrap();
        assert!(raw.contains("Message-ID: <draft-1@example.org>"), "{raw}");
        assert!(raw.contains("Bcc: secret@example.com"));
        assert!(raw.contains("To: Anna <anna@example.com>\r\n"));
        assert!(build_draft_message("", "mael@example.org", &empty, "<draft-2@example.org>", vec![]).is_ok());
    }

    #[test]
    fn html_version_quotes_lines() {
        let html = text_to_html("Hi <you>\n> quoted\n>> deeper\nback");
        assert!(html.contains("Hi &lt;you&gt;<br>"));
        assert_eq!(html.matches("<blockquote").count(), 2);
        assert!(html.contains("deeper<br></blockquote></blockquote>back"));
    }

    #[test]
    fn guesses_the_outgoing_server() {
        assert_eq!(smtp_defaults("imap.gmail.com"), ("smtp.gmail.com".into(), 465, Security::Tls));
        assert_eq!(smtp_defaults("outlook.office365.com").2, Security::StartTls);
        assert_eq!(smtp_defaults("imap.free.fr").0, "smtp.free.fr");
        assert_eq!(smtp_defaults("ssl0.ovh.net").0, "ssl0.ovh.net");
    }
}
