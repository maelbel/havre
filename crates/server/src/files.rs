//! Files API, under `/api/files` (needs a session, like the rest).
//!
//! | Method | Path | What |
//! |--------|------|------|
//! | GET    | /files?q=&kind=&person=&sent=&limit= | the files, once each, with counts and people |
//! | GET    | /files/in-message?account=&folder=&uid= | an email's files, with their other versions |
//! | GET    | /files/progress | how far Havre has looked through older mail |
//! | PUT    | /files/settings `{depth}` | how far back to look (0: everything) |
//!
//! A file is downloaded with Mail's route:
//! `GET /mail/messages/{uid}/attachments/{index}?account=&folder=`.

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, put},
};
use havre_core::files::{FileItem, FileList, FileQuery, FileStore, FilesProgress};
use serde::Deserialize;

use crate::{AppState, api::ApiError};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/files", get(list))
        .route("/files/in-message", get(in_message))
        .route("/files/progress", get(progress))
        .route("/files/settings", put(settings))
}

/// Files are read from the database's copy of your mail (not in demo mode).
fn store(state: &AppState) -> Result<FileStore, ApiError> {
    state
        .mail
        .manager()
        .map(|m| m.database().files())
        .map_err(|_| ApiError::BadRequest("Files aren't available in demo mode.".into()))
}

async fn list(State(state): State<AppState>, Query(q): Query<FileQuery>) -> Result<Json<FileList>, ApiError> {
    Ok(Json(store(&state)?.list(&q).await?))
}

#[derive(Deserialize)]
struct Message {
    account: i64,
    folder: String,
    uid: u32,
}

async fn in_message(
    State(state): State<AppState>,
    Query(m): Query<Message>,
) -> Result<Json<Vec<FileItem>>, ApiError> {
    Ok(Json(store(&state)?.in_message(m.account, &m.folder, m.uid).await?))
}

async fn progress(State(state): State<AppState>) -> Result<Json<FilesProgress>, ApiError> {
    Ok(Json(store(&state)?.progress().await?))
}

#[derive(Deserialize)]
struct Settings {
    depth: i64,
}

async fn settings(
    State(state): State<AppState>,
    Json(s): Json<Settings>,
) -> Result<Json<FilesProgress>, ApiError> {
    let files = store(&state)?;
    files.set_depth(s.depth).await?;
    Ok(Json(files.progress().await?))
}
