//! Apps: what Havre is made of.
//!
//! Each app (Mail, Agenda, People…) describes itself with a **manifest**:
//! a name, an icon, the page it lives on, and what it *provides* to the
//! others (for example Agenda provides `calendar.invitations`, which Mail
//! uses to answer a meeting invitation). The App library page lists the
//! manifests; you choose which apps sit in the dock, and in which order.
//!
//! For now the apps are built in (their code ships with Havre). Manifests
//! are the first step toward apps you can add later.

use serde::Serialize;
use sqlx::SqlitePool;

use crate::{Error, Result};

/// How an app describes itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Manifest {
    /// Stable id (`mail`, `agenda`…).
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// A name from the UI's icon set.
    pub icon: &'static str,
    /// The page that opens it.
    pub route: &'static str,
    /// What it offers the other apps.
    pub provides: &'static [&'static str],
    /// What it uses from the others (works without, just with less).
    pub uses: &'static [&'static str],
    /// In the dock from the start (`false`: you add it from the library).
    #[serde(skip)]
    pub default_enabled: bool,
}

/// The apps that ship with Havre, in their default order.
pub const BUILT_IN: &[Manifest] = &[
    Manifest {
        id: "mail",
        name: "Mail",
        description: "All your mail accounts in one inbox: read, search, write.",
        icon: "mail",
        route: "/mail",
        provides: &["mail.unread", "mail.send", "people.senders"],
        uses: &["calendar.invitations", "people.suggest"],
        default_enabled: true,
    },
    Manifest {
        id: "agenda",
        name: "Agenda",
        description: "Microsoft 365, Google and CalDAV calendars, side by side.",
        icon: "calendar",
        route: "/agenda",
        provides: &["calendar.events", "calendar.invitations", "people.attendees"],
        uses: &["people.suggest"],
        default_enabled: true,
    },
    Manifest {
        id: "people",
        name: "People",
        description: "Everyone you write to or meet, with your mail and meetings together.",
        icon: "people",
        route: "/people",
        provides: &["people.suggest"],
        uses: &["people.senders", "people.attendees"],
        default_enabled: true,
    },
    Manifest {
        id: "tasks",
        name: "Tasks",
        description: "Turn an email into a to-do, due on an Agenda day.",
        icon: "task",
        route: "/tasks",
        provides: &["tasks.due", "tasks.from_mail"],
        uses: &["mail.messages", "calendar.events", "people.suggest"],
        default_enabled: false,
    },
    Manifest {
        id: "notes",
        name: "Notes",
        description: "Markdown notes, attached to a meeting or a person.",
        icon: "note",
        route: "/notes",
        provides: &["notes.for_event", "notes.for_person"],
        uses: &["calendar.events", "people.suggest", "tasks.due"],
        default_enabled: false,
    },
    Manifest {
        id: "files",
        name: "Files",
        description: "Every attachment, every shared document, in one place.",
        icon: "folder",
        route: "/files",
        provides: &["files.list", "files.versions"],
        uses: &["mail.messages", "people.suggest"],
        default_enabled: false,
    },
    Manifest {
        id: "ai",
        name: "AI",
        description: "Ask about your mail and your days. Adds a brief, summaries and drafts to the other apps.",
        icon: "sparkle",
        route: "/ai",
        provides: &["ai.ask", "ai.brief", "ai.summaries"],
        uses: &["mail.messages", "calendar.events", "people.suggest"],
        default_enabled: false,
    },
];

/// A manifest plus your choices.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct App {
    #[serde(flatten)]
    pub manifest: Manifest,
    /// In the dock.
    pub enabled: bool,
    /// Place in the dock (0 first).
    pub position: i64,
}

/// Your choices, stored in `app_settings` (no row = default: enabled, in
/// the built-in order).
#[derive(Clone)]
pub struct AppSettings {
    pool: SqlitePool,
}

impl AppSettings {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Every app, in dock order (disabled ones too: the library shows them).
    pub async fn list(&self) -> Result<Vec<App>> {
        let rows: Vec<(String, bool, i64)> =
            sqlx::query_as("SELECT id, enabled, position FROM app_settings").fetch_all(&self.pool).await?;
        Ok(apply(&rows))
    }

    /// Shows or hides an app in the dock. At least one app stays.
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<Vec<App>> {
        let apps = self.list().await?;
        let app =
            apps.iter().find(|a| a.manifest.id == id).ok_or_else(|| Error::NotFound(format!("app {id}")))?;
        if !enabled && apps.iter().filter(|a| a.enabled).count() == 1 && app.enabled {
            return Err(Error::Config("Keep at least one app in the dock.".into()));
        }
        self.save(id, enabled, app.position).await?;
        self.list().await
    }

    /// Puts the apps in this order (ids; unknown ones are ignored, missing
    /// ones go after).
    pub async fn reorder(&self, ids: &[String]) -> Result<Vec<App>> {
        let apps = self.list().await?;
        let mut ordered: Vec<&App> =
            ids.iter().filter_map(|id| apps.iter().find(|a| a.manifest.id == id)).collect();
        for app in &apps {
            if !ordered.iter().any(|a| a.manifest.id == app.manifest.id) {
                ordered.push(app);
            }
        }
        for (position, app) in ordered.iter().enumerate() {
            self.save(app.manifest.id, app.enabled, position as i64).await?;
        }
        self.list().await
    }

    async fn save(&self, id: &str, enabled: bool, position: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO app_settings (id, enabled, position) VALUES (?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET enabled = excluded.enabled, position = excluded.position",
        )
        .bind(id)
        .bind(enabled)
        .bind(position)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// The built-in apps with the stored choices applied. Also used without a
/// database (demo mode): every app enabled, default order.
pub fn apply(rows: &[(String, bool, i64)]) -> Vec<App> {
    let mut apps: Vec<App> = BUILT_IN
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let row = rows.iter().find(|(id, _, _)| id == m.id);
            App {
                manifest: m.clone(),
                enabled: row.map_or(m.default_enabled, |r| r.1),
                position: row.map_or(i as i64, |r| r.2),
            }
        })
        .collect();
    // Stable sort: ties keep the built-in order.
    apps.sort_by_key(|a| a.position);
    apps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Database;

    #[tokio::test]
    async fn choose_and_order_apps() {
        let db = Database::in_memory().await.unwrap();
        let apps = db.apps();
        let ids = |list: &[App]| list.iter().map(|a| a.manifest.id).collect::<Vec<_>>();

        let list = apps.list().await.unwrap();
        assert_eq!(ids(&list), ["mail", "agenda", "people", "tasks", "notes", "files", "ai"]);
        // Tasks, Notes, Files and AI wait in the library until you add them.
        assert!(
            list.iter().all(|a| a.enabled == !["tasks", "notes", "files", "ai"].contains(&a.manifest.id))
        );
        apps.set_enabled("tasks", true).await.unwrap();

        let list = apps.reorder(&["agenda".into(), "mail".into()]).await.unwrap();
        assert_eq!(ids(&list), ["agenda", "mail", "people", "tasks", "notes", "files", "ai"]);
        apps.set_enabled("tasks", false).await.unwrap();

        apps.set_enabled("people", false).await.unwrap();
        apps.set_enabled("agenda", false).await.unwrap();
        // The last one can't go.
        assert!(apps.set_enabled("mail", false).await.is_err());
        let list = apps.list().await.unwrap();
        assert_eq!(list.iter().filter(|a| a.enabled).count(), 1);
        assert!(apps.set_enabled("nope", true).await.is_err());
    }
}
