//! Backups (step 9): Settings › Backups, and the `havre backup` /
//! `havre restore` / `havre key` commands. Behind the session, under `/api`.
//!
//! | Method | Path | What |
//! |--------|------|------|
//! | GET    | /backups | settings, folder, the backups, last result, next run, key fingerprint |
//! | PATCH  | /backups/settings `{enabled, time, keep}` | the nightly schedule |
//! | POST   | /backups | make one now |
//! | GET    | /backups/{name} | download one |
//! | DELETE | /backups/{name} | delete one |
//! | GET    | /agenda/calendars/{id}/export.ics | a calendar as an .ics file |

use std::path::Path;

use axum::{
    Json, Router,
    body::Body,
    extract::{Path as UrlPath, State},
    http::{StatusCode, header},
    response::Response,
    routing::get,
};
use havre_core::{
    Error,
    backup::{self, BackupFile, BackupSettings, BackupStatus, Backups},
    secrets::Secrets,
};

use crate::{AppState, api::ApiError};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/backups", get(status).post(run))
        .route("/backups/settings", axum::routing::patch(save_settings))
        .route("/backups/{name}", get(download).delete(remove))
        .route("/agenda/calendars/{id}/export.ics", get(export_calendar))
}

fn backups(state: &AppState) -> Result<&Backups, ApiError> {
    state.backups.as_ref().ok_or_else(|| ApiError::BadRequest("No backups in demo mode.".into()))
}

async fn status(State(state): State<AppState>) -> Result<Json<BackupStatus>, ApiError> {
    Ok(Json(backups(&state)?.status().await?))
}

async fn save_settings(
    State(state): State<AppState>,
    Json(s): Json<BackupSettings>,
) -> Result<Json<BackupSettings>, ApiError> {
    Ok(Json(backups(&state)?.save_settings(&s).await?))
}

async fn run(State(state): State<AppState>) -> Result<(StatusCode, Json<BackupFile>), ApiError> {
    Ok((StatusCode::CREATED, Json(backups(&state)?.run().await?)))
}

/// The file itself. Only names Havre writes are served (`is_backup_name`).
async fn download(
    State(state): State<AppState>,
    UrlPath(name): UrlPath<String>,
) -> Result<Response, ApiError> {
    let path = backups(&state)?.path_of(&name)?;
    let bytes = tokio::fs::read(&path).await.map_err(Error::from)?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "application/vnd.sqlite3")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .expect("valid response"))
}

async fn remove(
    State(state): State<AppState>,
    UrlPath(name): UrlPath<String>,
) -> Result<StatusCode, ApiError> {
    backups(&state)?.delete(&name)?;
    Ok(StatusCode::NO_CONTENT)
}

/// One calendar as `.ics` (e.g. Havre's own, which lives nowhere else).
async fn export_calendar(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Response, ApiError> {
    let agenda =
        state.agenda.as_ref().ok_or_else(|| ApiError::BadRequest("No Agenda in demo mode.".into()))?;
    let store = agenda.store();
    let calendar = store
        .calendars()
        .await?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| Error::NotFound(format!("calendar {id}")))?;
    let events: Vec<_> =
        store.events(i64::MIN / 4, i64::MAX / 4).await?.into_iter().filter(|e| e.calendar == id).collect();
    let file: String =
        calendar.name.chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-').collect();
    let file = if file.trim().is_empty() { "calendar".to_owned() } else { file.trim().replace(' ', "-") };
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "text/calendar; charset=utf-8")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{file}.ics\""))
        .body(Body::from(havre_core::agenda::ics::export(&calendar.name, &events)))
        .expect("valid response"))
}

// ---- Command line -----------------------------------------------------------------------

const HELP: &str = "\
havre                      start the server
havre backup               make a backup now (into HAVRE_BACKUP_DIR, default <data>/backups)
havre restore <file>       put a backup in place of the database (stop Havre first)
      [--key <64 hex>]     the recovery key, when restoring on a new server
      [--key-mismatch-ok]  restore without the right key: mail passwords to re-enter
havre key                  print the recovery key (keep it in your password manager)";

/// `havre <command>`: runs it and returns (the server doesn't start).
pub async fn command(args: &[String], data_dir: &Path, backup_dir: &Path) -> anyhow::Result<()> {
    match args[0].as_str() {
        "backup" => {
            let secrets = Secrets::load_or_create(data_dir)?;
            let db = havre_core::store::Database::open(&data_dir.join("havre.db"), secrets.clone()).await?;
            let file = Backups::new(db, backup_dir.to_owned(), secrets).run().await?;
            println!("Backup made: {} ({} KB)", backup_dir.join(&file.name).display(), file.size / 1024);
        }
        "restore" => {
            let Some(file) = args.get(1) else { anyhow::bail!("Which backup?\n\n{HELP}") };
            let key = args.iter().position(|a| a == "--key").and_then(|i| args.get(i + 1));
            let anyway = args.iter().any(|a| a == "--key-mismatch-ok");
            let path = Path::new(file);
            let info = if anyway {
                backup::restore_without_key(data_dir, path).await?
            } else {
                backup::restore(data_dir, path, key.map(String::as_str)).await?
            };
            let made = info.made_at();
            println!(
                "Restored the backup of {made} (Havre {}, {} account(s)) into {}.\n\
                 Any previous database was kept as havre.db.before-restore-…; start Havre as usual.",
                info.version,
                info.accounts,
                data_dir.display()
            );
            if anyway {
                println!("The key didn't match: enter your mail passwords again in Settings › Accounts.");
            }
        }
        "key" => {
            let secrets = Secrets::load_or_create(data_dir)?;
            println!("Recovery key (fingerprint {}):\n{}", secrets.fingerprint(), secrets.export_hex());
            println!("\nAnyone with this key and a backup can read your mail passwords: keep it private.");
        }
        "help" | "--help" | "-h" => println!("{HELP}"),
        other => anyhow::bail!("Unknown command “{other}”.\n\n{HELP}"),
    }
    Ok(())
}
