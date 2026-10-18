//! What the app remembers between launches: which Havre server to open.
//!
//! A small JSON file in the system's config folder
//! (`%APPDATA%\app.havre.desktop\config.json` on Windows,
//! `~/Library/Application Support/app.havre.desktop/` on a Mac,
//! `~/.config/app.havre.desktop/` on Linux). Nothing secret is in it: the
//! sign-in lives in the web view's cookies, like in a browser.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Url};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    /// The server's address, e.g. `http://havre.home.lan:8080`.
    pub server: Option<String>,
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|dir| dir.join("config.json"))
}

/// The saved config, or an empty one (first launch, unreadable file).
pub fn load(app: &AppHandle) -> Config {
    path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, config: &Config) -> Result<(), String> {
    let path = path(app).ok_or("No folder to save the settings in.")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

/// What someone types → a server address: `havre.home.lan:8080` becomes
/// `http://havre.home.lan:8080/`. Only http and https; no path, query or
/// password (Havre lives at the root of its address).
pub fn normalize(input: &str) -> Result<Url, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Type your Havre's address.".into());
    }
    let with_scheme = if input.contains("://") { input.to_owned() } else { format!("http://{input}") };
    let mut url = Url::parse(&with_scheme).map_err(|_| format!("“{input}” isn't a web address."))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("The address must start with http:// or https://.".into());
    }
    if url.host_str().is_none() {
        return Err(format!("“{input}” has no server name."));
    }
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    let _ = url.set_username("");
    let _ = url.set_password(None);
    Ok(url)
}

/// `true` when `url` is on the same server (scheme, host and port).
pub fn same_origin(a: &Url, b: &Url) -> bool {
    a.origin() == b.origin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_what_people_type() {
        assert_eq!(normalize("havre.home.lan:8080").unwrap().as_str(), "http://havre.home.lan:8080/");
        assert_eq!(
            normalize(" https://mail.example.com/mail?x=1 ").unwrap().as_str(),
            "https://mail.example.com/"
        );
        assert_eq!(normalize("http://me:pw@localhost:9988").unwrap().as_str(), "http://localhost:9988/");
        assert!(normalize("").is_err());
        assert!(normalize("ftp://example.com").is_err());
        assert!(normalize("file:///etc/passwd").is_err());
    }

    #[test]
    fn compares_origins() {
        let a = Url::parse("http://localhost:8080/mail").unwrap();
        assert!(same_origin(&a, &Url::parse("http://localhost:8080/agenda?x").unwrap()));
        assert!(!same_origin(&a, &Url::parse("http://localhost:9988/").unwrap()));
        assert!(!same_origin(&a, &Url::parse("https://localhost:8080/").unwrap()));
    }
}
