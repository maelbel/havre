//! # havre-core
//!
//! The heart of Havre. Everything here is plain Rust: no web framework,
//! no UI. The server (`havre-server`), the command line (`havre-cli`) and,
//! later, the desktop app (Tauri) all call into this crate.
//!
//! Modules:
//! - [`accounts`]: mail accounts, one sync engine each.
//! - [`ai`]: optional AI (Ollama, OpenAI-compatible, Anthropic): summaries, drafts, brief, search, assistant.
//! - [`apps`]: the apps Havre ships (manifests) and which are in the dock.
//! - [`auth`]: the login for Havre itself (password, sessions).
//! - [`compose`]: writing mail: drafts, replies, forwards, sending (SMTP).
//! - [`config`]: reading settings from environment variables.
//! - [`error`]: the error type shared by the whole crate.
//! - [`files`]: every attachment in your mail, in one list (the Files app).
//! - [`invitations`]: meeting invitations in Mail, answered in the Agenda.
//! - [`mail`]: talking to mail servers (IMAP for now).
//! - [`people`]: everyone you write to or meet, from Mail and the Agenda.
//! - [`notes`]: Markdown notes attached to a meeting, people or an email.
//! - [`mcp`]: Havre's tools for other AI apps (Claude Code…) over MCP.
//! - [`oauth`]: OAuth2 with Google and Microsoft (tokens instead of passwords).
//! - [`secrets`]: encrypting stored passwords.
//! - [`store`]: the database: accounts, and a local copy of each mailbox.
//! - [`sync`]: keeps the store in sync with the server, in the background.
//! - [`tasks`]: to-dos from Microsoft To Do, Google Tasks, CalDAV or Havre, linked to mail and people.

/// SQL put together at runtime, as sqlx 0.9 asks to be marked: a query built
/// with `format!` only compiles when the code says it was checked.
///
/// Every use in Havre pastes in **only text written in the code** — a list of
/// columns (`ACCOUNT_COLUMNS`…), a table name from a fixed list, `?` marks —
/// never something a user or a server sent: those always go through `.bind()`.
/// Keep it that way when adding one.
///
/// `sql!("SELECT {COLUMNS} FROM notes WHERE id = ?")` is
/// `sqlx::AssertSqlSafe(format!(…))`.
macro_rules! sql {
    ($($arg:tt)*) => {
        sqlx::AssertSqlSafe(format!($($arg)*))
    };
}

pub mod accounts;
pub mod agenda;
pub mod ai;
pub mod apps;
pub mod auth;
pub mod backup;
pub mod compose;
pub mod config;
pub mod error;
pub mod files;
pub mod invitations;
pub mod mail;
pub mod mcp;
pub mod notes;
pub mod oauth;
pub mod people;
pub mod push;
pub mod secrets;
pub mod store;
pub mod sync;
pub mod tasks;
pub mod users;

pub use error::{Error, Result};
