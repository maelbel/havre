//! Search by meaning.
//!
//! Each message becomes a list of numbers (an **embedding**, made by the
//! AI server) such that messages about the same thing get similar lists,
//! even with different words ("devis" and "quote", "budget review" and
//! "Q4 spending"). A search turns the question into such a list too and
//! looks for the closest messages: a dot product per message, which is
//! fast enough for tens of thousands of messages without a special
//! database.
//!
//! The index is built in the background, newest messages first, and kept
//! in memory for searching (rebuilt from `mail_vectors` when needed).

use std::sync::RwLock;

use mail_parser::MessageParser;
use serde::Serialize;
use sqlx::{Row, SqlitePool};

use super::provider::Client;
use crate::{Result, mail::Person};

/// At most this many messages are indexed (the newest ones).
pub const MAX_INDEXED: i64 = 20_000;
/// Characters of the body used per message.
const BODY_CHARS: usize = 1_500;

/// A search result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    pub account: i64,
    pub folder: String,
    pub uid: u32,
    pub subject: String,
    pub from: Option<Person>,
    pub date: Option<String>,
    /// How close (0–1).
    pub score: f32,
}

struct Entry {
    account: i64,
    folder: String,
    uid: u32,
    vector: Vec<f32>,
}

/// The vectors in memory, for one model.
struct Loaded {
    model: String,
    entries: Vec<Entry>,
}

pub struct MailIndex {
    pool: SqlitePool,
    loaded: RwLock<Option<Loaded>>,
}

/// One message to embed.
struct Pending {
    account: i64,
    folder: String,
    uid: u32,
    text: String,
    with_body: bool,
}

impl MailIndex {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool, loaded: RwLock::new(None) }
    }

    /// How many messages are indexed with `model`, out of how many.
    pub async fn progress(&self, model: &str) -> Result<(i64, i64)> {
        let done: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mail_vectors WHERE model = ?")
            .bind(model)
            .fetch_one(&self.pool)
            .await?;
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages").fetch_one(&self.pool).await?;
        Ok((done, total.min(MAX_INDEXED)))
    }

    /// Indexes up to `batch` messages: new ones, and ones whose body was
    /// downloaded since. Returns how many were done (0: nothing left).
    pub async fn step(&self, client: &Client, model: &str, batch: i64) -> Result<usize> {
        // The newest messages without a vector for this model, or indexed
        // before their body was known.
        let rows = sqlx::query(
            "SELECT m.account_id, m.folder, m.uid, m.subject, m.from_name, m.from_address, m.date, b.raw
             FROM (SELECT * FROM messages ORDER BY datetime(date) DESC LIMIT ?1) m
             LEFT JOIN mail_vectors v
               ON v.account_id = m.account_id AND v.folder = m.folder AND v.uid = m.uid AND v.model = ?2
             LEFT JOIN bodies b ON b.account_id = m.account_id AND b.folder = m.folder AND b.uid = m.uid
             WHERE v.uid IS NULL OR (v.with_body = 0 AND b.uid IS NOT NULL)
             ORDER BY datetime(m.date) DESC LIMIT ?3",
        )
        .bind(MAX_INDEXED)
        .bind(model)
        .bind(batch)
        .fetch_all(&self.pool)
        .await?;
        if rows.is_empty() {
            return Ok(0);
        }
        let pending: Vec<Pending> = rows
            .iter()
            .map(|r| {
                let raw: Option<Vec<u8>> = r.get("raw");
                let body = raw.as_deref().map(body_text).unwrap_or_default();
                let from = match (
                    r.get::<Option<String>, _>("from_name"),
                    r.get::<Option<String>, _>("from_address"),
                ) {
                    (Some(n), Some(a)) => format!("{n} <{a}>"),
                    (n, a) => n.or(a).unwrap_or_default(),
                };
                let subject: String = r.get("subject");
                Pending {
                    account: r.get("account_id"),
                    folder: r.get("folder"),
                    uid: r.get::<i64, _>("uid") as u32,
                    text: format!("From: {from}\nSubject: {subject}\n\n{body}"),
                    with_body: raw.is_some(),
                }
            })
            .collect();
        let texts: Vec<String> = pending.iter().map(|p| p.text.clone()).collect();
        let vectors = client.embed(&texts).await?;

        let mut tx = self.pool.begin().await?;
        for (p, v) in pending.iter().zip(&vectors) {
            sqlx::query(
                "INSERT OR REPLACE INTO mail_vectors (account_id, folder, uid, model, with_body, vector)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(p.account)
            .bind(&p.folder)
            .bind(p.uid as i64)
            .bind(model)
            .bind(p.with_body)
            .bind(to_bytes(v))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        // Next search reloads (cheap compared with embedding).
        *self.loaded.write().unwrap() = None;
        Ok(pending.len())
    }

    /// Vectors of another model are useless: drop them.
    pub async fn forget_other_models(&self, model: &str) -> Result<()> {
        sqlx::query("DELETE FROM mail_vectors WHERE model <> ?").bind(model).execute(&self.pool).await?;
        *self.loaded.write().unwrap() = None;
        Ok(())
    }

    async fn ensure_loaded(&self, model: &str) -> Result<()> {
        if self.loaded.read().unwrap().as_ref().is_some_and(|l| l.model == model) {
            return Ok(());
        }
        let rows = sqlx::query("SELECT account_id, folder, uid, vector FROM mail_vectors WHERE model = ?")
            .bind(model)
            .fetch_all(&self.pool)
            .await?;
        let entries = rows
            .iter()
            .map(|r| Entry {
                account: r.get("account_id"),
                folder: r.get("folder"),
                uid: r.get::<i64, _>("uid") as u32,
                vector: from_bytes(r.get("vector")),
            })
            .collect();
        *self.loaded.write().unwrap() = Some(Loaded { model: model.to_owned(), entries });
        Ok(())
    }

    /// The messages closest in meaning to `query`.
    pub async fn search(&self, client: &Client, model: &str, query: &str, limit: usize) -> Result<Vec<Hit>> {
        let q = client.embed(&[query.to_owned()]).await?.pop().unwrap_or_default();
        self.ensure_loaded(model).await?;
        let mut best: Vec<(f32, i64, String, u32)> = {
            let loaded = self.loaded.read().unwrap();
            let entries = loaded.as_ref().map(|l| l.entries.as_slice()).unwrap_or_default();
            entries
                .iter()
                .filter(|e| e.vector.len() == q.len())
                .map(|e| (dot(&q, &e.vector), e.account, e.folder.clone(), e.uid))
                .collect()
        };
        best.sort_by(|a, b| b.0.total_cmp(&a.0));
        best.truncate(limit);

        let mut hits = Vec::new();
        for (score, account, folder, uid) in best {
            let row = sqlx::query(
                "SELECT subject, from_name, from_address, date FROM messages
                 WHERE account_id = ? AND folder = ? AND uid = ?",
            )
            .bind(account)
            .bind(&folder)
            .bind(uid as i64)
            .fetch_optional(&self.pool)
            .await?;
            let Some(r) = row else { continue };
            hits.push(Hit {
                account,
                folder,
                uid,
                subject: r.get("subject"),
                from: match (
                    r.get::<Option<String>, _>("from_name"),
                    r.get::<Option<String>, _>("from_address"),
                ) {
                    (None, None) => None,
                    (name, address) => Some(Person { name, address }),
                },
                date: r.get("date"),
                score,
            });
        }
        Ok(hits)
    }
}

/// The readable text of a raw email, shortened.
pub fn body_text(raw: &[u8]) -> String {
    let text = MessageParser::default()
        .parse(raw)
        .and_then(|m| m.body_text(0).map(|t| t.into_owned()))
        .unwrap_or_default();
    // Quoted history adds little; keep what this message says.
    let own: String = text
        .lines()
        .take_while(|l| {
            !l.starts_with("-----Original Message") && !(l.starts_with("On ") && l.ends_with("wrote:"))
        })
        .filter(|l| !l.trim_start().starts_with('>'))
        .collect::<Vec<_>>()
        .join("\n");
    let squeezed = own.split_whitespace().collect::<Vec<_>>().join(" ");
    squeezed.chars().take(BODY_CHARS).collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_bytes(b: &[u8]) -> Vec<f32> {
    // `as_chunks::<4>()` cuts the bytes into `[u8; 4]` arrays (and a leftover,
    // ignored): the size is known when compiling, so no indexing is needed.
    let (chunks, _rest) = b.as_chunks::<4>();
    chunks.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_round_trip() {
        let v = vec![0.25f32, -1.5, 3.0];
        assert_eq!(from_bytes(&to_bytes(&v)), v);
    }

    #[test]
    fn body_text_skips_quotes() {
        let raw = b"From: a@b.c\r\nSubject: x\r\nContent-Type: text/plain\r\n\r\nSure, Thursday works.\r\n\r\nOn Tue, Anna wrote:\r\n> Can we meet?\r\n";
        assert_eq!(body_text(raw), "Sure, Thursday works.");
    }
}
