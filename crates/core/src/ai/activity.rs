//! The AI activity log: what the assistant, the brief and MCP apps read or
//! did, newest first (Settings › AI › Activity). Kept 90 days.

use serde::Serialize;
use sqlx::{Row, SqlitePool};

use crate::Result;

#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub id: i64,
    /// Unix seconds.
    pub at: i64,
    /// `assistant`, `brief`, `summary`, `draft`, `you`, or `mcp:<app>`.
    pub source: String,
    pub action: String,
    pub detail: String,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Adds an entry (failures are only logged: the log must never block).
pub async fn log(pool: &SqlitePool, source: &str, action: &str, detail: &str) {
    let detail: String = detail.chars().take(300).collect();
    let result = sqlx::query("INSERT INTO ai_activity (at, source, action, detail) VALUES (?, ?, ?, ?)")
        .bind(now())
        .bind(source)
        .bind(action)
        .bind(detail)
        .execute(pool)
        .await;
    if let Err(err) = result {
        tracing::warn!(%err, "AI activity log");
    }
    let _ = sqlx::query("DELETE FROM ai_activity WHERE at < ?").bind(now() - 90 * 86_400).execute(pool).await;
}

pub async fn list(pool: &SqlitePool, limit: u32) -> Result<Vec<Activity>> {
    let rows = sqlx::query("SELECT id, at, source, action, detail FROM ai_activity ORDER BY id DESC LIMIT ?")
        .bind(i64::from(limit))
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| Activity {
            id: r.get("id"),
            at: r.get("at"),
            source: r.get("source"),
            action: r.get("action"),
            detail: r.get("detail"),
        })
        .collect())
}
