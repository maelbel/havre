//! Settings, read from environment variables.
//!
//! In development, put them in a `.env` file at the project root
//! (see `.env.example`); the server and CLI load it on startup.

use std::sync::Arc;

use crate::{Error, Result, oauth::OAuthSession};

/// How we prove who we are to the IMAP server.
#[derive(Clone)]
pub enum ImapAuth {
    /// Classic `LOGIN user password`.
    Password(String),
    /// OAuth2 (Google, Microsoft): a fresh access token for each connection.
    OAuth(Arc<OAuthSession>),
    /// A single access token (used once, right after signing in).
    Bearer(String),
}

/// How to reach one IMAP mailbox.
///
/// `Clone` lets the server keep a copy in its shared state.
/// We implement `Debug` by hand so the password never ends up in logs.
#[derive(Clone)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: ImapAuth,
    /// Extra certificate authority (PEM file) to trust, for self-hosted
    /// servers with their own certificates. Usually `None`.
    pub ca_file: Option<String>,
}

impl std::fmt::Debug for ImapConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field(
                "auth",
                &match self.auth {
                    ImapAuth::Password(_) => "password ********",
                    ImapAuth::OAuth(_) | ImapAuth::Bearer(_) => "oauth ********",
                },
            )
            .field("ca_file", &self.ca_file)
            .finish()
    }
}

impl ImapConfig {
    /// Builds the config from `HAVRE_IMAP_HOST`, `HAVRE_IMAP_PORT` (default 993),
    /// `HAVRE_IMAP_USER`, `HAVRE_IMAP_PASSWORD` and optionally `HAVRE_IMAP_CA_FILE`.
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            host: required("HAVRE_IMAP_HOST")?,
            port: match std::env::var("HAVRE_IMAP_PORT") {
                Ok(p) => p
                    .parse()
                    .map_err(|_| Error::Config(format!("HAVRE_IMAP_PORT is not a valid port: {p}")))?,
                Err(_) => 993,
            },
            username: required("HAVRE_IMAP_USER")?,
            auth: ImapAuth::Password(required("HAVRE_IMAP_PASSWORD")?),
            ca_file: std::env::var("HAVRE_IMAP_CA_FILE").ok().filter(|v| !v.trim().is_empty()),
        })
    }
}

/// Reads an environment variable, or explains which one is missing.
fn required(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| Error::Config(format!("{name} is not set (see .env.example)")))
}
