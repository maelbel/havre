//! `/mcp`: Havre's tools for other AI apps (see `havre_core::mcp`), and
//! the routes of Settings › Claude Code that manage their tokens.
//!
//! | Method | Path | What |
//! |--------|------|------|
//! | POST   | /mcp (Bearer token) | MCP, JSON-RPC 2.0 ("Streamable HTTP", JSON answers) |
//! | GET · POST | /api/mcp/tokens `{name}` | connected apps · connect one (the secret is shown once) |
//! | DELETE | /api/mcp/tokens/{id} | disconnect an app |
//!
//! `/mcp` is outside `/api`: it doesn't use the browser session but its
//! own tokens. Requests from web pages (with an `Origin` header) are
//! refused unless they come from Havre itself, so a website you visit
//! can't use it.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get},
};
use havre_core::mcp::McpToken;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppState, api::ApiError};

/// `/mcp`, mounted at the root.
pub fn endpoint() -> Router<AppState> {
    Router::new().route("/mcp", axum::routing::post(handle).get(no_stream).delete(no_stream))
}

/// Token management, under `/api` (needs the session).
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/mcp/tokens", get(list_tokens).post(create_token))
        .route("/mcp/tokens/{id}", delete(revoke_token))
}

/// We don't push messages to clients (no server-sent stream).
async fn no_stream() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

fn rpc_error(status: StatusCode, code: i64, message: &str) -> Response {
    (status, Json(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": code, "message": message } })))
        .into_response()
}

async fn handle(State(state): State<AppState>, headers: HeaderMap, body: String) -> Response {
    let Some(ai) = &state.ai else {
        return rpc_error(StatusCode::NOT_FOUND, -32000, "MCP isn't available in demo mode.");
    };
    // A browser page from another site (DNS rebinding, CSRF): refuse.
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default();
        let origin_host = origin.split("://").nth(1).unwrap_or_default();
        if origin_host != host {
            return rpc_error(StatusCode::FORBIDDEN, -32000, "Cross-origin requests are not allowed.");
        }
    }
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default()
        .trim();
    let caller = match ai.mcp_caller(token).await {
        Ok(Some(caller)) => caller,
        Ok(None) => {
            let mut r = rpc_error(
                StatusCode::UNAUTHORIZED,
                -32001,
                "Missing or unknown token. Create one in Havre: Settings › Claude Code.",
            );
            r.headers_mut().insert(header::WWW_AUTHENTICATE, "Bearer".parse().unwrap());
            return r;
        }
        Err(err) => return rpc_error(StatusCode::INTERNAL_SERVER_ERROR, -32603, &err.to_string()),
    };
    let Ok(message) = serde_json::from_str::<Value>(&body) else {
        return rpc_error(StatusCode::BAD_REQUEST, -32700, "Parse error");
    };
    // A batch (a list of messages) or one message.
    match message {
        Value::Array(list) => {
            let mut answers = Vec::new();
            for m in &list {
                if let Some(a) = ai.mcp_handle(&caller, m).await {
                    answers.push(a);
                }
            }
            if answers.is_empty() {
                StatusCode::ACCEPTED.into_response()
            } else {
                Json(Value::Array(answers)).into_response()
            }
        }
        m => match ai.mcp_handle(&caller, &m).await {
            Some(answer) => Json(answer).into_response(),
            // A notification: accepted, nothing to say.
            None => StatusCode::ACCEPTED.into_response(),
        },
    }
}

fn ai(state: &AppState) -> Result<&std::sync::Arc<havre_core::ai::Ai>, ApiError> {
    state.ai.as_ref().ok_or_else(|| ApiError::BadRequest("Not available in demo mode.".into()))
}

async fn list_tokens(State(state): State<AppState>) -> Result<Json<Vec<McpToken>>, ApiError> {
    Ok(Json(ai(&state)?.mcp_tokens().list().await?))
}

#[derive(Deserialize)]
struct NewToken {
    #[serde(default)]
    name: String,
}

async fn create_token(
    State(state): State<AppState>,
    Json(new): Json<NewToken>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (token, secret) = ai(&state)?.mcp_tokens().create(&new.name).await?;
    Ok((StatusCode::CREATED, Json(json!({ "token": token, "secret": secret }))))
}

async fn revoke_token(State(state): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    ai(&state)?.mcp_tokens().revoke(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
