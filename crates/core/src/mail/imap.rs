//! Talking to the mail server over IMAP.
//!
//! Every function follows the same steps:
//! 1. open a TCP connection, 2. wrap it in TLS, 3. read the server greeting,
//! 4. log in, 5. select a folder, 6. do the work, 7. log out.
//!
//! Opening a connection per request is simple but slow-ish (TLS + login).
//! Step 2 of the roadmap replaces this with a local cache kept in sync by a
//! background task (IMAP IDLE).
//!
//! We always address messages by **UID** (`UID FETCH`, `UID STORE`): unlike
//! sequence numbers, UIDs don't change when other messages are deleted.

use std::sync::Arc;

use async_imap::types::{Fetch, Flag};
use futures::TryStreamExt;
use serde::Serialize;
use tokio::net::TcpStream;
use tokio_rustls::{TlsConnector, client::TlsStream, rustls};

use super::{
    Folder, FolderRole, MessageDetail, MessageSummary, RenderOptions,
    folder::sort_folders,
    message::{AttachmentData, extract_attachment},
    query::{ListQuery, MessagePage, search_criteria, uid_set},
};
use crate::{
    Error, Result,
    config::{ImapAuth, ImapConfig},
    oauth::XOAuth2,
};

/// An authenticated IMAP session over TLS.
pub(crate) type Session = async_imap::Session<TlsStream<TcpStream>>;

/// All folders that can be opened, special ones first.
pub async fn list_folders(config: &ImapConfig) -> Result<Vec<Folder>> {
    let mut session = connect(config).await?;
    let mut folders = folders_in_session(&mut session).await?;
    sort_folders(&mut folders);
    session.logout().await?;
    Ok(folders)
}

/// One page of messages of `folder`, newest first.
///
/// Uses `RFC822.HEADER`, which does not mark messages as read.
pub async fn fetch_messages(config: &ImapConfig, folder: &str, query: &ListQuery) -> Result<MessagePage> {
    let mut session = connect(config).await?;
    // EXAMINE = open read-only: listing must never change anything.
    let mailbox = session.examine(folder).await?;
    let limit = query.limit.max(1);

    let page = match search_criteria(query.before_uid, query.search.as_deref()) {
        // Plain listing: the newest messages are the last sequence numbers,
        // from 1 (oldest) to `exists` (newest). No search needed.
        None => {
            let total = mailbox.exists;
            if total == 0 {
                MessagePage { messages: Vec::new(), has_more: false }
            } else {
                let first = total.saturating_sub(limit - 1).max(1);
                let fetches: Vec<Fetch> = session
                    .fetch(format!("{first}:{total}"), "(UID FLAGS RFC822.HEADER)")
                    .await?
                    .try_collect()
                    .await?;
                MessagePage { messages: summaries(&fetches), has_more: first > 1 }
            }
        }
        // Search or "load more": ask the server which UIDs match, keep the
        // newest `limit` of them, then fetch only those.
        Some(criteria) => {
            let mut uids: Vec<u32> = session.uid_search(&criteria).await?.into_iter().collect();
            uids.sort_unstable_by(|a, b| b.cmp(a));
            let has_more = uids.len() > limit as usize;
            uids.truncate(limit as usize);
            let messages = if uids.is_empty() {
                Vec::new()
            } else {
                let fetches: Vec<Fetch> = session
                    .uid_fetch(uid_set(&uids), "(UID FLAGS RFC822.HEADER)")
                    .await?
                    .try_collect()
                    .await?;
                summaries(&fetches)
            };
            MessagePage { messages, has_more }
        }
    };

    session.logout().await?;
    Ok(page)
}

pub(crate) fn summaries(fetches: &[Fetch]) -> Vec<MessageSummary> {
    let mut messages: Vec<MessageSummary> = fetches
        .iter()
        .filter_map(|f| {
            let mut m = MessageSummary::from_headers(f.uid?, is_unread(f), f.header()?);
            m.flagged = is_flagged(f);
            Some(m)
        })
        .collect();
    // Newest first: a higher UID arrived later.
    messages.sort_by_key(|m| std::cmp::Reverse(m.uid));
    messages
}

/// What happened to messages after an action.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum ActionOutcome {
    /// Moved to the folder `to` (its server path).
    Moved { to: String },
    /// Gone for good (deleted from the Trash).
    Deleted,
}

/// Moves messages to another folder.
pub async fn move_messages(config: &ImapConfig, from: &str, uids: &[u32], to: &str) -> Result<ActionOutcome> {
    if from == to {
        return Err(Error::Config("the messages are already in this folder".into()));
    }
    let mut session = connect(config).await?;
    session.select(from).await?;
    move_in_session(&mut session, uids, to).await?;
    session.logout().await?;
    Ok(ActionOutcome::Moved { to: to.to_owned() })
}

/// Archives messages: moves them to the Archive folder (or Gmail's
/// "All Mail"), creating an `Archive` folder if the account has none.
pub async fn archive_messages(config: &ImapConfig, from: &str, uids: &[u32]) -> Result<ActionOutcome> {
    let mut session = connect(config).await?;
    let folders = folders_in_session(&mut session).await?;
    let target =
        match find_role(&folders, FolderRole::Archive).or_else(|| find_role(&folders, FolderRole::All)) {
            Some(path) => path,
            None => {
                session.create("Archive").await?;
                "Archive".to_owned()
            }
        };
    if target == from {
        session.logout().await?;
        return Err(Error::Config("these messages are already archived".into()));
    }
    session.select(from).await?;
    move_in_session(&mut session, uids, &target).await?;
    session.logout().await?;
    Ok(ActionOutcome::Moved { to: target })
}

/// Deletes messages: moves them to the Trash; if they already are in the
/// Trash (or the account has no Trash), deletes them permanently.
pub async fn trash_messages(config: &ImapConfig, from: &str, uids: &[u32]) -> Result<ActionOutcome> {
    let mut session = connect(config).await?;
    let folders = folders_in_session(&mut session).await?;
    let trash = find_role(&folders, FolderRole::Trash);
    session.select(from).await?;

    let outcome = match trash {
        Some(trash) if trash != from => {
            move_in_session(&mut session, uids, &trash).await?;
            ActionOutcome::Moved { to: trash }
        }
        _ => {
            expunge_uids(&mut session, uids).await?;
            ActionOutcome::Deleted
        }
    };
    session.logout().await?;
    Ok(outcome)
}

/// `UID MOVE` when the server supports it (RFC 6851). Otherwise the classic
/// way: copy, flag the originals `\Deleted`, then expunge them.
async fn move_in_session(session: &mut Session, uids: &[u32], to: &str) -> Result<()> {
    if uids.is_empty() {
        return Ok(());
    }
    let set = uid_set(uids);
    if session.capabilities().await?.has_str("MOVE") {
        session.uid_mv(&set, to).await?;
    } else {
        session.uid_copy(&set, to).await?;
        expunge_uids(session, uids).await?;
    }
    Ok(())
}

/// Permanently removes messages from the selected folder.
async fn expunge_uids(session: &mut Session, uids: &[u32]) -> Result<()> {
    let set = uid_set(uids);
    let _: Vec<Fetch> = session.uid_store(&set, "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
    if session.capabilities().await?.has_str("UIDPLUS") {
        // Removes only *these* messages.
        let _: Vec<u32> = session.uid_expunge(&set).await?.try_collect().await?;
    } else {
        // Old servers: removes every message flagged \Deleted in the folder.
        let _: Vec<u32> = session.expunge().await?.try_collect().await?;
    }
    Ok(())
}

/// Every folder that can be opened (`LIST "" *` = all levels).
pub(crate) async fn folders_in_session(session: &mut Session) -> Result<Vec<Folder>> {
    let names: Vec<_> = session.list(Some(""), Some("*")).await?.try_collect().await?;
    Ok(names.iter().filter_map(|n| Folder::from_list(n.name(), n.delimiter(), n.attributes())).collect())
}

/// The folder playing `role`, preferring one the server declared itself.
fn find_role(folders: &[Folder], role: FolderRole) -> Option<String> {
    let mut candidates: Vec<&Folder> = folders.iter().filter(|f| f.role == role).collect();
    candidates.sort_by_key(|f| !f.declared_role); // `false` sorts first
    candidates.first().map(|f| f.path.clone())
}

/// One full message. With `mark_read`, it is flagged as read on the server,
/// like opening it in any mail client.
pub async fn fetch_message(
    config: &ImapConfig,
    folder: &str,
    uid: u32,
    mark_read: bool,
    options: &RenderOptions<'_>,
) -> Result<MessageDetail> {
    let mut session = connect(config).await?;
    session.select(folder).await?;

    let (raw, unread) = fetch_raw(&mut session, uid).await?;
    if mark_read && unread {
        store_seen(&mut session, uid, true).await?;
    }
    session.logout().await?;

    let detail = MessageDetail::parse(uid, folder, unread && !mark_read, &raw, options);
    Ok(detail)
}

/// The raw bytes of one message, and whether it was unread. With
/// `mark_read`, it is flagged as read on the server.
pub async fn fetch_raw_message(
    config: &ImapConfig,
    folder: &str,
    uid: u32,
    mark_read: bool,
) -> Result<(Vec<u8>, bool)> {
    let mut session = connect(config).await?;
    if mark_read {
        session.select(folder).await?;
    } else {
        session.examine(folder).await?;
    }
    let (raw, unread) = fetch_raw(&mut session, uid).await?;
    if mark_read && unread {
        store_seen(&mut session, uid, true).await?;
    }
    session.logout().await?;
    Ok((raw, unread))
}

/// Marks a message as read (`seen = true`) or unread.
pub async fn set_seen(config: &ImapConfig, folder: &str, uid: u32, seen: bool) -> Result<()> {
    let mut session = connect(config).await?;
    session.select(folder).await?;
    store_seen(&mut session, uid, seen).await?;
    session.logout().await?;
    Ok(())
}

/// Marks a message as answered (`\Answered`), after a reply was sent.
pub async fn set_answered(config: &ImapConfig, folder: &str, uid: u32) -> Result<()> {
    let mut session = connect(config).await?;
    session.select(folder).await?;
    let _: Vec<Fetch> =
        session.uid_store(uid.to_string(), "+FLAGS.SILENT (\\Answered)").await?.try_collect().await?;
    session.logout().await?;
    Ok(())
}

/// Saves a message into a folder (`APPEND`), e.g. a copy of what was sent
/// into "Sent", already marked as read.
pub async fn append_message(config: &ImapConfig, folder: &str, raw: &[u8]) -> Result<()> {
    let mut session = connect(config).await?;
    session.append(folder, Some("(\\Seen)"), None, raw).await?;
    session.logout().await?;
    Ok(())
}

/// The decoded content of one attachment.
pub async fn fetch_attachment(
    config: &ImapConfig,
    folder: &str,
    uid: u32,
    index: usize,
) -> Result<AttachmentData> {
    let mut session = connect(config).await?;
    session.examine(folder).await?;
    let (raw, _) = fetch_raw(&mut session, uid).await?;
    session.logout().await?;
    extract_attachment(&raw, index)
        .ok_or_else(|| Error::NotFound(format!("attachment {index} of message {uid}")))
}

/// Downloads the whole message (`BODY.PEEK[]` does not mark it as read).
/// Several whole messages at once (the background download of recent
/// mail), without marking them read.
pub(crate) async fn fetch_raw_many(
    config: &ImapConfig,
    folder: &str,
    uids: &[u32],
) -> Result<Vec<(u32, Vec<u8>)>> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let mut session = connect(config).await?;
    session.examine(folder).await?;
    let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let fetches: Vec<Fetch> = session.uid_fetch(set, "(UID BODY.PEEK[])").await?.try_collect().await?;
    session.logout().await?;
    Ok(fetches.iter().filter_map(|f| Some((f.uid?, f.body()?.to_vec()))).collect())
}

async fn fetch_raw(session: &mut Session, uid: u32) -> Result<(Vec<u8>, bool)> {
    let fetches: Vec<Fetch> =
        session.uid_fetch(uid.to_string(), "(UID FLAGS BODY.PEEK[])").await?.try_collect().await?;
    // Some servers also send unrelated FETCH lines (flag updates): keep ours.
    let fetch = fetches
        .iter()
        .find(|f| f.uid == Some(uid) && f.body().is_some())
        .ok_or_else(|| Error::NotFound(format!("message {uid}")))?;
    Ok((fetch.body().unwrap_or_default().to_vec(), is_unread(fetch)))
}

async fn store_seen(session: &mut Session, uid: u32, seen: bool) -> Result<()> {
    let change = if seen { "+FLAGS.SILENT (\\Seen)" } else { "-FLAGS.SILENT (\\Seen)" };
    // The answer is a stream we must read to the end, even if we ignore it.
    let _: Vec<Fetch> = session.uid_store(uid.to_string(), change).await?.try_collect().await?;
    Ok(())
}

pub(crate) fn is_unread(fetch: &Fetch) -> bool {
    !fetch.flags().any(|f| matches!(f, Flag::Seen))
}

/// Starred: `\Flagged` (what Gmail calls a star and Outlook a flag).
pub(crate) fn is_flagged(fetch: &Fetch) -> bool {
    fetch.flags().any(|f| matches!(f, Flag::Flagged))
}

/// Adds or removes a flag (`\Seen`, `\Flagged`) on several messages at once.
pub async fn set_flag(config: &ImapConfig, folder: &str, uids: &[u32], flag: &str, on: bool) -> Result<()> {
    if uids.is_empty() {
        return Ok(());
    }
    let mut session = connect(config).await?;
    session.select(folder).await?;
    let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let op = if on { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
    let _: Vec<Fetch> = session.uid_store(set, format!("{op} ({flag})")).await?.try_collect().await?;
    session.logout().await?;
    Ok(())
}

/// The UIDs, in `folder`, of the messages with these `Message-ID`s (to
/// undo a move: the messages got new UIDs over there).
pub async fn find_by_message_id(config: &ImapConfig, folder: &str, ids: &[String]) -> Result<Vec<u32>> {
    let mut session = connect(config).await?;
    session.select(folder).await?;
    let mut uids = Vec::new();
    for id in ids.iter().filter(|id| !id.is_empty()) {
        // Quotes and backslashes would break the IMAP string: such ids don't occur in practice.
        if id.contains(['"', '\\', '\r', '\n']) {
            continue;
        }
        let found = session.uid_search(format!("HEADER Message-ID \"{id}\"")).await?;
        uids.extend(found);
    }
    session.logout().await?;
    uids.sort_unstable();
    uids.dedup();
    Ok(uids)
}

/// Opens a TLS connection and logs in.
pub(crate) async fn connect(config: &ImapConfig) -> Result<Session> {
    tracing::debug!(host = %config.host, port = config.port, "connecting to IMAP server");

    let tcp = TcpStream::connect((config.host.as_str(), config.port)).await?;
    let tls =
        TlsConnector::from(Arc::new(tls_config(config)?)).connect(server_name(&config.host)?, tcp).await?;

    let mut client = async_imap::Client::new(tls);
    // The server speaks first ("* OK ready"); we must read that greeting.
    let _greeting = client
        .read_response()
        .await?
        .ok_or_else(|| Error::Config("the IMAP server closed the connection".into()))?;

    // On failure, `login`/`authenticate` give the client back too; we only
    // keep the error.
    match &config.auth {
        ImapAuth::Password(password) => {
            client.login(&config.username, password).await.map_err(|(err, _client)| Error::Imap(err))
        }
        ImapAuth::Bearer(token) => xoauth2_login(client, &config.username, token).await,
        ImapAuth::OAuth(session) => {
            let token = session.access_token().await?;
            let result = xoauth2_login(client, &config.username, &token).await;
            if result.is_err() {
                // Maybe revoked: get a new one next time.
                session.forget().await;
            }
            result
        }
    }
}

/// `AUTHENTICATE XOAUTH2`: logs in with an OAuth access token.
async fn xoauth2_login(
    client: async_imap::Client<TlsStream<TcpStream>>,
    user: &str,
    token: &str,
) -> Result<Session> {
    client.authenticate("XOAUTH2", XOAuth2::new(user, token)).await.map_err(|(err, _client)| match err {
        async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => Error::SignIn(
            "The mail server refused the OAuth2 sign-in. Check that IMAP is enabled for this mailbox \
             (Microsoft 365: admin center › Users › Mail › Email apps; Google Workspace: admin console \
             › Gmail › End user access)."
                .into(),
        ),
        other => Error::Imap(other),
    })
}

/// Which certificates we trust: Mozilla's list, plus an optional extra
/// certificate authority for self-hosted servers (`HAVRE_IMAP_CA_FILE`).
fn tls_config(config: &ImapConfig) -> Result<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    if let Some(path) = &config.ca_file {
        use rustls::pki_types::{CertificateDer, pem::PemObject};
        let certs = CertificateDer::pem_file_iter(path)
            .map_err(|e| Error::Config(format!("cannot read HAVRE_IMAP_CA_FILE {path}: {e}")))?;
        for cert in certs {
            let cert = cert.map_err(|e| Error::Config(format!("invalid certificate in {path}: {e}")))?;
            roots.add(cert).map_err(|e| Error::Config(format!("invalid certificate in {path}: {e}")))?;
        }
    }

    Ok(rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth())
}

fn server_name(host: &str) -> Result<rustls::pki_types::ServerName<'static>> {
    rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|_| Error::Config(format!("invalid IMAP host name: {host}")))
}
