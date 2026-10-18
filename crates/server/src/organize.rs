//! Organizing mail (step 8): stars, labels, snooze, several messages at
//! once, undo. Under `/api`, behind the session like the rest.
//!
//! | Method | Path | What |
//! |--------|------|------|
//! | GET    | /mail/organize | labels (with counts), how many starred and snoozed |
//! | GET    | /mail/view?view=starred\|snoozed\|label&label=&limit=&offset=&q= | a list across accounts |
//! | POST   | /mail/flags `{account, folder, uids, seen?, flagged?}` | read/unread, star, on several messages |
//! | POST   | /mail/labels `{account, folder, uids, label, on}` | add or remove a label |
//! | POST   | /mail/snooze `{account, folder, uids, until}` | snooze until a time (`null`: wake up now) |
//! | POST   | /mail/undo `{account, from, to, message_ids}` | move messages back after archive/move/delete |
//! | POST   | /labels `{name, color}` | new label |
//! | PATCH  | /labels/{id} `{name?, color?}` | rename, recolor |
//! | DELETE | /labels/{id} | delete a label (messages stay) |

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch, post},
};
use havre_core::{
    Error,
    mail::MessagePage,
    store::{Label, View},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{AppState, api::ApiError, mail_source::MailSource};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/mail/organize", get(organize))
        .route("/mail/view", get(view))
        .route("/mail/flags", post(flags))
        .route("/mail/labels", post(label_messages))
        .route("/mail/snooze", post(snooze))
        .route("/mail/undo", post(undo))
        .route("/labels", post(create_label))
        .route("/labels/{id}", patch(update_label).delete(delete_label))
}

/// Organizing needs the database: not in demo mode.
fn manager(state: &AppState) -> Result<&std::sync::Arc<havre_core::accounts::AccountManager>, ApiError> {
    match &state.mail {
        MailSource::Demo(_) => {
            Err(Error::Config("Stars, labels and snooze aren't available in demo mode.".into()).into())
        }
        MailSource::Accounts(m) => Ok(m),
    }
}

async fn organize(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(match &state.mail {
        MailSource::Demo(_) => json!({ "labels": [], "starred": 0, "snoozed": 0 }),
        MailSource::Accounts(m) => {
            let db = m.database();
            let counts = db.organize_counts().await?;
            json!({ "labels": db.labels().await?, "starred": counts.starred, "snoozed": counts.snoozed })
        }
    }))
}

#[derive(Deserialize)]
struct ViewQuery {
    view: String,
    label: Option<i64>,
    limit: Option<u32>,
    #[serde(default)]
    offset: u32,
    q: Option<String>,
}

async fn view(
    State(state): State<AppState>,
    Query(v): Query<ViewQuery>,
) -> Result<Json<MessagePage>, ApiError> {
    let which = match (v.view.as_str(), v.label) {
        ("inboxes", _) => View::Inboxes,
        ("starred", _) => View::Starred,
        ("snoozed", _) => View::Snoozed,
        ("label", Some(id)) => View::Label(id),
        _ => {
            return Err(ApiError::BadRequest(
                "view: inboxes, starred, snoozed or label (with label=)".into(),
            ));
        }
    };
    Ok(Json(match &state.mail {
        MailSource::Demo(_) => MessagePage { messages: Vec::new(), has_more: false },
        MailSource::Accounts(m) => {
            let q = v.q.as_deref().filter(|q| !q.trim().is_empty());
            m.database().across(which, v.limit.unwrap_or(50).clamp(1, 200), v.offset, q).await?
        }
    }))
}

/// Which messages: one account and folder, one or more UIDs.
#[derive(Deserialize)]
struct Selection {
    account: i64,
    folder: String,
    uids: Vec<u32>,
}

impl Selection {
    fn check(&self) -> Result<(), ApiError> {
        if self.uids.is_empty() {
            return Err(ApiError::BadRequest("no messages selected".into()));
        }
        if self.uids.len() > 1000 {
            return Err(ApiError::BadRequest("1000 messages at most at once".into()));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct FlagsBody {
    #[serde(flatten)]
    selection: Selection,
    seen: Option<bool>,
    flagged: Option<bool>,
}

async fn flags(State(state): State<AppState>, Json(b): Json<FlagsBody>) -> Result<StatusCode, ApiError> {
    b.selection.check()?;
    let s = &b.selection;
    match &state.mail {
        // Demo: read/unread only, one at a time.
        MailSource::Demo(_) => {
            if let Some(seen) = b.seen {
                for uid in &s.uids {
                    state.mail.set_seen(s.account, &s.folder, *uid, seen).await?;
                }
            }
        }
        MailSource::Accounts(m) => {
            let engine = m.engine(s.account).await?;
            if let Some(seen) = b.seen {
                engine.set_seen_many(&s.folder, &s.uids, seen).await?;
            }
            if let Some(flagged) = b.flagged {
                engine.set_flagged(&s.folder, &s.uids, flagged).await?;
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct LabelBody {
    #[serde(flatten)]
    selection: Selection,
    label: i64,
    on: bool,
}

async fn label_messages(
    State(state): State<AppState>,
    Json(b): Json<LabelBody>,
) -> Result<StatusCode, ApiError> {
    b.selection.check()?;
    let s = &b.selection;
    manager(&state)?.engine(s.account).await?.set_label(&s.folder, &s.uids, b.label, b.on).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SnoozeBody {
    #[serde(flatten)]
    selection: Selection,
    /// Unix seconds; `null` wakes them up now.
    until: Option<i64>,
}

async fn snooze(State(state): State<AppState>, Json(b): Json<SnoozeBody>) -> Result<StatusCode, ApiError> {
    b.selection.check()?;
    let s = &b.selection;
    manager(&state)?.engine(s.account).await?.snooze(&s.folder, &s.uids, b.until).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct UndoBody {
    account: i64,
    /// Where the messages are now.
    from: String,
    /// Where they were.
    to: String,
    message_ids: Vec<String>,
}

async fn undo(State(state): State<AppState>, Json(b): Json<UndoBody>) -> Result<Json<Value>, ApiError> {
    if b.message_ids.is_empty() {
        return Err(ApiError::BadRequest("Nothing to undo.".into()));
    }
    let moved = manager(&state)?.engine(b.account).await?.undo_move(&b.from, &b.to, &b.message_ids).await?;
    Ok(Json(json!({ "moved": moved })))
}

#[derive(Deserialize)]
struct NewLabel {
    name: String,
    #[serde(default)]
    color: String,
}

async fn create_label(
    State(state): State<AppState>,
    Json(b): Json<NewLabel>,
) -> Result<(StatusCode, Json<Label>), ApiError> {
    Ok((StatusCode::CREATED, Json(manager(&state)?.database().create_label(&b.name, &b.color).await?)))
}

#[derive(Deserialize)]
struct LabelUpdate {
    name: Option<String>,
    color: Option<String>,
}

async fn update_label(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(b): Json<LabelUpdate>,
) -> Result<Json<Label>, ApiError> {
    Ok(Json(manager(&state)?.database().update_label(id, b.name.as_deref(), b.color.as_deref()).await?))
}

async fn delete_label(State(state): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    manager(&state)?.database().delete_label(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
