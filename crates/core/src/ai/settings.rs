//! Which AI to use: stored in the `ai_settings` row, edited in
//! Settings › AI. The API key is encrypted (like mail passwords) and never
//! sent back to the browser: the UI only learns whether one is saved.

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::{Error, Result, secrets::Secrets};

/// The kinds of AI servers Havre talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Ollama, usually on the same machine. Nothing leaves your network.
    Ollama,
    /// Anything speaking the OpenAI API: OpenAI, Mistral, Groq,
    /// OpenRouter, LM Studio, vLLM…
    OpenAi,
    /// Anthropic's Claude.
    Anthropic,
    /// The Claude Code command line, with the Claude plan you signed in
    /// with. The "URL" is the command (`claude` or a full path).
    #[serde(rename = "claude-code")]
    ClaudeCode,
}

impl ProviderKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ollama" => Some(Self::Ollama),
            "openai" => Some(Self::OpenAi),
            "anthropic" => Some(Self::Anthropic),
            "claude-code" => Some(Self::ClaudeCode),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::ClaudeCode => "claude-code",
        }
    }

    /// Where it usually is, when the URL field is left empty.
    pub fn default_url(self) -> &'static str {
        match self {
            Self::Ollama => "http://localhost:11434",
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com",
            Self::ClaudeCode => "claude",
        }
    }

    /// Runs on your own machine or network (no data leaves it)?
    pub fn is_local(self, url: &str) -> bool {
        if self == Self::ClaudeCode {
            return false; // Claude runs at Anthropic.
        }
        let host = url.split("://").nth(1).unwrap_or(url).split(['/', ':']).next().unwrap_or("");
        self == Self::Ollama
            || matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
            || host.ends_with(".local")
            || host.starts_with("192.168.")
            || host.starts_with("10.")
    }
}

/// What the Settings page shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AiSettings {
    /// The AI app is in the dock (added from the App library).
    pub installed: bool,
    /// Installed *and* set up (a model chosen): AI features can run.
    pub enabled: bool,
    pub provider: ProviderKind,
    /// As typed (may be empty).
    pub url: String,
    /// What's actually used.
    pub effective_url: String,
    pub model: String,
    /// An API key is saved (never sent to the browser).
    pub has_key: bool,
    /// Embeddings server for search by meaning (empty = the chat server).
    pub embed_url: String,
    /// Embedding model; empty = search by meaning off.
    pub embed_model: String,
    /// Index mail in the background (needs `embed_model`).
    pub index_mail: bool,
    /// Your mail stays on your machine / network.
    pub local: bool,
    /// What the AI may read (Settings › AI › What it can read).
    pub read_mail: bool,
    pub read_agenda: bool,
    pub read_people: bool,
    pub read_notes: bool,
    pub read_tasks: bool,
    /// The Notes / Tasks apps are in the dock: without them, their switches
    /// don't show and the assistant doesn't get their tools.
    pub notes_app: bool,
    pub tasks_app: bool,
    /// Write the morning brief ahead, on weekdays at `brief_time` (HH:MM,
    /// the server's time zone).
    pub brief_enabled: bool,
    pub brief_time: String,
    /// Other AI apps may use Havre over MCP (with their key).
    pub mcp_enabled: bool,
}

/// A change from the Settings page; missing fields stay as they are.
/// (There's no on/off any more: adding or removing the AI app is the switch.)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AiSettingsUpdate {
    pub provider: Option<ProviderKind>,
    pub url: Option<String>,
    pub model: Option<String>,
    /// `""` removes the saved key.
    pub api_key: Option<String>,
    pub embed_url: Option<String>,
    pub embed_model: Option<String>,
    pub index_mail: Option<bool>,
    pub read_mail: Option<bool>,
    pub read_agenda: Option<bool>,
    pub read_people: Option<bool>,
    pub read_notes: Option<bool>,
    pub read_tasks: Option<bool>,
    pub brief_enabled: Option<bool>,
    pub brief_time: Option<String>,
    pub mcp_enabled: Option<bool>,
}

/// Everything needed to call the provider, key included (server side only).
#[derive(Clone)]
pub struct AiConfig {
    pub settings: AiSettings,
    pub api_key: Option<String>,
}

impl std::fmt::Debug for AiConfig {
    // Never print the key in logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiConfig").field("settings", &self.settings).field("api_key", &"…").finish()
    }
}

pub(crate) async fn load(pool: &SqlitePool, secrets: &Secrets) -> Result<AiConfig> {
    // `installed` comes from the dock: the AI app's row in app_settings
    // (no row: not added yet). Subqueries read it, and whether Notes and
    // Tasks are in the dock, in the same request.
    let r = sqlx::query(
        "SELECT s.*,
                COALESCE((SELECT enabled FROM app_settings WHERE id = 'ai'), 0) AS installed,
                COALESCE((SELECT enabled FROM app_settings WHERE id = 'notes'), 0) AS notes_app,
                COALESCE((SELECT enabled FROM app_settings WHERE id = 'tasks'), 0) AS tasks_app
         FROM ai_settings s WHERE s.id = 1",
    )
    .fetch_one(pool)
    .await?;
    let provider = ProviderKind::parse(r.get("provider")).unwrap_or(ProviderKind::Ollama);
    let url: String = r.get("url");
    let effective_url =
        if url.trim().is_empty() { provider.default_url().to_owned() } else { url.trim().to_owned() };
    let key: Option<Vec<u8>> = r.get("api_key");
    let api_key = match key {
        Some(bytes) => Some(secrets.decrypt(&bytes)?),
        None => None,
    };
    let installed: bool = r.get("installed");
    let model: String = r.get("model");
    // Claude Code has a default model; the others need one chosen.
    let configured = provider == ProviderKind::ClaudeCode || !model.trim().is_empty();
    Ok(AiConfig {
        settings: AiSettings {
            installed,
            enabled: installed && configured,
            provider,
            local: provider.is_local(&effective_url),
            url,
            effective_url,
            model,
            has_key: api_key.is_some(),
            embed_url: r.get("embed_url"),
            embed_model: r.get("embed_model"),
            index_mail: r.get("index_mail"),
            read_mail: r.get("read_mail"),
            read_agenda: r.get("read_agenda"),
            read_people: r.get("read_people"),
            read_notes: r.get("read_notes"),
            read_tasks: r.get("read_tasks"),
            notes_app: r.get("notes_app"),
            tasks_app: r.get("tasks_app"),
            brief_enabled: r.get("brief_enabled"),
            brief_time: r.get("brief_time"),
            mcp_enabled: r.get("mcp_enabled"),
        },
        api_key,
    })
}

pub(crate) async fn save(pool: &SqlitePool, secrets: &Secrets, update: &AiSettingsUpdate) -> Result<()> {
    let trim = |s: &Option<String>| s.as_ref().map(|v| v.trim().to_owned());
    // "7:30" → "07:30"; anything else is refused.
    let brief_time = match &update.brief_time {
        None => None,
        Some(t) => {
            let parsed = chrono::NaiveTime::parse_from_str(t.trim(), "%H:%M")
                .map_err(|_| Error::Config("The brief's time must look like 07:30.".into()))?;
            Some(parsed.format("%H:%M").to_string())
        }
    };
    sqlx::query(
        "UPDATE ai_settings SET
            provider = COALESCE(?, provider), url = COALESCE(?, url),
            model = COALESCE(?, model), embed_url = COALESCE(?, embed_url),
            embed_model = COALESCE(?, embed_model), index_mail = COALESCE(?, index_mail),
            read_mail = COALESCE(?, read_mail), read_agenda = COALESCE(?, read_agenda),
            read_people = COALESCE(?, read_people), read_notes = COALESCE(?, read_notes),
            read_tasks = COALESCE(?, read_tasks), brief_enabled = COALESCE(?, brief_enabled),
            brief_time = COALESCE(?, brief_time), mcp_enabled = COALESCE(?, mcp_enabled)
         WHERE id = 1",
    )
    .bind(update.provider.map(ProviderKind::as_str))
    .bind(trim(&update.url))
    .bind(trim(&update.model))
    .bind(trim(&update.embed_url))
    .bind(trim(&update.embed_model))
    .bind(update.index_mail)
    .bind(update.read_mail)
    .bind(update.read_agenda)
    .bind(update.read_people)
    .bind(update.read_notes)
    .bind(update.read_tasks)
    .bind(update.brief_enabled)
    .bind(brief_time)
    .bind(update.mcp_enabled)
    .execute(pool)
    .await?;
    if let Some(key) = &update.api_key {
        let key = key.trim();
        let stored = if key.is_empty() { None } else { Some(secrets.encrypt(key)?) };
        sqlx::query("UPDATE ai_settings SET api_key = ? WHERE id = 1").bind(stored).execute(pool).await?;
    }
    Ok(())
}
