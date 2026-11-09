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
//! | GET · PATCH | /push/settings `{new_mail?, mail_details?, reminders?}` | what to send |
//! | POST   | /push/test `{endpoint}` | a test notification to this browser |
//! | PUT    | /push/seen `{endpoint}` | Havre is open and in front here: send nothing for a minute |
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
use havre_core::push::{Device, NewSubscription, Push, PushSettings, PushSettingsUpdate};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppState, api::ApiError};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/push/key", get(key))
        .route("/push/subscribe", post(subscribe).delete(unsubscribe))
        .route("/push/devices", get(devices))
        .route("/push/devices/{id}", delete(remove_device))
        .route("/push/settings", get(settings).patch(update_settings))
        .route("/push/test", post(test))
        .route("/push/seen", put(seen))
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
    Json(s): Json<NewSubscription>,
) -> Result<StatusCode, ApiError> {
    push(&state)?.subscribe(&s).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unsubscribe(
    State(state): State<AppState>,
    Json(d): Json<ThisDevice>,
) -> Result<StatusCode, ApiError> {
    push(&state)?.unsubscribe(&d.endpoint).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn devices(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Vec<Device>>, ApiError> {
    let this = headers.get("x-push-endpoint").and_then(|v| v.to_str().ok());
    Ok(Json(push(&state)?.devices(this).await?))
}

async fn remove_device(State(state): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    push(&state)?.remove_device(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn settings(State(state): State<AppState>) -> Result<Json<PushSettings>, ApiError> {
    Ok(Json(push(&state)?.settings().await?))
}

async fn update_settings(
    State(state): State<AppState>,
    Json(u): Json<PushSettingsUpdate>,
) -> Result<Json<PushSettings>, ApiError> {
    Ok(Json(push(&state)?.update_settings(&u).await?))
}

async fn test(State(state): State<AppState>, Json(d): Json<ThisDevice>) -> Result<StatusCode, ApiError> {
    push(&state)?.test(&d.endpoint).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn seen(State(state): State<AppState>, Json(d): Json<ThisDevice>) -> Result<StatusCode, ApiError> {
    push(&state)?.seen(&d.endpoint);
    Ok(StatusCode::NO_CONTENT)
}
