//! Contacts sync and `.vcf` files, under `/api` (needs a session).
//!
//! | Method | Path | What |
//! |--------|------|------|
//! | GET    | /contacts/settings | accounts and servers that can sync contacts, their address books, the default one |
//! | PUT    | /contacts/sources/{key} `{enabled}` | turn contacts sync on/off for one (`account:3`, `caldav:2`), then sync |
//! | PUT    | /contacts/default `{book}` | where details of someone in no address book go (`null`: Havre only) |
//! | POST   | /contacts/sync | sync now and wait |
//! | POST   | /people `{name, email?}` | someone new, by name (an address is optional) |
//! | POST   | /people/import | a `.vcf` file (the body, as text): into Havre only |
//! | GET    | /people/export.vcf | everyone with details, as one `.vcf` file |
//! | GET    | /people/{email}/photo | their photo from an address book (404: none) |
//!
//! `{email}` is one of their addresses, or `~42` (their contact) for
//! someone without one.

use std::sync::Arc;

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use havre_core::people::{
    PersonDetail,
    books::ImportReport,
    sync::{ContactSettings, ContactSync},
};
use serde::Deserialize;

use crate::{AppState, api::ApiError};

/// A `.vcf` with photos can be big: 20 MB.
const MAX_IMPORT: usize = 20 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/contacts/settings", get(settings))
        .route("/contacts/sources/{key}", put(set_source))
        .route("/contacts/default", put(set_default))
        .route("/contacts/sync", post(sync_now))
        .route("/people", post(create))
        .route("/people/import", post(import).layer(DefaultBodyLimit::max(MAX_IMPORT)))
        .route("/people/export.vcf", get(export))
        .route("/people/{email}/photo", get(photo))
}

fn contacts(state: &AppState) -> Result<&Arc<ContactSync>, ApiError> {
    state
        .contacts
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Contacts aren't available in demo mode.".into()))
}

async fn settings(State(state): State<AppState>) -> Result<Json<ContactSettings>, ApiError> {
    Ok(Json(contacts(&state)?.settings().await?))
}

#[derive(Deserialize)]
struct Enabled {
    enabled: bool,
}

async fn set_source(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<Enabled>,
) -> Result<Json<ContactSettings>, ApiError> {
    Ok(Json(contacts(&state)?.set_source(&key, body.enabled).await?))
}

#[derive(Deserialize)]
struct Default {
    book: Option<i64>,
}

async fn set_default(
    State(state): State<AppState>,
    Json(body): Json<Default>,
) -> Result<Json<ContactSettings>, ApiError> {
    Ok(Json(contacts(&state)?.set_default_book(body.book).await?))
}

async fn sync_now(State(state): State<AppState>) -> Result<Json<ContactSettings>, ApiError> {
    let sync = contacts(&state)?;
    sync.sync_now().await?;
    Ok(Json(sync.settings().await?))
}

#[derive(Deserialize)]
struct NewPerson {
    #[serde(default)]
    name: String,
    email: Option<String>,
}

async fn create(
    State(state): State<AppState>,
    Json(body): Json<NewPerson>,
) -> Result<Json<PersonDetail>, ApiError> {
    let db = state.mail.manager()?.database();
    let page = db.people().create(&body.name, body.email.as_deref()).await?;
    // Into your default address book, if you chose one.
    if let Some(contacts) = &state.contacts {
        contacts.push_later(page.person.contact);
    }
    Ok(Json(page))
}

async fn import(State(state): State<AppState>, body: String) -> Result<Json<ImportReport>, ApiError> {
    let db = state.mail.manager()?.database();
    Ok(Json(db.people().import_vcf(&body).await?))
}

async fn export(State(state): State<AppState>) -> Result<Response, ApiError> {
    let db = state.mail.manager()?.database();
    let text = db.people().export_vcf().await?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/vcard; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"havre-people.vcf\""),
        ],
        text,
    )
        .into_response())
}

async fn photo(State(state): State<AppState>, Path(email): Path<String>) -> Result<Response, ApiError> {
    let Some(photo) = contacts(&state)?.photo(&email).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, photo.mime)
        // The browser keeps it an hour: lists show many at once.
        .header(header::CACHE_CONTROL, "private, max-age=3600")
        // Never run as a page, whatever it claims to be.
        .header(header::CONTENT_SECURITY_POLICY, "default-src 'none'; sandbox")
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from(photo.data))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()))
}
