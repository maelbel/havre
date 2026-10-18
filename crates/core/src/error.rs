//! One error type for the whole crate.
//!
//! `thiserror` generates the boilerplate (`Display`, `std::error::Error`,
//! and the `From` conversions marked with `#[from]`), so `?` can turn an
//! I/O or IMAP error into our own [`Error`] automatically.

/// Everything that can go wrong inside `havre-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A setting is missing or invalid (for example `HAVRE_IMAP_HOST`).
    #[error("configuration error: {0}")]
    Config(String),

    /// Network problem: DNS, TCP connection, TLS handshake…
    #[error("network error: {0}")]
    Io(#[from] std::io::Error),

    /// The message, folder or attachment does not exist (any more).
    #[error("not found: {0}")]
    NotFound(String),

    /// The local database failed.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// The local database could not be migrated to the current version.
    #[error("database migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    /// Signing in with OAuth failed (expired, revoked, not approved…).
    #[error("{0}")]
    SignIn(String),

    /// The AI provider failed or refused (unreachable, wrong key, unknown
    /// model…). The message is shown as is.
    #[error("{0}")]
    Ai(String),

    /// The IMAP server answered with an error (wrong password, unknown folder…).
    #[error("IMAP error: {0}")]
    Imap(#[from] async_imap::error::Error),
}

/// Shortcut so we can write `Result<T>` instead of `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;
