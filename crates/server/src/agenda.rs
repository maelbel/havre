//! Agenda API, under `/api/agenda` (needs a session, like the rest).
//!
//! | Method | Path | What |
//! |--------|------|------|
//! | GET    | /agenda/calendars | calendars (all sources) |
//! | PATCH  | /agenda/calendars/{id} `{enabled?, color?, is_default?}` | show/hide, recolor, default |
//! | GET    | /agenda/events?from=&to= | events overlapping the period (unix seconds) |
//! | POST   | /agenda/events `{calendar, title, start, end, all_day, repeat?, time_zone?…}` | create |
//! | PATCH  | /agenda/events/{id}?scope=this\|following\|all (same body) | change (or move to another calendar) |
//! | DELETE | /agenda/events/{id}?scope=this\|following\|all | delete |
//! | POST   | /agenda/sync | sync now and wait |
//! | GET · POST | /agenda/caldav `{name, url, username, password}` | CalDAV servers · add one |
//! | DELETE | /agenda/caldav/{id} | remove one (its calendars go too) |
//! | POST   | /agenda/invitations `{account, folder, uid, index, calendar?}` | add an `.ics` from an email |
//! | GET    | /agenda/invitation?account=&folder=&uid=&index= | the invitation card (time, conflicts, your answer) |
//! | POST   | /agenda/invitation/respond `{account, folder, uid, index, response}` | accept / maybe / decline from Mail |
//! | GET    | /agenda/invitations/pending | meetings waiting for your answer |
//! | POST   | /agenda/events/{id}/respond `{response}` | answer from the Agenda (Microsoft / Google) |
//! | GET    | /agenda/events/{id}/related | recent mail and files from its guests and organizer |
//! | GET    | /agenda/found-in-mail | invitations in your mail still to answer or add |

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch, post},
};
use havre_core::{
    agenda::{Agenda, CalDavAccount, Calendar, Event, EventInput, Scope},
    invitations::{self, InvitationView},
};
use serde::Deserialize;

use crate::{AppState, api::ApiError};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/agenda/calendars", get(calendars))
        .route("/agenda/calendars/{id}", patch(update_calendar))
        .route("/agenda/events", get(events).post(create_event))
        .route("/agenda/events/{id}", patch(update_event).delete(delete_event))
        .route("/agenda/sync", post(sync_now))
        .route("/agenda/caldav", get(caldav_accounts).post(add_caldav))
        .route("/agenda/caldav/{id}", axum::routing::delete(remove_caldav))
        .route("/agenda/invitations", post(add_invitation))
        .route("/agenda/invitation", get(invitation))
        .route("/agenda/invitation/respond", post(respond_invitation))
        .route("/agenda/invitations/pending", get(pending_invitations))
        .route("/agenda/events/{id}/respond", post(respond_event))
        .route("/agenda/events/{id}/related", get(related))
        .route("/agenda/found-in-mail", get(found_in_mail))
}

fn agenda(state: &AppState) -> Result<&Arc<Agenda>, ApiError> {
    state
        .agenda
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("The Agenda isn't available in demo mode.".into()))
}

async fn calendars(State(state): State<AppState>) -> Result<Json<Vec<Calendar>>, ApiError> {
    Ok(Json(agenda(&state)?.store().calendars().await?))
}

#[derive(Deserialize)]
struct CalendarUpdate {
    enabled: Option<bool>,
    color: Option<String>,
    is_default: Option<bool>,
}

async fn update_calendar(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(update): Json<CalendarUpdate>,
) -> Result<Json<Calendar>, ApiError> {
    let calendar = agenda(&state)?
        .update_calendar(id, update.enabled, update.color.as_deref(), update.is_default)
        .await?;
    Ok(Json(calendar))
}

#[derive(Deserialize)]
struct Period {
    from: i64,
    to: i64,
}

async fn events(
    State(state): State<AppState>,
    Query(p): Query<Period>,
) -> Result<Json<Vec<Event>>, ApiError> {
    Ok(Json(agenda(&state)?.events(p.from, p.to).await?))
}

async fn create_event(
    State(state): State<AppState>,
    Json(input): Json<EventInput>,
) -> Result<(StatusCode, Json<Event>), ApiError> {
    Ok((StatusCode::CREATED, Json(agenda(&state)?.create(&input).await?)))
}

/// `?scope=`: which occurrences of a repeating event (ignored for the others).
#[derive(Deserialize)]
struct ScopeQuery {
    #[serde(default)]
    scope: Scope,
}

async fn update_event(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<ScopeQuery>,
    Json(input): Json<EventInput>,
) -> Result<Json<Event>, ApiError> {
    Ok(Json(agenda(&state)?.update(id, &input, q.scope).await?))
}

async fn delete_event(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<ScopeQuery>,
) -> Result<StatusCode, ApiError> {
    agenda(&state)?.delete(id, q.scope).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn sync_now(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    agenda(&state)?.sync_now().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn caldav_accounts(State(state): State<AppState>) -> Result<Json<Vec<CalDavAccount>>, ApiError> {
    Ok(Json(agenda(&state)?.store().caldav_accounts().await?))
}

#[derive(Deserialize)]
struct NewCalDav {
    #[serde(default)]
    name: String,
    url: String,
    username: String,
    password: String,
}

/// Checks the address and login by listing the calendars, then saves.
async fn add_caldav(
    State(state): State<AppState>,
    Json(new): Json<NewCalDav>,
) -> Result<(StatusCode, Json<CalDavAccount>), ApiError> {
    let added = agenda(&state)?.add_caldav(&new.name, &new.url, &new.username, &new.password).await?;
    Ok((StatusCode::CREATED, Json(added)))
}

async fn remove_caldav(State(state): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    agenda(&state)?.remove_caldav(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Invitation {
    account: i64,
    folder: String,
    uid: u32,
    /// The `.ics` attachment's index.
    index: usize,
    calendar: Option<i64>,
}

/// "Add to Agenda" on an invitation received by email.
async fn add_invitation(
    State(state): State<AppState>,
    Json(inv): Json<Invitation>,
) -> Result<(StatusCode, Json<Vec<Event>>), ApiError> {
    let file = state.mail.attachment(inv.account, &inv.folder, inv.uid, inv.index).await?;
    let text = String::from_utf8_lossy(&file.bytes);
    let added = agenda(&state)?.add_from_ics(&text, inv.calendar).await?;
    Ok((StatusCode::CREATED, Json(added)))
}

/// Which attachment of which message.
#[derive(Deserialize)]
struct InvitationRef {
    account: i64,
    folder: String,
    uid: u32,
    index: usize,
}

/// The card Mail shows above an invitation.
async fn invitation(
    State(state): State<AppState>,
    Query(r): Query<InvitationRef>,
) -> Result<Json<InvitationView>, ApiError> {
    let accounts = state.mail.manager()?;
    Ok(Json(invitations::view(accounts, agenda(&state)?, r.account, &r.folder, r.uid, r.index).await?))
}

/// The Agenda's "Found in Mail": invitations still waiting for you.
async fn found_in_mail(
    State(state): State<AppState>,
) -> Result<Json<Vec<invitations::FoundInvitation>>, ApiError> {
    let accounts = state.mail.manager()?;
    Ok(Json(invitations::found_in_mail(accounts, agenda(&state)?, 6).await?))
}

#[derive(Deserialize)]
struct InvitationAnswer {
    #[serde(flatten)]
    invitation: InvitationRef,
    /// `accepted`, `tentative` or `declined`.
    response: String,
}

/// Accept / Maybe / Decline, straight from Mail.
async fn respond_invitation(
    State(state): State<AppState>,
    Json(a): Json<InvitationAnswer>,
) -> Result<Json<InvitationView>, ApiError> {
    let r = a.invitation;
    let accounts = state.mail.manager()?;
    let view =
        invitations::respond(accounts, agenda(&state)?, r.account, &r.folder, r.uid, r.index, &a.response)
            .await?;
    Ok(Json(view))
}

async fn pending_invitations(State(state): State<AppState>) -> Result<Json<Vec<Event>>, ApiError> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Ok(Json(agenda(&state)?.store().pending_invitations(now as i64).await?))
}

#[derive(Deserialize)]
struct Answer {
    response: String,
}

async fn respond_event(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(a): Json<Answer>,
) -> Result<Json<Event>, ApiError> {
    Ok(Json(agenda(&state)?.respond(id, &a.response).await?))
}

/// "Related emails" in the event panel: what its people sent lately.
async fn related(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<havre_core::people::Related>, ApiError> {
    let event = agenda(&state)?.store().event(id).await?;
    let db = state.mail.manager()?.database();
    let mine: Vec<String> = db.accounts().await?.into_iter().map(|a| a.email.to_lowercase()).collect();
    let mut emails: Vec<String> = event.attendees.iter().map(|a| a.email.to_lowercase()).collect();
    if let Some(o) = &event.organizer {
        let address = o.rsplit_once('<').map_or(o.as_str(), |(_, a)| a.trim_end_matches('>'));
        emails.push(address.trim().to_lowercase());
    }
    emails.retain(|e| e.contains('@') && !mine.contains(e));
    emails.sort();
    emails.dedup();
    Ok(Json(db.people().related(&emails, 6).await?))
}
