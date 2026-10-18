//! Notes API, under `/api/notes` (needs a session, like the rest).
//!
//! | Method | Path | What |
//! |--------|------|------|
//! | GET    | /notes?q=&person=&event_uid=&event_start=&message_id=&pinned=&limit= | notes (summaries), pinned first |
//! | POST   | /notes `{title, body, pinned, event?, people, mail?}` | create |
//! | POST   | /notes/from-event `{event}` | a note for a meeting: its title, attendees, headings |
//! | POST   | /notes/from-mail `{account, folder, uid}` | the email's note: the one it has, or a new one linked to it and its sender |
//! | GET    | /notes/people | everyone with notes about them |
//! | GET    | /notes/{id} | one note, with its Markdown read into blocks |
//! | PUT    | /notes/{id} (same body as create) | save |
//! | PUT    | /notes/{id}/pin `{pinned}` | pin / unpin |
//! | PUT    | /notes/{id}/todos/{line} `{done}` | tick a `- [ ]` line (and its task) |
//! | POST   | /notes/{id}/todos/{line}/task `{utc_offset}` | make a task of a `- [ ]` line |
//! | DELETE | /notes/{id} | delete (tasks made from it stay) |
//! | GET    | /notes/{id}/mail | where its email is now `{account, folder, uid}` |

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use havre_core::{
    notes::{
        self, EventLink, MailLink, NoteInput, NotePerson, NoteQuery, NoteStore, NoteSummary, NoteView,
        PersonLink,
    },
    tasks::Links,
};
use serde::{Deserialize, Serialize};

use crate::{AppState, api::ApiError};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/notes", get(list).post(create))
        .route("/notes/from-event", post(from_event))
        .route("/notes/from-mail", post(from_mail))
        .route("/notes/people", get(people))
        .route("/notes/{id}", get(one).put(update).delete(delete))
        .route("/notes/{id}/pin", put(pin))
        .route("/notes/{id}/todos/{line}", put(tick))
        .route("/notes/{id}/todos/{line}/task", post(make_task))
        .route("/notes/{id}/mail", get(locate_mail))
}

/// Notes live in the database, so they need the real one (not demo mode).
fn store(state: &AppState) -> Result<NoteStore, ApiError> {
    state
        .mail
        .manager()
        .map(|m| m.database().notes())
        .map_err(|_| ApiError::BadRequest("Notes aren't available in demo mode.".into()))
}

async fn list(
    State(state): State<AppState>,
    Query(q): Query<NoteQuery>,
) -> Result<Json<Vec<NoteSummary>>, ApiError> {
    Ok(Json(store(&state)?.notes(&q).await?))
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<NoteInput>,
) -> Result<(StatusCode, Json<NoteView>), ApiError> {
    Ok((StatusCode::CREATED, Json(store(&state)?.create(&input).await?)))
}

#[derive(Deserialize)]
struct FromEvent {
    /// The Agenda's id for the event (the occurrence, for a repeating one).
    event: i64,
}

/// A note for a meeting. The server reads the event itself (rather than
/// trusting what the browser sends) and leaves you out of its people.
async fn from_event(
    State(state): State<AppState>,
    Json(r): Json<FromEvent>,
) -> Result<(StatusCode, Json<NoteView>), ApiError> {
    let notes = store(&state)?;
    let agenda =
        state.agenda.as_ref().ok_or_else(|| ApiError::BadRequest("No Agenda in demo mode.".into()))?;
    let event = agenda.store().event(r.event).await?;
    let mine: Vec<String> = state
        .mail
        .manager()?
        .database()
        .accounts()
        .await?
        .into_iter()
        .map(|a| a.email.to_lowercase())
        .collect();
    let people = event
        .attendees
        .iter()
        .filter(|a| a.email.contains('@') && !mine.contains(&a.email.to_lowercase()))
        .map(|a| PersonLink { email: a.email.clone(), name: a.name.clone() })
        .collect();
    // Events from a calendar without UIDs still get a stable key.
    let uid =
        event.uid.clone().filter(|u| !u.is_empty()).unwrap_or_else(|| format!("havre-event-{}", event.id));
    let link = EventLink { uid, title: event.title.clone(), start: event.start };
    let input = notes::from_meeting(link, people);
    Ok((StatusCode::CREATED, Json(notes.create(&input).await?)))
}

#[derive(Deserialize)]
struct FromMail {
    account: i64,
    folder: String,
    uid: u32,
}

/// The note about an email: the one it already has, or a new one titled
/// with its subject, linked to it (by `Message-ID`) and to its sender.
async fn from_mail(
    State(state): State<AppState>,
    Json(r): Json<FromMail>,
) -> Result<(StatusCode, Json<NoteView>), ApiError> {
    let notes = store(&state)?;
    let mail_store = state.mail.manager()?.database().store(r.account);
    let (subject, from_name, from_address, message_id) = mail_store.mail_link(&r.folder, r.uid).await?;
    // Already a note about this email (found by Message-ID, wherever the
    // email is now)? Open that one rather than making a second.
    if let Some(id) = &message_id {
        let query = NoteQuery { message_id: Some(id.clone()), ..NoteQuery::default() };
        if let Some(found) = notes.notes(&query).await?.into_iter().find(|n| n.mail.is_some()) {
            return Ok((StatusCode::OK, Json(notes.view(found.id).await?)));
        }
    }
    let from = from_name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| from_address.clone())
        .unwrap_or_default();
    let people = from_address
        .filter(|a| a.contains('@'))
        .map(|email| vec![PersonLink { email, name: from_name }])
        .unwrap_or_default();
    let input = NoteInput {
        title: notes::title_from_subject(&subject),
        body: String::new(),
        pinned: false,
        event: None,
        people,
        mail: Some(MailLink {
            account: r.account,
            message_id,
            folder: Some(r.folder),
            uid: Some(r.uid),
            subject,
            from,
        }),
    };
    Ok((StatusCode::CREATED, Json(notes.create(&input).await?)))
}

async fn people(State(state): State<AppState>) -> Result<Json<Vec<NotePerson>>, ApiError> {
    Ok(Json(store(&state)?.people().await?))
}

async fn one(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<NoteView>, ApiError> {
    Ok(Json(store(&state)?.view(id).await?))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<NoteInput>,
) -> Result<Json<NoteView>, ApiError> {
    Ok(Json(store(&state)?.update(id, &input).await?))
}

#[derive(Deserialize)]
struct Pin {
    pinned: bool,
}

async fn pin(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(p): Json<Pin>,
) -> Result<Json<NoteView>, ApiError> {
    Ok(Json(store(&state)?.set_pinned(id, p.pinned).await?))
}

#[derive(Deserialize)]
struct Done {
    done: bool,
}

/// Ticks a to-do line. If it became a task, the task is ticked *first*:
/// if Microsoft or Google refuses, nothing changes and you see why.
async fn tick(
    State(state): State<AppState>,
    Path((id, line)): Path<(i64, usize)>,
    Json(d): Json<Done>,
) -> Result<Json<NoteView>, ApiError> {
    let notes = store(&state)?;
    let view = notes.view(id).await?;
    let task = view.blocks.iter().find_map(|b| match b {
        notes::markdown::Block::Todo { line: l, task, .. } if *l == line => *task,
        _ => None,
    });
    if let (Some(task), Some(tasks)) = (task, state.tasks.as_ref()) {
        tasks.set_done(task, d.done).await?;
    }
    let (view, _) = notes.set_todo(id, line, d.done).await?;
    Ok(Json(view))
}

#[derive(Deserialize)]
struct MakeTask {
    /// The browser's `getTimezoneOffset()`, for words like "fri 10:00".
    #[serde(default)]
    utc_offset: i32,
}

/// Turns a `- [ ]` line into a task, read like quick add ("Call Thomas
/// fri" is due Friday) and linked to the note's meeting, first person and
/// email.
async fn make_task(
    State(state): State<AppState>,
    Path((id, line)): Path<(i64, usize)>,
    Json(m): Json<MakeTask>,
) -> Result<(StatusCode, Json<NoteView>), ApiError> {
    let notes = store(&state)?;
    let tasks = state.tasks.as_ref().ok_or_else(|| ApiError::BadRequest("No Tasks in demo mode.".into()))?;
    let text = notes.todo_text(id, line).await?;
    let note = notes.note(id).await?;
    let links = Links { mail: note.mail, person: note.people.into_iter().next(), event: note.event };
    let task = tasks.quick_add(&text, m.utc_offset, Some(links)).await?;
    Ok((StatusCode::CREATED, Json(notes.link_task(id, &text, task.id).await?)))
}

async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    store(&state)?.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct MailLocation {
    account: i64,
    folder: String,
    uid: u32,
}

/// Where the note's email is now: found by its `Message-ID` (it may have
/// been archived or moved), else where it was. Same as for a task.
async fn locate_mail(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<MailLocation>, ApiError> {
    let note = store(&state)?.note(id).await?;
    let mail = note.mail.ok_or_else(|| {
        ApiError::Core(havre_core::Error::NotFound("This note isn't about an email.".into()))
    })?;
    let mail_store = state.mail.manager()?.database().store(mail.account);
    if let Some(message_id) = &mail.message_id
        && let Some((folder, uid)) = mail_store.locate(message_id).await?
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
