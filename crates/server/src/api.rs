//! JSON API, mounted under `/api`.
//!
//! | Method | Path                                             | What                              |
//! |--------|--------------------------------------------------|-----------------------------------|
//! | GET    | /health                                          | server status                     |
//! | GET    | /auth/status                                     | first run? signed in? admin?      |
//! | POST   | /auth/setup · /auth/login  `{password}`          | choose the password · sign in     |
//! | POST   | /auth/logout                                     | sign out                          |
//! | POST   | /auth/password  `{current, new}`                 | change it (signs out other browsers) |
//! | GET    | /auth/sessions · DELETE /auth/sessions/others    | signed-in browsers                |
//! | GET    | /accounts                                        | accounts, with status and unread  |
//! | POST   | /accounts  `{name, email, imap_host, …}`         | add (after testing the login)     |
//! | PATCH  | /accounts/{id}  `{name?, password?, …}`          | edit                              |
//! | DELETE | /accounts/{id}                                   | remove (local copy only)          |
//! | GET    | /accounts/detect?email=                          | Google / Microsoft mailbox?       |
//! | POST   | /accounts/oauth  `{name, email, imap_host, …}` or `{reconnect}` | start OAuth2      |
//! | GET    | /accounts/oauth/{flow}                           | poll it: pending / done / failed  |
//! | GET    | /oauth/{provider}/callback?code=&state=          | the provider sends the pop-up back here |
//! | GET    | /mail/folders?account=                           | folders, special ones first       |
//! | GET    | /mail/messages?account=&folder=&limit=&before=&q=&remote= | a page / a search        |
//! | GET    | /mail/messages/{uid}?account=&folder=&images=    | one full message                  |
//! | PUT    | /mail/messages/{uid}/seen?account=&folder=  `{seen}` | mark read / unread            |
//! | POST   | /mail/actions  `{account, action, folder, uids, to?}` | archive / trash / move       |
//! | GET    | /mail/messages/{uid}/attachments/{i}?account=&folder= | download an attachment       |
//! | POST   | /mail/sync?account=&folder=                      | sync now and wait                 |
//! | GET    | /mail/status?account=                            | sync status                       |
//! | GET    | /events                                          | live changes (Server-Sent Events) |
//! | GET    | /contacts?q=                                     | recipient suggestions             |
//! | GET    | /drafts · POST /drafts `{account, kind, folder?, uid?}` | drafts · start one (reply…) |
//! | POST   | /drafts/from-message `{account, folder, uid}`  | finish a draft from the Drafts folder |
//! | GET/PATCH/DELETE | /drafts/{id}  `{to?, subject?, body_html?, rich?…}` | one draft (autosave; delete: also on the server) |
//! | POST   | /drafts/{id}/close                               | save it on the server now (window closed) |
//! | POST   | /drafts/{id}/attachments?filename=  (raw body)   | attach a file                     |
//! | DELETE | /drafts/{id}/attachments/{attachment}            | remove it                         |
//! | POST   | /drafts/{id}/send `{at?}`                        | send after the undo delay, or at `at` (send later); answers when |
//! | POST   | /drafts/{id}/unsend                              | undo send / don't send: a draft again |
//! | GET    | /mail/scheduled                                  | messages waiting to be sent (and failed ones) |
//! | GET/PUT | /mail/settings `{undo_seconds, compose_format}` | Mail's settings                   |
//!
//! The Agenda's routes are in `agenda.rs`; AI in `ai.rs`; MCP in `mcp.rs`; apps, people and the unread
//! inbox (what the apps share) in `platform.rs`; Tasks, Notes and Files in `tasks.rs`, `notes.rs`, `files.rs`.
//!
//! Every route except `/health` and the first four `/auth` ones needs the
//! session cookie (`401` otherwise).
//!
//! `account` defaults to the first account, `folder` to `INBOX`. Folder names
//! go in the query string because they can contain `/`.

use std::{convert::Infallible, time::Duration};

use axum::{
    Json, Router,
    body::Body,
    extract::DefaultBodyLimit,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    middleware,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{delete, get, patch, post, put},
};
use futures::{Stream, StreamExt};
use havre_core::{
    accounts::{AccountView, OAuthRequest, SignInStart, SignInState},
    compose::{
        self, Draft, DraftAttachment, DraftUpdate, NewDraft,
        outbox::{MailSettings, SendReply},
    },
    mail::{ActionOutcome, AttachmentData, ListQuery, MessageDetail, MessagePage, RenderOptions},
    oauth::{self, Provider},
    store::{Account, AccountUpdate, Contact, FolderInfo, NewAccount},
    sync::{AccountEvent, Action, SyncEvent, SyncStatus},
    users::UserId,
};
use serde::Deserialize;
use serde_json::json;
use tokio_stream::wrappers::{BroadcastStream, errors::BroadcastStreamRecvError};

use crate::{
    AppState,
    auth::{self, CurrentUser},
    mail_source::MailSource,
};

/// The API. Everything but `/health` and `/auth/{status,setup,login,logout}`
/// needs a session (see `auth.rs`).
pub fn router(state: AppState) -> Router<AppState> {
    let protected = Router::new()
        .route("/accounts", get(accounts).post(add_account))
        .route("/accounts/{id}", patch(update_account).delete(remove_account))
        .route("/accounts/detect", get(detect_account))
        .route("/accounts/oauth", post(start_oauth))
        .route("/accounts/oauth/{flow}", get(poll_oauth))
        .route("/oauth/{provider}/callback", get(oauth_callback))
        .route("/mail/folders", get(folders))
        .route("/mail/messages", get(messages))
        .route("/mail/messages/{uid}", get(message))
        .route("/mail/messages/{uid}/seen", put(set_seen))
        .route("/mail/messages/{uid}/attachments/{index}", get(attachment))
        .route("/mail/actions", post(apply_action))
        .route("/mail/sync", post(sync_now))
        .route("/mail/status", get(status))
        .route("/events", get(events))
        .route("/contacts", get(contacts))
        .route("/drafts", get(list_drafts).post(create_draft))
        .route("/drafts/from-message", post(draft_from_message))
        .route("/drafts/{id}", get(get_draft).patch(update_draft).delete(delete_draft))
        .route("/drafts/{id}/close", post(close_draft))
        .route(
            "/drafts/{id}/attachments",
            post(add_attachment).layer(DefaultBodyLimit::max(compose::MAX_ATTACHMENTS + 1024 * 1024)),
        )
        .route("/drafts/{id}/attachments/{attachment}", delete(remove_attachment))
        .route("/drafts/{id}/send", post(send_draft))
        .route("/drafts/{id}/unsend", post(unsend_draft))
        .route("/mail/scheduled", get(scheduled))
        .route("/mail/settings", get(mail_settings).put(set_mail_settings))
        .merge(auth::protected_routes())
        .merge(crate::agenda::routes())
        .merge(crate::tasks::routes())
        .merge(crate::contacts::routes())
        .merge(crate::notes::routes())
        .merge(crate::files::routes())
        .merge(crate::platform::routes())
        .merge(crate::organize::routes())
        .merge(crate::backups::routes())
        .merge(crate::push::routes())
        .merge(crate::ai::routes())
        .merge(crate::mcp::routes())
        .route_layer(middleware::from_fn_with_state(state, auth::require_session));
    Router::new()
        .route("/health", get(health))
        .merge(auth::routes())
        .merge(protected)
        .layer(middleware::from_fn(auth::same_origin))
}

/// `GET /api/health` — is the server up?
async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "demo": matches!(state.mail, MailSource::Demo(_)),
        // Which OAuth2 providers are set up in .env.
        "oauth": {
            "microsoft": state.mail.oauth_enabled(Provider::Microsoft),
            "google": state.mail.oauth_enabled(Provider::Google),
        },
    }))
}

// ---- Accounts -------------------------------------------------------------------

async fn accounts(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<AccountView>>, ApiError> {
    Ok(Json(state.mail.accounts(user).await?))
}

/// `POST /api/accounts` — tests the login first: a wrong password is
/// reported right away (400) and nothing is saved.
async fn add_account(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(new): Json<NewAccount>,
) -> Result<(StatusCode, Json<Account>), ApiError> {
    Ok((StatusCode::CREATED, Json(state.mail.add_account(user, new).await?)))
}

async fn update_account(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Json(update): Json<AccountUpdate>,
) -> Result<Json<Account>, ApiError> {
    Ok(Json(state.mail.update_account(user, id, update).await?))
}

async fn remove_account(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.mail.remove_account(user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct DetectQuery {
    email: String,
}

/// `GET /api/accounts/detect?email=` — is the address hosted by Google
/// (Gmail, Google Workspace) or Microsoft (Outlook.com, Microsoft 365)?
/// Company domains included: they need OAuth2 too.
async fn detect_account(Query(q): Query<DetectQuery>) -> Json<serde_json::Value> {
    let email = q.email.trim();
    let provider = if email.contains('@') { oauth::detect_provider(email).await } else { None };
    Json(json!({ "provider": provider }))
}

/// `POST /api/accounts/oauth` — returns the sign-in page to open (or, for
/// Microsoft with `mode: "code"`, a code), and a flow id to poll.
async fn start_oauth(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(request): Json<OAuthRequest>,
) -> Result<Json<SignInStart>, ApiError> {
    Ok(Json(state.mail.start_oauth(user, request).await?))
}

async fn poll_oauth(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(flow): Path<String>,
) -> Result<Json<SignInState>, ApiError> {
    Ok(Json(state.mail.poll_oauth(user, &flow).await?))
}

#[derive(Deserialize)]
struct CallbackQuery {
    #[serde(default)]
    state: String,
    code: Option<String>,
    error: Option<String>,
    #[serde(default)]
    error_description: String,
}

/// `GET /api/oauth/{provider}/callback` — where Google or Microsoft sends
/// the sign-in window back. Answers with a tiny page that closes the
/// window; the main page learns the outcome by polling `/accounts/oauth/{flow}`.
async fn oauth_callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let provider = Provider::parse(&provider).unwrap_or(Provider::Microsoft);
    let error = q.error.as_deref().map(|e| oauth::explain_redirect_error(provider, e, &q.error_description));
    let outcome = state.mail.oauth_callback(&q.state, q.code.as_deref(), error.as_deref()).await;
    let (title, message, ok) = match outcome {
        Ok(()) => ("Signed in", "You can close this window: Havre is opening your mailbox.".to_owned(), true),
        Err(err) => ("Sign-in failed", err.to_string(), false),
    };
    let page = CALLBACK_PAGE
        .replace("{title}", title)
        .replace("{message}", &html_escape(&message))
        .replace("{ok}", if ok { "true" } else { "false" });
    let mut response = axum::response::Html(page).into_response();
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The page shown in the sign-in window when the provider sends it back.
const CALLBACK_PAGE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width">
<title>{title} · Havre</title>
<style>
  :root { color-scheme: light dark; --bg: #f6f4ef; --text: #24211c; --muted: #6f6a60; }
  @media (prefers-color-scheme: dark) { :root { --bg: #161513; --text: #ece8df; --muted: #a39d92; } }
  body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: var(--bg);
         color: var(--text); font: 15px/1.5 system-ui, sans-serif; }
  main { max-width: 360px; padding: 24px; text-align: center; }
  h1 { font-size: 20px; font-weight: 600; margin: 0 0 8px; }
  p { margin: 0 0 18px; color: var(--muted); }
  button { font: inherit; padding: 8px 16px; border-radius: 10px; border: 1px solid var(--muted);
           background: transparent; color: var(--text); cursor: pointer; }
</style></head>
<body><main><h1>{title}</h1><p>{message}</p><button onclick="window.close()">Close this window</button></main>
<script>
  try { window.opener && window.opener.postMessage({ type: 'havre-oauth' }, location.origin) } catch (e) {}
  if ({ok}) setTimeout(function () { window.close() }, 600)
</script></body></html>"#;

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ---- Writing ------------------------------------------------------------------------

#[derive(Deserialize)]
struct ContactQuery {
    #[serde(default)]
    q: String,
}

async fn contacts(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<ContactQuery>,
) -> Result<Json<Vec<Contact>>, ApiError> {
    if q.q.trim().len() < 2 {
        return Ok(Json(Vec::new()));
    }
    Ok(Json(state.mail.contacts(user, &q.q).await?))
}

async fn list_drafts(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<Draft>>, ApiError> {
    Ok(Json(state.mail.drafts(user)?.list().await?))
}

/// `POST /api/drafts` — a new message, or a reply / forward of
/// `{folder, uid}` (prefilled: recipients, subject, quoted text…).
async fn create_draft(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(start): Json<NewDraft>,
) -> Result<(StatusCode, Json<Draft>), ApiError> {
    Ok((StatusCode::CREATED, Json(state.mail.new_draft(user, start).await?)))
}

async fn get_draft(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<Draft>, ApiError> {
    Ok(Json(state.mail.drafts(user)?.get(id).await?))
}

/// `PATCH /api/drafts/{id}` — autosave while typing.
async fn update_draft(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Json(update): Json<DraftUpdate>,
) -> Result<Json<Draft>, ApiError> {
    Ok(Json(state.mail.drafts(user)?.update(id, &update).await?))
}

async fn delete_draft(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state.mail.discard_draft(user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct FromMessage {
    account: i64,
    folder: String,
    uid: u32,
}

async fn draft_from_message(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(m): Json<FromMessage>,
) -> Result<Json<Draft>, ApiError> {
    Ok(Json(state.mail.draft_from_message(user, m.account, &m.folder, m.uid).await?))
}

async fn close_draft(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<Draft>, ApiError> {
    Ok(Json(state.mail.save_draft_to_server(user, id).await?))
}

#[derive(Deserialize)]
struct AttachmentQuery {
    filename: String,
}

/// `POST /api/drafts/{id}/attachments?filename=report.pdf` — the file is
/// the request body, its type the `Content-Type` header.
async fn add_attachment(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Query(q): Query<AttachmentQuery>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<DraftAttachment>), ApiError> {
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|m| !m.is_empty())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let file = AttachmentData { filename: q.filename, mime, bytes: body.to_vec() };
    Ok((StatusCode::CREATED, Json(state.mail.drafts(user)?.add_attachment(id, &file).await?)))
}

async fn remove_attachment(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((id, attachment)): Path<(i64, i64)>,
) -> Result<StatusCode, ApiError> {
    state.mail.drafts(user)?.remove_attachment(id, attachment).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
struct SendAt {
    /// Send later: when (unix seconds). Missing: after the undo delay.
    #[serde(default)]
    at: Option<i64>,
}

/// `POST /api/drafts/{id}/send`
async fn send_draft(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
    Json(body): Json<SendAt>,
) -> Result<Json<SendReply>, ApiError> {
    Ok(Json(state.mail.manager()?.send_draft_at(user, id, body.at).await?))
}

/// `POST /api/drafts/{id}/unsend` — Undo, or "Don't send".
async fn unsend_draft(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<Draft>, ApiError> {
    Ok(Json(state.mail.manager()?.unsend(user, id).await?))
}

async fn scheduled(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<Vec<Draft>>, ApiError> {
    Ok(Json(state.mail.drafts(user)?.scheduled().await?))
}

async fn mail_settings(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<MailSettings>, ApiError> {
    Ok(Json(state.mail.drafts(user)?.mail_settings().await?))
}

async fn set_mail_settings(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(settings): Json<MailSettings>,
) -> Result<Json<MailSettings>, ApiError> {
    Ok(Json(state.mail.drafts(user)?.set_mail_settings(&settings).await?))
}

// ---- Mail -------------------------------------------------------------------------

/// The `?account=` of a request, or the first account.
/// Whose it is gets checked where it's used (the sync engine is the user's).
async fn account(state: &AppState, user: UserId, requested: Option<i64>) -> Result<i64, ApiError> {
    match requested {
        Some(id) => Ok(id),
        None => Ok(state.mail.default_account(user).await?),
    }
}

#[derive(Deserialize)]
struct Where {
    account: Option<i64>,
    #[serde(default = "inbox")]
    folder: String,
}

fn inbox() -> String {
    "INBOX".into()
}

#[derive(Deserialize)]
struct AccountOnly {
    account: Option<i64>,
}

async fn folders(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<AccountOnly>,
) -> Result<Json<Vec<FolderInfo>>, ApiError> {
    let id = account(&state, user, q.account).await?;
    Ok(Json(state.mail.folders(user, id).await?))
}

#[derive(Deserialize)]
struct MessagesQuery {
    account: Option<i64>,
    #[serde(default = "inbox")]
    folder: String,
    limit: Option<u32>,
    /// Only messages older than this UID ("load more").
    before: Option<u32>,
    /// Search text.
    q: Option<String>,
    /// `?remote=1`: search on the mail server (message bodies too).
    #[serde(default)]
    remote: u8,
}

async fn messages(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<MessagesQuery>,
) -> Result<Json<MessagePage>, ApiError> {
    let id = account(&state, user, q.account).await?;
    let query = ListQuery { limit: q.limit.unwrap_or(50).clamp(1, 500), before_uid: q.before, search: q.q };
    Ok(Json(state.mail.messages(user, id, &q.folder, &query, q.remote == 1).await?))
}

#[derive(Deserialize)]
struct MessageQuery {
    account: Option<i64>,
    #[serde(default = "inbox")]
    folder: String,
    /// `?images=1` keeps remote images.
    #[serde(default)]
    images: u8,
    /// `?mark_read=0` opens without marking as read.
    #[serde(default = "one")]
    mark_read: u8,
}

fn one() -> u8 {
    1
}

async fn message(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(uid): Path<u32>,
    Query(q): Query<MessageQuery>,
) -> Result<Json<MessageDetail>, ApiError> {
    let id = account(&state, user, q.account).await?;
    let folder = q.folder.clone();
    // Embedded images point back to our own attachment endpoint.
    let attachment_url = move |index: usize| {
        format!("/api/mail/messages/{uid}/attachments/{index}?account={id}&folder={}", encode(&folder))
    };
    let options = RenderOptions { allow_remote_images: q.images == 1, attachment_url: &attachment_url };
    Ok(Json(state.mail.message(user, id, &q.folder, uid, q.mark_read == 1, &options).await?))
}

#[derive(Deserialize)]
struct SeenBody {
    seen: bool,
}

async fn set_seen(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(uid): Path<u32>,
    Query(q): Query<Where>,
    Json(body): Json<SeenBody>,
) -> Result<StatusCode, ApiError> {
    let id = account(&state, user, q.account).await?;
    state.mail.set_seen(user, id, &q.folder, uid, body.seen).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ActionBody {
    account: Option<i64>,
    action: Action,
    folder: String,
    uids: Vec<u32>,
    /// Target folder, for `move`.
    to: Option<String>,
}

/// What an action did, and what Undo needs: the messages' Message-IDs.
#[derive(serde::Serialize)]
struct Applied {
    #[serde(flatten)]
    outcome: ActionOutcome,
    message_ids: Vec<String>,
}

/// `POST /api/mail/actions` — archive, delete or move messages.
async fn apply_action(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<ActionBody>,
) -> Result<Json<Applied>, ApiError> {
    if body.uids.is_empty() {
        return Err(ApiError::BadRequest("no messages selected".into()));
    }
    let id = account(&state, user, body.account).await?;
    let (outcome, message_ids) = match &state.mail {
        MailSource::Accounts(m) => {
            m.engine(user, id)
                .await?
                .apply_undoable(body.action, &body.folder, &body.uids, body.to.as_deref())
                .await?
        }
        MailSource::Demo(_) => (
            state.mail.apply(user, id, body.action, &body.folder, &body.uids, body.to.as_deref()).await?,
            Vec::new(),
        ),
    };
    Ok(Json(Applied { outcome, message_ids }))
}

#[derive(Deserialize)]
struct SyncQuery {
    account: Option<i64>,
    folder: Option<String>,
}

/// `POST /api/mail/sync` — sync now (one folder, or everything) and answer
/// when done. Gives up waiting after 60 s; the sync itself goes on.
async fn sync_now(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<SyncQuery>,
) -> Result<StatusCode, ApiError> {
    let id = account(&state, user, q.account).await?;
    match tokio::time::timeout(Duration::from_secs(60), state.mail.sync_now(user, id, q.folder)).await {
        Ok(result) => result?,
        Err(_) => return Ok(StatusCode::ACCEPTED),
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/mail/status`
async fn status(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<AccountOnly>,
) -> Result<Json<SyncStatus>, ApiError> {
    let id = account(&state, user, q.account).await?;
    Ok(Json(state.mail.status(user, id).await?))
}

/// `GET /api/events` — Server-Sent Events: the browser keeps this request
/// open and receives a line of JSON each time something changes, in any of
/// the user's accounts. Each event carries its `account` id. Events of other
/// users (their new mail, their reminders) are never sent here.
async fn events(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // First, the current status of every account.
    let initial: Vec<AccountEvent> = state
        .mail
        .accounts(user)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|a| AccountEvent { account: a.account.id, user, event: SyncEvent::Status { status: a.status } })
        .collect();
    let updates = BroadcastStream::new(state.mail.subscribe()).filter_map(move |item| {
        let event = match item {
            // Everyone's events go through the channel: this user's only.
            Ok(event) if event.user == user => Some(event),
            Ok(_) => None,
            // We were too slow and missed events: tell the UI to reload everything.
            Err(BroadcastStreamRecvError::Lagged(_)) => {
                Some(AccountEvent { account: 0, user, event: SyncEvent::Accounts })
            }
        };
        // `filter_map` on a stream wants a future: `ready` is one that's already done.
        std::future::ready(event)
    });
    let stream = futures::stream::iter(initial)
        .chain(updates)
        .map(|event| Ok(Event::default().json_data(&event).unwrap_or_default()))
        // Stop when the server shuts down, so it can exit cleanly.
        .take_until(state.shutdown.clone().wait_for_owned());
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(20)))
}

/// Sends an attachment's bytes.
///
/// Security: an attachment is content from a stranger. If we served an
/// `.html` or `.svg` attachment "inline" from our own site, its scripts could
/// act as the user. So only common image types are shown inline; everything
/// else is forced to download, and the browser is told not to guess types.
async fn attachment(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((uid, index)): Path<(u32, usize)>,
    Query(q): Query<Where>,
) -> Result<Response, ApiError> {
    let id = account(&state, user, q.account).await?;
    let data = state.mail.attachment(user, id, &q.folder, uid, index).await?;

    let safe_inline = matches!(data.mime.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp");
    let content_type = if safe_inline { data.mime.as_str() } else { "application/octet-stream" };
    let disposition = format!(
        "{}; filename=\"{}\"; filename*=UTF-8''{}",
        if safe_inline { "inline" } else { "attachment" },
        data.filename.replace(|c: char| !c.is_ascii_graphic() && c != ' ' || c == '"', "_"),
        encode(&data.filename),
    );

    let mut response = Response::new(Body::from(data.bytes));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=3600"));
    Ok(response)
}

/// Percent-encodes a string for a URL query or a `filename*=` header.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Errors the API can return. `IntoResponse` turns each one into an HTTP
/// status + a JSON body the web UI can display.
pub enum ApiError {
    BadRequest(String),
    Core(havre_core::Error),
    /// No session on a route that needs one.
    Unauthorized,
    /// An admin-only route asked by someone else: answered like a route
    /// that doesn't exist.
    NotFound,
}

// Lets `?` convert a core error into an ApiError.
impl From<havre_core::Error> for ApiError {
    fn from(err: havre_core::Error) -> Self {
        Self::Core(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "Sign in to Havre first.".into()),
            Self::NotFound => (StatusCode::NOT_FOUND, "Not found.".into()),
            Self::Core(err @ havre_core::Error::NotFound(_)) => (StatusCode::NOT_FOUND, err.to_string()),
            // Config errors here are user mistakes ("already in this folder",
            // "wrong password" when adding an account…).
            Self::Core(havre_core::Error::Config(message) | havre_core::Error::SignIn(message)) => {
                (StatusCode::BAD_REQUEST, message)
            }
            // The AI server failed: its message says why.
            Self::Core(havre_core::Error::Ai(message)) => (StatusCode::BAD_GATEWAY, message),
            Self::Core(err) => {
                tracing::error!("mail request failed: {err}");
                (StatusCode::BAD_GATEWAY, err.to_string())
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::encode;

    #[test]
    fn encodes_folder_names() {
        assert_eq!(encode("[Gmail]/Sent Mail"), "%5BGmail%5D%2FSent%20Mail");
        assert_eq!(encode("INBOX"), "INBOX");
    }
}
