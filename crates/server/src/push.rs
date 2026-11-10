//! Push notifications (step 20): Settings › Notifications and the service
//! worker's subscription. Behind the session, under `/api`.
//!
//! | Method | Path | What |
//! |--------|------|------|
//! | GET    | /push/key | Havre's public key, for the browser's `subscribe()` `{key}` |
//! | POST   | /push/subscribe `{endpoint, keys: {p256dh, auth}, label}` | this browser gets notifications |
//! | DELETE | /push/subscribe `{endpoint}` | this browser stops getting them |
//! | GET    | /push/devices (header `X-Push-Endpoint`: this browser's, if any) | the devices that get them |
//! | DELETE | /push/devices/{id} | stop sending to a device |
//! | GET · PATCH | /push/settings `{new_mail?, mail_details?, reminders?, event_reminders?, invitations?}` | what to send |
//! | POST   | /push/test `{endpoint}` | a test notification to this browser |
//! | PUT    | /push/seen `{endpoint}` | Havre is open and in front here: send nothing for a minute |
//! | POST   | /push/action `{action, target}` | a notification's button (the service worker calls it) |
//!
//! A browser's endpoint is sent in a body or a header, never in the address:
//! it's a secret of sorts (whoever has it and the keys can reach the phone),
//! and addresses end up in logs.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post, put},
};
use havre_core::{
    invitations,
    push::{Action, Device, NewSubscription, Push, PushSettings, PushSettingsUpdate, Target},
    sync::Action as MailAction,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppState, MailSource, api::ApiError, auth::CurrentUser};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/push/key", get(key))
        .route("/push/subscribe", post(subscribe).delete(unsubscribe))
        .route("/push/devices", get(devices))
        .route("/push/devices/{id}", delete(remove_device))
        .route("/push/settings", get(settings).patch(update_settings))
        .route("/push/test", post(test))
        .route("/push/seen", put(seen))
        .route("/push/action", post(action))
}

fn push(state: &AppState) -> Result<&Arc<Push>, ApiError> {
    state
        .push
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("Notifications aren't available in demo mode.".into()))
}

#[derive(Deserialize)]
struct ThisDevice {
    endpoint: String,
}

async fn key(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "key": push(&state)?.public_key().await? })))
}

async fn subscribe(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(s): Json<NewSubscription>,
) -> Result<StatusCode, ApiError> {
    push(&state)?.subscribe(user, &s).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unsubscribe(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(d): Json<ThisDevice>,
) -> Result<StatusCode, ApiError> {
    push(&state)?.unsubscribe(user, &d.endpoint).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn devices(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
) -> Result<Json<Vec<Device>>, ApiError> {
    let this = headers.get("x-push-endpoint").and_then(|v| v.to_str().ok());
    Ok(Json(push(&state)?.devices(user, this).await?))
}

async fn remove_device(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    push(&state)?.remove_device(user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn settings(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<PushSettings>, ApiError> {
    Ok(Json(push(&state)?.settings(user).await?))
}

async fn update_settings(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(u): Json<PushSettingsUpdate>,
) -> Result<Json<PushSettings>, ApiError> {
    Ok(Json(push(&state)?.update_settings(user, &u).await?))
}

async fn test(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(d): Json<ThisDevice>,
) -> Result<StatusCode, ApiError> {
    push(&state)?.test(user, &d.endpoint).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn seen(State(state): State<AppState>, Json(d): Json<ThisDevice>) -> Result<StatusCode, ApiError> {
    push(&state)?.seen(&d.endpoint);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ActionBody {
    /// The button's name (`archive`, `snooze-10`…).
    action: String,
    /// What it acts on: the notification's `data`.
    target: Target,
}

/// A notification's button. The service worker sends what the notification
/// carried; the core refuses unknown buttons and buttons on the wrong kind of
/// target, then the work is done by the same functions as the app's buttons
/// (which fail on an email, task or invitation that isn't there).
async fn action(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Json(b): Json<ActionBody>,
) -> Result<StatusCode, ApiError> {
    let push = push(&state)?;
    let action = Action::parse(&b.action)?;
    action.check(&b.target)?;
    // Mail buttons: only on the emails still there.
    let target = push.still_there(user, &b.target).await?;
    // `match` on the pair: each button with the target it goes with.
    match (action, &target) {
        (Action::Archive, Target::Mail { account, folder, uid }) => {
            let engine = state.mail.manager()?.engine(user, *account).await?;
            engine.apply_undoable(MailAction::Archive, folder, &[*uid], None).await?;
        }
        (Action::Read, Target::Mail { account, folder, uid }) => {
            state.mail.manager()?.engine(user, *account).await?.set_seen_many(folder, &[*uid], true).await?;
        }
        (Action::Read, Target::Inbox { account, folder, uids }) => {
            state.mail.manager()?.engine(user, *account).await?.set_seen_many(folder, uids, true).await?;
        }
        (Action::Done, Target::Task { task }) => {
            let tasks =
                state.tasks.as_ref().ok_or_else(|| ApiError::BadRequest("Tasks aren't available.".into()))?;
            tasks.set_done(user, *task, true).await?;
        }
        (Action::Snooze(minutes), target) => push.snooze(user, target, minutes).await?,
        (Action::Accept | Action::Decline, Target::Invitation { account, folder, uid, index }) => {
            let MailSource::Accounts(accounts) = &state.mail else {
                return Err(ApiError::BadRequest("Invitations aren't available in demo mode.".into()));
            };
            let agenda = state
                .agenda
                .as_ref()
                .ok_or_else(|| ApiError::BadRequest("The Agenda isn't available.".into()))?;
            let answer = if action == Action::Accept { "accepted" } else { "declined" };
            invitations::respond(accounts, &agenda.for_user(user), *account, folder, *uid, *index, answer)
                .await?;
        }
        // `check` above already refused the other pairs.
        _ => return Err(ApiError::BadRequest("This button doesn't go with this notification.".into())),
    }
    Ok(StatusCode::NO_CONTENT)
}
