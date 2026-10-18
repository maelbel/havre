//! Tasks API, under `/api/tasks` (needs a session, like the rest).
//!
//! | Method | Path | What |
//! |--------|------|------|
//! | GET    | /tasks?list=&done=&from=&to=&person=&message_id=&q=&limit= | tasks (still to do + done in the last 30 days) |
//! | POST   | /tasks `{list?, title, notes, due?, due_at?, remind_at?, done, links?}` | create |
//! | POST   | /tasks/quick `{text, utc_offset, links?}` | quick add: "Call Thomas fri 10:00 #work" |
//! | POST   | /tasks/parse `{text, utc_offset}` | what quick add understands (preview) |
//! | POST   | /tasks/from-mail `{account, folder, uid, text, utc_offset, list?, due?}` | a task linked to an email and its sender |
//! | GET    | /tasks/{id} | one task |
//! | PUT    | /tasks/{id} (same body as create) | change (or move to another list) |
//! | PUT    | /tasks/{id}/done `{done}` | tick / untick |
//! | DELETE | /tasks/{id} | delete |
//! | GET    | /tasks/{id}/mail | where its email is now `{account, folder, uid}` |
//! | GET · POST | /tasks/lists `{name}` | lists · new Havre list |
//! | PATCH  | /tasks/lists/{id} `{name?, color?, is_default?}` | rename (Havre lists), recolor, default |
//! | DELETE | /tasks/lists/{id} | delete a Havre list (with its tasks) |
//! | POST   | /tasks/sync | sync now and wait |
//! | GET    | /tasks/sources | accounts and CalDAV servers that can sync tasks |
//! | PUT    | /tasks/sources/{key} `{enabled}` | turn task sync on/off for one |

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use havre_core::tasks::{
    Links, MailLink, PersonLink, Task, TaskInput, TaskList, TaskQuery, TaskSource, Tasks, parse::Parsed,
};
use serde::{Deserialize, Serialize};

use crate::{AppState, api::ApiError};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/tasks", get(list_tasks).post(create))
        .route("/tasks/quick", post(quick))
        .route("/tasks/parse", post(parse))
        .route("/tasks/from-mail", post(from_mail))
        .route("/tasks/lists", get(lists).post(create_list))
        .route("/tasks/lists/{id}", axum::routing::patch(update_list).delete(delete_list))
        .route("/tasks/sync", post(sync_now))
        .route("/tasks/sources", get(sources))
        .route("/tasks/sources/{key}", put(set_source))
        .route("/tasks/{id}", get(one).put(update).delete(delete))
        .route("/tasks/{id}/done", put(set_done))
        .route("/tasks/{id}/mail", get(locate_mail))
}

fn tasks(state: &AppState) -> Result<&Arc<Tasks>, ApiError> {
    state.tasks.as_ref().ok_or_else(|| ApiError::BadRequest("Tasks aren't available in demo mode.".into()))
}

async fn list_tasks(
    State(state): State<AppState>,
    Query(q): Query<TaskQuery>,
) -> Result<Json<Vec<Task>>, ApiError> {
    Ok(Json(tasks(&state)?.store().tasks(&q).await?))
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<TaskInput>,
) -> Result<(StatusCode, Json<Task>), ApiError> {
    Ok((StatusCode::CREATED, Json(tasks(&state)?.create(&input).await?)))
}

#[derive(Deserialize)]
struct QuickAdd {
    text: String,
    /// The browser's `getTimezoneOffset()`.
    #[serde(default)]
    utc_offset: i32,
    #[serde(default)]
    links: Option<Links>,
}

async fn quick(
    State(state): State<AppState>,
    Json(q): Json<QuickAdd>,
) -> Result<(StatusCode, Json<Task>), ApiError> {
    Ok((StatusCode::CREATED, Json(tasks(&state)?.quick_add(&q.text, q.utc_offset, q.links).await?)))
}

async fn parse(State(state): State<AppState>, Json(q): Json<QuickAdd>) -> Result<Json<Parsed>, ApiError> {
    Ok(Json(tasks(&state)?.parse(&q.text, q.utc_offset).await?))
}

#[derive(Deserialize)]
struct FromMail {
    account: i64,
    folder: String,
    uid: u32,
    /// What you typed (read like quick add: "Confirm the quote fri").
    text: String,
    #[serde(default)]
    utc_offset: i32,
    /// A list chosen in the menu (else `#list`, else the default).
    #[serde(default)]
    list: Option<i64>,
    /// A day chosen in the menu (else what the text says).
    #[serde(default)]
    due: Option<i64>,
}

/// A task made from an email: linked to it (by `Message-ID`, so it's found
/// after a move) and to its sender.
async fn from_mail(
    State(state): State<AppState>,
    Json(r): Json<FromMail>,
) -> Result<(StatusCode, Json<Task>), ApiError> {
    let tasks = tasks(&state)?;
    let store = state.mail.manager()?.database().store(r.account);
    let (subject, from_name, from_address, message_id) = store.mail_link(&r.folder, r.uid).await?;
    let from = from_name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| from_address.clone())
        .unwrap_or_default();
    let links = Links {
        mail: Some(MailLink {
            account: r.account,
            message_id,
            folder: Some(r.folder.clone()),
            uid: Some(r.uid),
            subject,
            from,
        }),
        person: from_address.filter(|a| a.contains('@')).map(|email| PersonLink { email, name: from_name }),
        event: None,
    };
    let parsed = tasks.parse(&r.text, r.utc_offset).await?;
    // A day picked in the menu wins; the time typed stays only on that day.
    let due = r.due.or(parsed.due);
    let due_at = if due == parsed.due { parsed.due_at } else { None };
    let input = TaskInput {
        list: r.list.or(parsed.list),
        title: parsed.title,
        due,
        due_at,
        links: Some(links),
        ..TaskInput::default()
    };
    Ok((StatusCode::CREATED, Json(tasks.create(&input).await?)))
}

async fn one(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<Task>, ApiError> {
    Ok(Json(tasks(&state)?.store().task(id).await?))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<TaskInput>,
) -> Result<Json<Task>, ApiError> {
    Ok(Json(tasks(&state)?.update(id, &input).await?))
}

#[derive(Deserialize)]
struct Done {
    done: bool,
}

async fn set_done(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(d): Json<Done>,
) -> Result<Json<Task>, ApiError> {
    Ok(Json(tasks(&state)?.set_done(id, d.done).await?))
}

async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    tasks(&state)?.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct MailLocation {
    account: i64,
    folder: String,
    uid: u32,
}

/// Where the task's email is now: found by its `Message-ID` (it may have
/// been archived or moved), else where it was.
async fn locate_mail(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<MailLocation>, ApiError> {
    let task = tasks(&state)?.store().task(id).await?;
    let mail = task.links.mail.ok_or_else(|| {
        ApiError::Core(havre_core::Error::NotFound("This task wasn't made from an email.".into()))
    })?;
    let store = state.mail.manager()?.database().store(mail.account);
    if let Some(message_id) = &mail.message_id
        && let Some((folder, uid)) = store.locate(message_id).await?
    {
        return Ok(Json(MailLocation { account: mail.account, folder, uid }));
    }
    match (mail.folder, mail.uid) {
        (Some(folder), Some(uid)) => Ok(Json(MailLocation { account: mail.account, folder, uid })),
        _ => Err(ApiError::Core(havre_core::Error::NotFound(
            "This email is no longer in your mailbox.".into(),
        ))),
    }
}

async fn lists(State(state): State<AppState>) -> Result<Json<Vec<TaskList>>, ApiError> {
    Ok(Json(tasks(&state)?.store().lists().await?))
}

#[derive(Deserialize)]
struct NewList {
    name: String,
}

async fn create_list(
    State(state): State<AppState>,
    Json(l): Json<NewList>,
) -> Result<(StatusCode, Json<TaskList>), ApiError> {
    let tasks = tasks(&state)?;
    let list = tasks.store().create_local_list(&l.name).await?;
    tasks.wake();
    Ok((StatusCode::CREATED, Json(list)))
}

#[derive(Deserialize)]
struct ListUpdate {
    name: Option<String>,
    color: Option<String>,
    is_default: Option<bool>,
}

async fn update_list(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(u): Json<ListUpdate>,
) -> Result<Json<TaskList>, ApiError> {
    let store = tasks(&state)?.store();
    Ok(Json(store.update_list(id, u.name.as_deref(), u.color.as_deref(), u.is_default).await?))
}

async fn delete_list(State(state): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    tasks(&state)?.store().delete_local_list(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn sync_now(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    tasks(&state)?.sync_now().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn sources(State(state): State<AppState>) -> Result<Json<Vec<TaskSource>>, ApiError> {
    Ok(Json(tasks(&state)?.sources().await?))
}

#[derive(Deserialize)]
struct Enabled {
    enabled: bool,
}

async fn set_source(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(e): Json<Enabled>,
) -> Result<Json<Vec<TaskSource>>, ApiError> {
    Ok(Json(tasks(&state)?.set_sync(&key, e.enabled).await?))
}
