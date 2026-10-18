//! `havre` — the server binary.
//!
//! One process does everything: the JSON API under `/api`, and the web UI
//! (the Nuxt build) for every other path. That is what keeps self-hosting
//! simple: one binary, one port.

mod agenda;
mod ai;
mod api;
mod auth;
mod backups;
mod files;
mod mail_source;
mod mcp;
mod notes;
mod organize;
mod platform;
mod tasks;

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use axum::Router;
use havre_core::{
    accounts::AccountManager, agenda::Agenda, ai::Ai, auth::Auth, backup::Backups, config::ImapConfig,
    secrets::Secrets, store::Database, tasks::Tasks,
};
use mail_source::MailSource;
use tokio::sync::watch;
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::EnvFilter;

/// Data shared by every request handler.
/// Axum clones it for each request, so keep it cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub mail: MailSource,
    /// The login for Havre (`None` in demo mode: nothing to protect).
    pub auth: Option<Auth>,
    /// The Agenda (`None` in demo mode).
    pub agenda: Option<Arc<Agenda>>,
    /// Tasks (`None` in demo mode).
    pub tasks: Option<Arc<Tasks>>,
    /// AI, off until turned on in Settings (`None` in demo mode).
    pub ai: Option<Arc<Ai>>,
    /// Nightly backups (`None` in demo mode).
    pub backups: Option<Backups>,
    /// Slows down password guessing.
    pub throttle: auth::Throttle,
    /// Fires when the server is stopping (long-lived requests end early).
    pub shutdown: Shutdown,
}

/// A cloneable "the server is stopping" signal.
#[derive(Clone)]
pub struct Shutdown(watch::Receiver<bool>);

impl Shutdown {
    /// Resolves once shutdown has started.
    pub async fn wait_for_owned(mut self) {
        let _ = self.0.wait_for(|stopping| *stopping).await;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load `.env` if there is one (ignored in production, where real
    // environment variables are used).
    dotenvy::dotenv().ok();

    // Logs. Control the level with RUST_LOG, e.g. `RUST_LOG=debug`.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "havre=info,tower_http=info".into()),
        )
        .init();

    // Accounts and the local copy of each mailbox live in
    // HAVRE_DATA_DIR/havre.db; passwords are encrypted with the key in
    // HAVRE_SECRET_KEY or HAVRE_DATA_DIR/secret.key.
    let data_dir = PathBuf::from(std::env::var("HAVRE_DATA_DIR").unwrap_or_else(|_| "data".into()));
    let backup_dir =
        std::env::var("HAVRE_BACKUP_DIR").map(PathBuf::from).unwrap_or_else(|_| data_dir.join("backups"));

    // `havre backup`, `havre restore <file>`, `havre key`: do that and stop.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        // A plain message for people at a terminal, not a Rust error dump.
        if let Err(err) = backups::command(&args, &data_dir, &backup_dir).await {
            eprintln!("{err}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // `HAVRE_DEMO=1` serves sample mail; otherwise use the IMAP account.
    let demo = matches!(std::env::var("HAVRE_DEMO").as_deref(), Ok("1" | "true"));
    let (mail, login, agenda, tasks, ai, backups) = if demo {
        tracing::info!("demo mode: serving sample mail");
        (MailSource::demo(), None, None, None, None, None)
    } else {
        let secrets = Secrets::load_or_create(&data_dir)?;
        let db = Database::open(&data_dir.join("havre.db"), secrets.clone()).await?;
        // Nightly backups, checked every minute.
        let backups = Backups::new(db.clone(), backup_dir.clone(), secrets);
        tokio::spawn(backups.clone().scheduler());
        tracing::info!("database: {}", data_dir.join("havre.db").display());
        let login = db.auth();
        // Forgot the password? Start once with HAVRE_RESET_PASSWORD=1.
        if matches!(std::env::var("HAVRE_RESET_PASSWORD").as_deref(), Ok("1" | "true")) {
            login.reset().await?;
            tracing::warn!(
                "HAVRE_RESET_PASSWORD: password forgotten, everyone signed out. Choose a new one in the web \
                 UI, then remove HAVRE_RESET_PASSWORD from .env"
            );
        }
        if !login.is_set_up().await? {
            tracing::info!("no password yet: open the web UI to choose one");
        }
        let manager = AccountManager::start(db).await?;
        // Older setups configured one account in .env: import it once.
        if let Ok(config) = ImapConfig::from_env() {
            manager.import_from_env(config).await?;
        }
        let count = manager.database().accounts().await?.len();
        if count == 0 {
            tracing::info!("no mail account yet: add one in the web UI (Settings)");
        } else {
            tracing::info!("{count} mail account(s)");
        }
        // Calendars: synced in the background, changes announced on the
        // same channel as mail.
        let agenda = Agenda::start(manager.database().clone(), manager.sender());
        // Tasks: the same, plus reminders.
        let tasks = Tasks::start(manager.database().clone(), manager.sender());
        // AI: does nothing until turned on in Settings › AI.
        let ai = Ai::start(Arc::clone(&manager), Arc::clone(&agenda));
        (MailSource::Accounts(manager), Some(login), Some(agenda), Some(tasks), Some(ai), Some(backups))
    };

    let web_dir =
        PathBuf::from(std::env::var("HAVRE_WEB_DIR").unwrap_or_else(|_| "web/.output/public".into()));
    // Single-page app: unknown paths (like /mail) return index.html and the
    // front-end router takes over.
    let web = ServeDir::new(&web_dir).fallback(ServeFile::new(web_dir.join("index.html")));

    let (stopping_tx, stopping_rx) = watch::channel(false);
    let state = AppState {
        mail,
        auth: login,
        agenda,
        tasks,
        ai,
        backups,
        throttle: auth::Throttle::default(),
        shutdown: Shutdown(stopping_rx),
    };
    let app = Router::new()
        .nest("/api", api::router(state.clone()))
        .merge(mcp::endpoint())
        .fallback_service(web)
        .with_state(state.clone())
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = std::env::var("HAVRE_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into()).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // Claude Code (as the AI provider) calls back here for Havre's tools.
    if let Some(ai) = &state.ai {
        let local = match addr.ip() {
            ip if ip.is_unspecified() && ip.is_ipv4() => SocketAddr::from(([127, 0, 0, 1], addr.port())),
            ip if ip.is_unspecified() => SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, addr.port())),
            _ => addr,
        };
        ai.set_self_url(format!("http://{local}"));
    }
    tracing::info!("Havre is running on http://{addr}");

    // `with_connect_info`: handlers can see the client's address (to
    // slow down password guessing per address).
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            tracing::info!("shutting down");
            let _ = stopping_tx.send(true);
        })
        .await?;
    Ok(())
}

/// Ctrl+C, or SIGTERM (what `docker stop` sends).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
