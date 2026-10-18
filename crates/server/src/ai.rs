//! AI routes, under `/api/ai` (need a session, like the rest).
//!
//! | Method | Path | What |
//! |--------|------|------|
//! | GET    | /ai/status | on? which model? search by meaning ready? |
//! | GET · PATCH | /ai/settings `{provider?, url?, model?, api_key?, embed_url?, embed_model?, index_mail?, …}` | Settings › AI (on = the AI app is in the dock) |
//! | GET    | /ai/models | models offered by the configured server |
//! | POST   | /ai/test | a tiny request to check the settings |
//! | POST   | /ai/summarize `{account, folder, uid, refresh?}` | summary of a message (streamed text) |
//! | POST   | /ai/draft `{draft, instruction, body}` | reply body from an instruction (streamed text) |
//! | POST   | /ai/brief `{utc_offset, refresh?}` | the morning brief (streamed text) |
//! | GET    | /ai/search?q= | search by meaning |
//! | POST   | /ai/ask `{turns, utc_offset}` | the assistant: answer, steps, proposals (not kept) |
//! | GET    | /ai/conversations | the AI app's conversations, latest first |
//! | POST   | /ai/conversations/ask `{conversation?, question, utc_offset}` | ask in one (none: a new one) → the conversation |
//! | GET · PATCH · DELETE | /ai/conversations/{id} `{title}` | one conversation · rename · delete |
//! | PUT    | /ai/conversations/{id}/turns/{turn}/done `{proposal}` | a proposal you confirmed |
//! | GET · POST | /ai/activity `{action, detail}` | the activity log · note something you confirmed |
//! | GET    | /ai/health | is the AI server answering? `{ok, error?}` |
//! | GET    | /ai/claude-code | Claude Code installed? which version? signed in? |
//! | POST · DELETE | /ai/claude-code/login | start signing in (→ `{url}`) · give up |
//! | POST   | /ai/claude-code/login/complete `{code}` | the code from Anthropic's page → `{success, output}` |
//! | POST   | /ai/claude-code/logout | sign Claude Code out |
//!
//! "Streamed text" is a plain-text response that arrives in pieces (the
//! browser reads it with `fetch` and a stream reader). An error before the
//! first piece is a normal JSON error; after it, the text ends with a line
//! starting with `⚠`.

use std::{convert::Infallible, future::Future, sync::Arc};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures::stream;
use havre_core::ai::{
    Ai, AiStatus, LocalTime,
    assistant::{Answer, Turn},
    conversations::{Conversation, ConversationSummary},
    index::Hit,
    settings::{AiSettings, AiSettingsUpdate},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::{AppState, api::ApiError};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/ai/status", get(status))
        .route("/ai/settings", get(settings).patch(update_settings))
        .route("/ai/models", get(models))
        .route("/ai/test", post(test))
        .route("/ai/summarize", post(summarize))
        .route("/ai/draft", post(draft))
        .route("/ai/brief", post(brief))
        .route("/ai/search", get(search))
        .route("/ai/ask", post(ask))
        .route("/ai/conversations", get(conversations))
        .route("/ai/conversations/ask", post(ask_in))
        .route(
            "/ai/conversations/{id}",
            get(conversation).patch(rename_conversation).delete(delete_conversation),
        )
        .route("/ai/conversations/{id}/turns/{turn}/done", axum::routing::put(mark_done))
        .route("/ai/claude-code", get(claude_status))
        .route("/ai/activity", get(activity).post(confirmed))
        .route("/ai/health", get(health))
        .route("/ai/claude-code/login", post(claude_login_start).delete(claude_login_cancel))
        .route("/ai/claude-code/login/complete", post(claude_login_complete))
        .route("/ai/claude-code/logout", post(claude_logout))
}

fn ai(state: &AppState) -> Result<&Arc<Ai>, ApiError> {
    state.ai.as_ref().ok_or_else(|| ApiError::BadRequest("AI isn't available in demo mode.".into()))
}

async fn status(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(match &state.ai {
        None => json!({ "installed": false, "enabled": false, "search": false }),
        Some(ai) => serde_json::to_value::<AiStatus>(ai.status().await?).unwrap_or_default(),
    }))
}

async fn settings(State(state): State<AppState>) -> Result<Json<AiSettings>, ApiError> {
    Ok(Json(ai(&state)?.settings().await?))
}

async fn update_settings(
    State(state): State<AppState>,
    Json(update): Json<AiSettingsUpdate>,
) -> Result<Json<AiSettings>, ApiError> {
    Ok(Json(ai(&state)?.update_settings(&update).await?))
}

async fn models(State(state): State<AppState>) -> Result<Json<Vec<String>>, ApiError> {
    Ok(Json(ai(&state)?.models().await?))
}

async fn test(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "reply": ai(&state)?.test().await? })))
}

/// Runs `work` in the background and sends its text as it's written.
async fn stream_text<F, Fut>(work: F) -> Result<Response, ApiError>
where
    F: FnOnce(mpsc::Sender<String>) -> Fut + Send + 'static,
    Fut: Future<Output = havre_core::Result<()>> + Send + 'static,
{
    let (tx, mut rx) = mpsc::channel::<String>(64);
    let (done_tx, done_rx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = done_tx.send(work(tx).await);
    });
    // Nothing written before an error: answer with a normal error.
    let Some(first) = rx.recv().await else {
        return match done_rx.await {
            Ok(Err(err)) => Err(err.into()),
            _ => Ok(text_response(Body::empty())),
        };
    };
    // `unfold`: a stream built step by step from a state — here the
    // channel, then (once it's closed) the final result.
    let pieces = stream::once(async move { Ok::<_, Infallible>(first) });
    let rest = stream::unfold((rx, Some(done_rx)), |(mut rx, done)| async move {
        if let Some(piece) = rx.recv().await {
            return Some((Ok(piece), (rx, done)));
        }
        let done = done?;
        match done.await {
            Ok(Err(err)) => Some((Ok(format!("\n\n⚠ {err}")), (rx, None))),
            _ => None,
        }
    });
    use futures::StreamExt;
    Ok(text_response(Body::from_stream(pieces.chain(rest))))
}

fn text_response(body: Body) -> Response {
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // Proxies (nginx…) would otherwise wait for the whole answer.
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

#[derive(Deserialize)]
struct SummarizeRequest {
    account: i64,
    folder: String,
    uid: u32,
    #[serde(default)]
    refresh: bool,
}

async fn summarize(
    State(state): State<AppState>,
    Json(r): Json<SummarizeRequest>,
) -> Result<Response, ApiError> {
    let ai = Arc::clone(ai(&state)?);
    stream_text(move |out| async move { ai.summarize(r.account, &r.folder, r.uid, r.refresh, out).await })
        .await
}

#[derive(Deserialize)]
struct DraftRequest {
    draft: i64,
    #[serde(default)]
    instruction: String,
    /// What the compose window contains now.
    #[serde(default)]
    body: String,
}

async fn draft(State(state): State<AppState>, Json(r): Json<DraftRequest>) -> Result<Response, ApiError> {
    let ai = Arc::clone(ai(&state)?);
    stream_text(move |out| async move { ai.draft_reply(r.draft, &r.instruction, &r.body, out).await }).await
}

#[derive(Deserialize)]
struct BriefRequest {
    /// The browser's `new Date().getTimezoneOffset()`.
    #[serde(default)]
    utc_offset: i32,
    #[serde(default)]
    refresh: bool,
}

async fn brief(State(state): State<AppState>, Json(r): Json<BriefRequest>) -> Result<Response, ApiError> {
    let ai = Arc::clone(ai(&state)?);
    let local = LocalTime::from_js_offset(r.utc_offset);
    stream_text(move |out| async move { ai.brief(local, r.refresh, out).await }).await
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    limit: Option<usize>,
}

async fn search(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<Hit>>, ApiError> {
    Ok(Json(ai(&state)?.search(&q.q, q.limit.unwrap_or(20).min(50)).await?))
}

#[derive(Deserialize)]
struct AskRequest {
    turns: Vec<Turn>,
    #[serde(default)]
    utc_offset: i32,
}

async fn ask(State(state): State<AppState>, Json(r): Json<AskRequest>) -> Result<Json<Answer>, ApiError> {
    let turns: Vec<Turn> = r.turns.into_iter().rev().take(12).rev().collect();
    Ok(Json(ai(&state)?.ask(&turns, LocalTime::from_js_offset(r.utc_offset)).await?))
}

async fn conversations(State(state): State<AppState>) -> Result<Json<Vec<ConversationSummary>>, ApiError> {
    Ok(Json(ai(&state)?.conversations().await?))
}

async fn conversation(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Conversation>, ApiError> {
    Ok(Json(ai(&state)?.conversation(id).await?))
}

#[derive(Deserialize)]
struct AskIn {
    /// `None`: start a new conversation.
    #[serde(default)]
    conversation: Option<i64>,
    question: String,
    #[serde(default)]
    utc_offset: i32,
}

async fn ask_in(State(state): State<AppState>, Json(r): Json<AskIn>) -> Result<Json<Conversation>, ApiError> {
    let local = LocalTime::from_js_offset(r.utc_offset);
    Ok(Json(ai(&state)?.ask_in(r.conversation, &r.question, local).await?))
}

#[derive(Deserialize)]
struct Title {
    title: String,
}

async fn rename_conversation(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(t): Json<Title>,
) -> Result<Json<Conversation>, ApiError> {
    let ai = ai(&state)?;
    ai.rename_conversation(id, &t.title).await?;
    Ok(Json(ai.conversation(id).await?))
}

async fn delete_conversation(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<axum::http::StatusCode, ApiError> {
    ai(&state)?.delete_conversation(id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Done {
    proposal: usize,
}

async fn mark_done(
    State(state): State<AppState>,
    Path((id, turn)): Path<(i64, i64)>,
    Json(d): Json<Done>,
) -> Result<axum::http::StatusCode, ApiError> {
    ai(&state)?.mark_done(id, turn, d.proposal).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn claude_status(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let status = ai(&state)?.claude_status().await?;
    Ok(Json(serde_json::to_value(status).unwrap_or_default()))
}

async fn claude_login_start(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "url": ai(&state)?.claude_login_start().await? })))
}

#[derive(Deserialize)]
struct LoginCode {
    code: String,
}

async fn claude_login_complete(
    State(state): State<AppState>,
    Json(c): Json<LoginCode>,
) -> Result<Json<Value>, ApiError> {
    if c.code.trim().is_empty() {
        return Err(ApiError::BadRequest("Paste the code shown by Anthropic.".into()));
    }
    let (success, output) = ai(&state)?.claude_login_complete(&c.code).await?;
    Ok(Json(json!({ "success": success, "output": output })))
}

async fn claude_login_cancel(State(state): State<AppState>) -> Result<axum::http::StatusCode, ApiError> {
    ai(&state)?.claude_login_cancel().await;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn claude_logout(State(state): State<AppState>) -> Result<axum::http::StatusCode, ApiError> {
    ai(&state)?.claude_logout().await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ActivityQuery {
    limit: Option<u32>,
}

async fn activity(
    State(state): State<AppState>,
    Query(q): Query<ActivityQuery>,
) -> Result<Json<Vec<havre_core::ai::activity::Activity>>, ApiError> {
    Ok(Json(ai(&state)?.activity(q.limit.unwrap_or(100).min(500)).await?))
}

#[derive(Deserialize)]
struct Confirmed {
    action: String,
    #[serde(default)]
    detail: String,
}

/// The palette tells what you confirmed (created an event, opened a draft…).
async fn confirmed(
    State(state): State<AppState>,
    Json(c): Json<Confirmed>,
) -> Result<axum::http::StatusCode, ApiError> {
    ai(&state)?.log_confirmed(c.action.trim(), c.detail.trim()).await;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

async fn health(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(match ai(&state)?.health().await {
        Ok(()) => json!({ "ok": true }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    }))
}
