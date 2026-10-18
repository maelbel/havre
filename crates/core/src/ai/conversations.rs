//! The AI app's conversations, kept in the database.
//!
//! A conversation is a list of turns: what you asked, what the assistant
//! answered. An answer is more than text — the steps it took, the emails
//! and events it cites, the changes it proposes — so an assistant turn
//! keeps all of that as JSON (`answer`), exactly as the browser got it,
//! plus which proposals you already confirmed (`done`).
//!
//! Why JSON in a column, not tables for steps and proposals? Nothing ever
//! searches inside an answer: it's written once and shown again as is.
//! One column is simpler, and `serde_json` turns it back into a value.

use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use super::{Ai, LocalTime, assistant::Turn};
use crate::{Error, Result};

/// How many earlier turns go to the model with a new question (the rest
/// stays in the conversation, just not in the prompt).
const CONTEXT_TURNS: usize = 20;

/// A conversation in the list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConversationSummary {
    pub id: i64,
    pub title: String,
    pub updated_at: i64,
    /// The start of the last answer, for the list.
    pub last: String,
}

/// One turn, as stored.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoredTurn {
    pub id: i64,
    /// `user` or `assistant`.
    pub role: String,
    pub text: String,
    /// The whole answer (assistant turns): steps, proposals, citations.
    pub answer: Option<Value>,
    /// Proposals already confirmed (their position in `answer.proposals`).
    pub done: Vec<usize>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Conversation {
    #[serde(flatten)]
    pub summary: ConversationSummary,
    pub turns: Vec<StoredTurn>,
}

/// "What did Thomas say about the quote?\nand…" → its first line, at most
/// 60 characters: the conversation's title.
pub fn title_for(question: &str) -> String {
    let line = question.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("New conversation");
    cut(line, 60)
}

/// The first `max` characters (not bytes: "é" is two bytes), with "…".
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let short: String = text.chars().take(max).collect();
    format!("{}…", short.trim_end())
}

pub(crate) async fn list(pool: &SqlitePool) -> Result<Vec<ConversationSummary>> {
    // The last answer's text comes from a subquery: one request for the list.
    let rows = sqlx::query(
        "SELECT c.id, c.title, c.updated_at,
                (SELECT t.text FROM ai_turns t WHERE t.conversation_id = c.id AND t.role = 'assistant'
                 ORDER BY t.id DESC LIMIT 1) AS last
         FROM ai_conversations c ORDER BY c.updated_at DESC, c.id DESC LIMIT 500",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(summary_from_row).collect())
}

fn summary_from_row(r: &sqlx::sqlite::SqliteRow) -> ConversationSummary {
    let last: Option<String> = r.get("last");
    let first_line =
        last.as_deref().unwrap_or("").lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    ConversationSummary {
        id: r.get("id"),
        title: r.get("title"),
        updated_at: r.get("updated_at"),
        last: cut(&without_citations(&first_line.replace("**", "")), 90),
    }
}

/// "In [n1] two to-dos [t2]." → "In two to-dos.": the list shows plain
/// text, and an id like `n1` means nothing there.
fn without_citations(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let close = after.find(']');
        // `[` + one of m/e/n/t + digits + `]` is a citation.
        let is_citation = close.is_some_and(|c| {
            let inside = &after[..c];
            inside.len() > 1
                && inside.starts_with(['m', 'e', 'n', 't'])
                && inside[1..].chars().all(|ch| ch.is_ascii_digit())
        });
        if is_citation {
            out.push_str(rest[..open].trim_end());
            rest = &after[close.unwrap_or_default() + 1..];
        } else {
            out.push_str(&rest[..=open]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

pub(crate) async fn get(pool: &SqlitePool, id: i64) -> Result<Conversation> {
    let row = sqlx::query(
        "SELECT c.id, c.title, c.updated_at,
                (SELECT t.text FROM ai_turns t WHERE t.conversation_id = c.id AND t.role = 'assistant'
                 ORDER BY t.id DESC LIMIT 1) AS last
         FROM ai_conversations c WHERE c.id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| Error::NotFound("This conversation was deleted.".into()))?;
    let rows = sqlx::query(
        "SELECT id, role, text, answer, done, created_at FROM ai_turns WHERE conversation_id = ? ORDER BY id",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let turns = rows
        .iter()
        .map(|r| {
            let answer: Option<String> = r.get("answer");
            let done: String = r.get("done");
            StoredTurn {
                id: r.get("id"),
                role: r.get("role"),
                text: r.get("text"),
                // A broken value (it shouldn't happen) shows as no answer
                // rather than failing the whole conversation.
                answer: answer.and_then(|a| serde_json::from_str(&a).ok()),
                done: serde_json::from_str(&done).unwrap_or_default(),
                created_at: r.get("created_at"),
            }
        })
        .collect();
    Ok(Conversation { summary: summary_from_row(&row), turns })
}

/// Saves a question and its answer, in a new conversation or after the
/// others. Returns the conversation's id.
pub(crate) async fn save_exchange(
    pool: &SqlitePool,
    conversation: Option<i64>,
    question: &str,
    answer_text: &str,
    answer: &Value,
) -> Result<i64> {
    let now = Utc::now().timestamp();
    // Both turns, or neither: a transaction (`BEGIN IMMEDIATE` takes the
    // write lock at once, see `TaskStore::write`).
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let id = match conversation {
        Some(id) => {
            let found = sqlx::query("UPDATE ai_conversations SET updated_at = ? WHERE id = ?")
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            if found.rows_affected() == 0 {
                return Err(Error::NotFound("This conversation was deleted.".into()));
            }
            id
        }
        None => sqlx::query("INSERT INTO ai_conversations (title, created_at, updated_at) VALUES (?, ?, ?)")
            .bind(title_for(question))
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?
            .last_insert_rowid(),
    };
    sqlx::query("INSERT INTO ai_turns (conversation_id, role, text, created_at) VALUES (?, 'user', ?, ?)")
        .bind(id)
        .bind(question.trim())
        .bind(now)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO ai_turns (conversation_id, role, text, answer, created_at) VALUES (?, 'assistant', ?, ?, ?)",
    )
    .bind(id)
    .bind(answer_text)
    .bind(answer.to_string())
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// The turns to send to the model with a new question.
pub(crate) async fn context(pool: &SqlitePool, id: i64) -> Result<Vec<Turn>> {
    let conversation = get(pool, id).await?;
    let skip = conversation.turns.len().saturating_sub(CONTEXT_TURNS);
    Ok(conversation.turns.into_iter().skip(skip).map(|t| Turn { role: t.role, text: t.text }).collect())
}

pub(crate) async fn mark_done(
    pool: &SqlitePool,
    conversation: i64,
    turn: i64,
    proposal: usize,
) -> Result<()> {
    let done: Option<String> = sqlx::query_scalar(
        "SELECT done FROM ai_turns WHERE id = ? AND conversation_id = ? AND role = 'assistant'",
    )
    .bind(turn)
    .bind(conversation)
    .fetch_optional(pool)
    .await?;
    let done = done.ok_or_else(|| Error::NotFound("This answer is no longer here.".into()))?;
    let mut list: Vec<usize> = serde_json::from_str(&done).unwrap_or_default();
    if !list.contains(&proposal) {
        list.push(proposal);
        list.sort_unstable();
    }
    sqlx::query("UPDATE ai_turns SET done = ? WHERE id = ?")
        .bind(serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()))
        .bind(turn)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn rename(pool: &SqlitePool, id: i64, title: &str) -> Result<()> {
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::Config("Give the conversation a title.".into()));
    }
    sqlx::query("UPDATE ai_conversations SET title = ? WHERE id = ?")
        .bind(cut(title, 120))
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    // Its turns go with it (`ON DELETE CASCADE`).
    sqlx::query("DELETE FROM ai_conversations WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

impl Ai {
    pub async fn conversations(&self) -> Result<Vec<ConversationSummary>> {
        list(&self.pool).await
    }

    /// How many conversations there are (Settings › Your apps › AI).
    pub async fn conversation_count(&self) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM ai_conversations").fetch_one(&self.pool).await?)
    }

    pub async fn conversation(&self, id: i64) -> Result<Conversation> {
        get(&self.pool, id).await
    }

    /// Asks `question` in a conversation (`None`: a new one), with its
    /// earlier turns as context, and keeps both turns. If the model fails,
    /// nothing is saved: you can ask again.
    pub async fn ask_in(
        &self,
        conversation: Option<i64>,
        question: &str,
        local: LocalTime,
    ) -> Result<Conversation> {
        if question.trim().is_empty() {
            return Err(Error::Config("Ask something first.".into()));
        }
        let mut turns = match conversation {
            Some(id) => context(&self.pool, id).await?,
            None => Vec::new(),
        };
        turns.push(Turn { role: "user".into(), text: question.trim().to_owned() });
        let answer = self.ask(&turns, local).await?;
        let value = serde_json::to_value(&answer).map_err(|e| Error::Ai(e.to_string()))?;
        let id = save_exchange(&self.pool, conversation, question, &answer.text, &value).await?;
        get(&self.pool, id).await
    }

    pub async fn mark_done(&self, conversation: i64, turn: i64, proposal: usize) -> Result<()> {
        mark_done(&self.pool, conversation, turn, proposal).await
    }

    pub async fn rename_conversation(&self, id: i64, title: &str) -> Result<()> {
        rename(&self.pool, id, title).await
    }

    pub async fn delete_conversation(&self, id: i64) -> Result<()> {
        delete(&self.pool, id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Database;
    use serde_json::json;

    #[test]
    fn citations_are_left_out_of_the_list() {
        assert_eq!(without_citations("In [n1] two to-dos [t12]."), "In two to-dos.");
        assert_eq!(without_citations("See [the note] and [x1]"), "See [the note] and [x1]");
    }

    #[test]
    fn titles() {
        assert_eq!(title_for("  \nWhat did Thomas say?\nAnd Claire?"), "What did Thomas say?");
        assert_eq!(title_for(&"é".repeat(70)).chars().count(), 61, "60 characters and …");
        assert_eq!(title_for(""), "New conversation");
    }

    #[tokio::test]
    async fn conversations_keep_their_turns() {
        let db = Database::in_memory().await.unwrap();
        let pool = db.pool();
        let answer = json!({ "text": "He sent it at 08:15.", "proposals": [{ "kind": "email" }] });
        let id = save_exchange(&pool, None, "What did Thomas send?", "He sent it at 08:15.", &answer)
            .await
            .unwrap();
        save_exchange(&pool, Some(id), "Reply that it's fine", "**Prepared** a reply.", &answer)
            .await
            .unwrap();

        let c = get(&pool, id).await.unwrap();
        assert_eq!(c.summary.title, "What did Thomas send?");
        assert_eq!(c.summary.last, "Prepared a reply.", "marks are left out of the list");
        assert_eq!(c.turns.len(), 4);
        assert_eq!(c.turns[1].answer.as_ref().unwrap()["proposals"][0]["kind"], "email");

        // The model gets the earlier turns, as text.
        let turns = context(&pool, id).await.unwrap();
        assert_eq!(
            turns.iter().map(|t| t.role.as_str()).collect::<Vec<_>>(),
            ["user", "assistant", "user", "assistant"]
        );

        // Confirming a proposal is remembered, once.
        let turn = c.turns[3].id;
        mark_done(&pool, id, turn, 0).await.unwrap();
        mark_done(&pool, id, turn, 0).await.unwrap();
        assert_eq!(get(&pool, id).await.unwrap().turns[3].done, vec![0]);
        assert!(mark_done(&pool, id, c.turns[2].id, 0).await.is_err(), "a question has no proposals");

        rename(&pool, id, "Thomas’s quote").await.unwrap();
        assert!(rename(&pool, id, " ").await.is_err());
        let other = save_exchange(&pool, None, "Week?", "Calm.", &json!({})).await.unwrap();
        let all = list(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|c| c.title == "Thomas’s quote"));

        delete(&pool, id).await.unwrap();
        assert!(get(&pool, id).await.is_err());
        assert!(save_exchange(&pool, Some(id), "Still there?", "", &json!({})).await.is_err());
        assert_eq!(list(&pool).await.unwrap()[0].id, other);
    }
}
