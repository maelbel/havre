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
use mail_parser::{Address, MessageParser};
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
    /// Plain text. Replies start with the quoted original (`> …`).
    pub body: String,
    pub attachments: Vec<DraftAttachment>,
    pub updated_at: i64,
    #[serde(skip)]
    in_reply_to: Option<String>,
    #[serde(skip)]
    references: Option<String>,
    #[serde(skip)]
    source: Option<(String, u32)>,
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
    pub body: Option<String>,
}

/// The largest attachment total accepted (most servers refuse ~25 MB).
pub const MAX_ATTACHMENTS: usize = 25 * 1024 * 1024;

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
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
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
        Ok(sqlx::query_scalar(
            "INSERT INTO drafts (account_id, kind, to_addrs, cc_addrs, subject, body, in_reply_to, refs,
                                 source_folder, source_uid, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(account)
        .bind(kind.as_str())
        .bind(&prefill.to)
        .bind(&prefill.cc)
        .bind(&prefill.subject)
        .bind(&prefill.body)
        .bind(&prefill.in_reply_to)
        .bind(&prefill.references)
        .bind(source.map(|s| s.0))
        .bind(source.map(|s| s.1 as i64))
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?)
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
        let mut sql = sqlx::QueryBuilder::<sqlx::Sqlite>::new("UPDATE drafts SET updated_at = ");
        sql.push_bind(now());
        let fields = [
            ("to_addrs", &update.to),
            ("cc_addrs", &update.cc),
            ("bcc_addrs", &update.bcc),
            ("subject", &update.subject),
            ("body", &update.body),
        ];
        for (column, value) in fields {
            if let Some(value) = value {
                sql.push(format!(", {column} = ")).push_bind(value.clone());
            }
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
        sqlx::query("UPDATE drafts SET updated_at = ? WHERE id = ?")
            .bind(now())
            .bind(draft)
            .execute(&self.pool)
            .await?;
        Ok(DraftAttachment { id, filename, mime: file.mime.clone(), size: file.bytes.len() as i64 })
    }

    pub async fn remove_attachment(&self, draft: i64, attachment: i64) -> Result<()> {
        sqlx::query("DELETE FROM draft_attachments WHERE id = ? AND draft_id = ?")
            .bind(attachment)
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
    Draft {
        id: r.get("id"),
        account: r.get("account_id"),
        kind: DraftKind::parse(r.get("kind")),
        to: r.get("to_addrs"),
        cc: r.get("cc_addrs"),
        bcc: r.get("bcc_addrs"),
        subject: r.get("subject"),
        body: r.get("body"),
        attachments: Vec::new(),
        updated_at: r.get("updated_at"),
        in_reply_to: r.get("in_reply_to"),
        references: r.get("refs"),
        source,
    }
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
    let message_id = message.message_id().map(|id| format!("<{id}>"));
    let mut references: Vec<String> = message
        .references()
        .as_text_list()
        .map(|list| list.iter().map(|r| format!("<{r}>")).collect())
        .or_else(|| message.references().as_text().map(|r| vec![format!("<{r}>")]))
        .unwrap_or_default();
    if let Some(id) = &message_id {
        references.push(id.clone());
    }

    let is_own = |p: &(Option<String>, String)| own.iter().any(|o| o.eq_ignore_ascii_case(&p.1));
    let sender_name = from.first().map(|p| p.0.clone().unwrap_or_else(|| p.1.clone())).unwrap_or_default();

    match kind {
        DraftKind::Forward => Prefill {
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
                in_reply_to: message_id,
                references: (!references.is_empty()).then(|| references.join(" ")),
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

fn people(address: &Address<'_>) -> Vec<(Option<String>, String)> {
    address
        .iter()
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

/// The MIME message for a draft.
pub fn build_message(
    from_name: &str,
    from_email: &str,
    draft: &Draft,
    attachments: Vec<AttachmentData>,
) -> Result<Message> {
    let from = Mailbox::new(
        Some(from_name.to_owned()).filter(|n| !n.trim().is_empty()),
        from_email
            .parse()
            .map_err(|_| Error::Config(format!("“{from_email}” isn't a valid sender address")))?,
    );
    let (to, cc, bcc) =
        (recipients(&draft.to, "To")?, recipients(&draft.cc, "Cc")?, recipients(&draft.bcc, "Bcc")?);
    if to.iter().chain(cc.iter()).chain(bcc.iter()).next().is_none() {
        return Err(Error::Config("Add at least one recipient.".into()));
    }
    let domain = from_email.rsplit_once('@').map_or("havre.local", |(_, d)| d);
    let mut builder = Message::builder()
        .from(from)
        .subject(draft.subject.trim())
        .date_now()
        .message_id(Some(format!("<{}@{domain}>", random_hex())))
        .user_agent(concat!("Havre/", env!("CARGO_PKG_VERSION")).to_owned());
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
    let alternative = MultiPart::alternative_plain_html(text.clone(), text_to_html(&text));
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
    let mut html = String::from(
        "<!doctype html><html><body><div style=\"font-family: -apple-system, 'Segoe UI', sans-serif; font-size: 14px; line-height: 1.5\">",
    );
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
    html.push_str("</div></body></html>");
    html
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

    /// Sends a draft, files a copy in Sent, marks the original as answered,
    /// and deletes the draft.
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
        drafts.delete(id).await?;
        if let Some(engine) = engine {
            engine.request_sync(sent_folder);
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            attachments: vec![],
            updated_at: 0,
            in_reply_to: Some("<abc@example.com>".into()),
            references: Some("<abc@example.com>".into()),
            source: None,
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
        let mut wrong = draft;
        wrong.to = "not an address".into();
        assert!(build_message("", "mael@example.org", &wrong, vec![]).is_err());
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
