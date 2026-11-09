//! What the apps share, under `/api` (needs a session, like the rest).
//!
//! | Method | Path | What |
//! |--------|------|------|
//! | GET    | /apps | every app (manifest + in the dock? + position) |
//! | PATCH  | /apps/{id} `{enabled}` | put an app in the dock, or take it out |
//! | PUT    | /apps/order `{ids}` | the dock's order |
//! | GET    | /people?q=&sort=&filter=&company=&limit= | people from Mail and the Agenda (sort: recent, frequent, name; filter: everyone, favorites, waiting, automated, all) |
//! | GET    | /people/companies | the companies of the people you know, the biggest first |
//! | GET    | /people/today?date= | birthdays this week, favourites to keep in touch with (`date`: the browser's day) |
//! | GET    | /people/{email} | one person, by any of their addresses: details, keeping in touch, mail, meetings |
//! | PATCH  | /people/{email} `{name?, company?, role?, phones?, birthday?, note?, favorite?, kind?}` | your details about them |
//! | POST   | /people/{email}/merge `{email}` | the same person: one page for both addresses |
//! | DELETE | /people/{email}/addresses/{other} | that address is someone else again |
//! | GET    | /mail/unread?limit= | unread mail in every inbox (the Today page) |
//! | GET    | /system | version, memory, data size, counts (the App library) |
//! | GET    | /mail/all?limit=&offset=&q= | every inbox together, newest first ("All inboxes") |
//! | GET    | /mail/search?q=&limit= | sender or subject, every account and folder (the Ctrl+K palette) |

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, patch, post, put},
};
use havre_core::{
    ai::index::Hit,
    apps::{self, App},
    mail::MessagePage,
    people::{Company, ListQuery, PeopleToday, Person, PersonDetail, PersonUpdate},
    store::UnreadInbox,
};
use serde::Deserialize;

use crate::{AppState, api::ApiError, mail_source::MailSource};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/apps", get(list_apps))
        .route("/apps/order", put(reorder_apps))
        .route("/apps/{id}", patch(update_app))
        .route("/system", get(system))
        .route("/people", get(people))
        .route("/people/companies", get(companies))
        .route("/people/today", get(people_today))
        .route("/people/{email}", get(person).patch(update_person))
        .route("/people/{email}/merge", post(merge_person))
        .route("/people/{email}/addresses/{other}", delete(remove_address))
        .route("/mail/unread", get(unread))
        .route("/mail/search", get(search_all))
        .route("/mail/all", get(all_inboxes))
}

async fn list_apps(State(state): State<AppState>) -> Result<Json<Vec<App>>, ApiError> {
    Ok(Json(match &state.mail {
        // No database in demo mode: the default dock.
        MailSource::Demo(_) => apps::apply(&[]),
        MailSource::Accounts(m) => m.database().apps().list().await?,
    }))
}

#[derive(Deserialize)]
struct AppUpdate {
    enabled: bool,
}

async fn update_app(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(update): Json<AppUpdate>,
) -> Result<Json<Vec<App>>, ApiError> {
    let db = state.mail.manager()?.database();
    let apps = db.apps().set_enabled(&id, update.enabled).await?;
    // The AI app added or removed: its background work notices at once.
    if id == "ai"
        && let Some(ai) = &state.ai
    {
        ai.wake();
    }
    Ok(Json(apps))
}

#[derive(Deserialize)]
struct Order {
    ids: Vec<String>,
}

async fn reorder_apps(
    State(state): State<AppState>,
    Json(order): Json<Order>,
) -> Result<Json<Vec<App>>, ApiError> {
    let db = state.mail.manager()?.database();
    Ok(Json(db.apps().reorder(&order.ids).await?))
}

#[derive(Deserialize)]
struct Search {
    #[serde(default)]
    q: String,
    limit: Option<u32>,
}

async fn people(
    State(state): State<AppState>,
    Query(mut q): Query<ListQuery>,
) -> Result<Json<Vec<Person>>, ApiError> {
    q.limit = Some(q.limit.unwrap_or(200).min(500));
    Ok(Json(match &state.mail {
        MailSource::Demo(_) => Vec::new(),
        MailSource::Accounts(m) => m.database().people().list(&q).await?,
    }))
}

async fn companies(State(state): State<AppState>) -> Result<Json<Vec<Company>>, ApiError> {
    Ok(Json(match &state.mail {
        MailSource::Demo(_) => Vec::new(),
        MailSource::Accounts(m) => m.database().people().companies().await?,
    }))
}

#[derive(Deserialize)]
struct Day {
    /// The browser's date (`YYYY-MM-DD`): "today" where you are.
    date: String,
}

async fn people_today(
    State(state): State<AppState>,
    Query(d): Query<Day>,
) -> Result<Json<PeopleToday>, ApiError> {
    Ok(Json(match &state.mail {
        MailSource::Demo(_) => PeopleToday { birthdays: Vec::new(), keep_in_touch: Vec::new() },
        MailSource::Accounts(m) => m.database().people().today_on(&d.date).await?,
    }))
}

async fn person(
    State(state): State<AppState>,
    Path(email): Path<String>,
) -> Result<Json<PersonDetail>, ApiError> {
    let db = state.mail.manager()?.database();
    Ok(Json(db.people().person(&email).await?))
}

async fn update_person(
    State(state): State<AppState>,
    Path(email): Path<String>,
    Json(update): Json<PersonUpdate>,
) -> Result<Json<PersonDetail>, ApiError> {
    let db = state.mail.manager()?.database();
    Ok(Json(db.people().update(&email, &update).await?))
}

#[derive(Deserialize)]
struct Other {
    email: String,
}

async fn merge_person(
    State(state): State<AppState>,
    Path(email): Path<String>,
    Json(other): Json<Other>,
) -> Result<Json<PersonDetail>, ApiError> {
    let db = state.mail.manager()?.database();
    Ok(Json(db.people().merge(&email, &other.email).await?))
}

async fn remove_address(
    State(state): State<AppState>,
    Path((email, other)): Path<(String, String)>,
) -> Result<Json<PersonDetail>, ApiError> {
    let db = state.mail.manager()?.database();
    Ok(Json(db.people().remove_address(&email, &other).await?))
}

#[derive(Deserialize)]
struct Limit {
    limit: Option<u32>,
}

async fn unread(
    State(state): State<AppState>,
    Query(l): Query<Limit>,
) -> Result<Json<UnreadInbox>, ApiError> {
    Ok(Json(match &state.mail {
        MailSource::Demo(_) => UnreadInbox { total: 0, messages: Vec::new() },
        MailSource::Accounts(m) => m.database().unread_inbox(l.limit.unwrap_or(8).min(50)).await?,
    }))
}

async fn search_all(
    State(state): State<AppState>,
    Query(s): Query<Search>,
) -> Result<Json<Vec<Hit>>, ApiError> {
    Ok(Json(match &state.mail {
        MailSource::Demo(_) => Vec::new(),
        MailSource::Accounts(m) => {
            m.database().search_all(&s.q, i64::from(s.limit.unwrap_or(8).min(50))).await?
        }
    }))
}

#[derive(Deserialize)]
struct AllQuery {
    limit: Option<u32>,
    #[serde(default)]
    offset: u32,
    q: Option<String>,
}

async fn all_inboxes(
    State(state): State<AppState>,
    Query(a): Query<AllQuery>,
) -> Result<Json<MessagePage>, ApiError> {
    Ok(Json(match &state.mail {
        MailSource::Demo(_) => MessagePage { messages: Vec::new(), has_more: false },
        MailSource::Accounts(m) => {
            let q = a.q.as_deref().filter(|q| !q.trim().is_empty());
            m.database().all_inboxes(a.limit.unwrap_or(50).clamp(1, 200), a.offset, q).await?
        }
    }))
}

/// The App library's status card. Memory is known on Linux (and Docker).
async fn system(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let memory = std::fs::read_to_string("/proc/self/status").ok().and_then(|s| {
        s.lines()
            .find(|l| l.starts_with("VmRSS:"))
            .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            .map(|kb| kb * 1024)
    });
    let (data, accounts, calendars, people) = match &state.mail {
        MailSource::Demo(_) => (None, 1, 0, 0),
        MailSource::Accounts(m) => {
            let db = m.database();
            let calendars = match &state.agenda {
                Some(a) => a.store().calendars().await?.len(),
                None => 0,
            };
            (
                Some(db.size().await?),
                db.accounts().await?.len(),
                calendars,
                db.people().search("", 100_000).await?.len(),
            )
        }
    };
    // Tasks still to do (the Tasks card).
    let tasks = match &state.tasks {
        Some(t) => t.store().lists().await?.iter().map(|l| l.open).sum::<i64>(),
        None => 0,
    };
    // How many notes (the Notes card).
    let notes = match state.mail.manager() {
        Ok(m) => m.database().notes().count().await?,
        Err(_) => 0,
    };
    // Files found in your mail (the Files card).
    let files = match state.mail.manager() {
        Ok(m) => m.database().files().count().await?,
        Err(_) => 0,
    };
    // The AI app's conversations (its page in Settings).
    let conversations = match &state.ai {
        Some(ai) => ai.conversation_count().await?,
        None => 0,
    };
    // The status card's "Backup" line.
    let backup = match &state.backups {
        Some(b) => {
            let s = b.status().await?;
            serde_json::json!({
                "enabled": s.settings.enabled,
                "last_at": s.last_at,
                "last_error": s.last_error,
                "next_at": s.next_at,
                "size": s.backups.first().map(|f| f.size),
                // Off-site copies failing for a day turn the line orange.
                "offsite_failing": s.remote.failing_for_a_day,
                "offsite_error": s.remote.last_error,
            })
        }
        None => serde_json::Value::Null,
    };
    Ok(Json(serde_json::json!({
        "backup": backup,
        "version": env!("CARGO_PKG_VERSION"),
        "memory": memory,
        "data": data,
        "accounts": accounts,
        "calendars": calendars,
        "people": people,
        "tasks": tasks,
        "notes": notes,
        "files": files,
        "conversations": conversations,
    })))
}
