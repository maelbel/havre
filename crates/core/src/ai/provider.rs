//! Talking to the AI server.
//!
//! Two dialects cover the three choices:
//! - the **OpenAI** API (`/chat/completions`, `/embeddings`, `/models`),
//!   spoken by OpenAI, Mistral, Groq, OpenRouter, LM Studio… and by
//!   **Ollama** (under `/v1`);
//! - the **Anthropic** API (`/v1/messages`).
//!
//! Havre's own types ([`Request`], [`Message`], [`Reply`]) are translated to
//! each dialect here, so the features never care which one is used.
//!
//! Streaming: the server sends the answer in small pieces (Server-Sent
//! Events). [`Client::stream_reply`] forwards each piece of text to a channel
//! as it arrives, so the UI can show the text being written, and puts the
//! tool calls together from their pieces (`SseLines`, `StreamedReply`).

use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{
    claude_code::{ClaudeCode, Run},
    settings::{AiConfig, ProviderKind},
};
use crate::{Error, Result};

/// A tool the model may call (the assistant's actions).
#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema of the arguments.
    pub schema: Value,
}

/// The model asked to run a tool.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// One turn of a conversation.
#[derive(Debug, Clone)]
pub enum Message {
    User(String),
    Assistant {
        text: String,
        calls: Vec<ToolCall>,
    },
    /// What the tools returned: `(call id, result)`.
    ToolResults(Vec<(String, String)>),
}

#[derive(Debug, Clone, Default)]
pub struct Request {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub max_tokens: u32,
}

impl Request {
    /// One question, one answer.
    pub fn simple(system: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            system: system.into(),
            messages: vec![Message::User(user.into())],
            tools: Vec::new(),
            max_tokens: 1200,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Reply {
    pub text: String,
    pub calls: Vec<ToolCall>,
}

/// A configured connection to the AI server.
#[derive(Clone)]
pub struct Client {
    kind: ProviderKind,
    /// Chat API base: `…/v1` for the OpenAI dialect, the host for Anthropic.
    base: String,
    key: Option<String>,
    model: String,
    /// Embeddings (always the OpenAI dialect): base, key, model.
    embed: Option<(String, Option<String>, String)>,
    http: reqwest::Client,
    /// Set when the provider is Claude Code (a command, not an HTTP API).
    claude: Option<ClaudeCode>,
}

/// `http://localhost:11434` → `http://localhost:11434/v1` (Ollama's
/// OpenAI-compatible API); other URLs are used as typed.
fn openai_base(kind: ProviderKind, url: &str) -> String {
    let url = url.trim_end_matches('/');
    if kind == ProviderKind::Ollama && !url.ends_with("/v1") { format!("{url}/v1") } else { url.to_owned() }
}

impl Client {
    pub fn new(config: &AiConfig) -> Result<Self> {
        let s = &config.settings;
        if s.model.is_empty() && s.provider != ProviderKind::ClaudeCode {
            return Err(Error::Ai("Choose a model in Settings › AI.".into()));
        }
        let claude = (s.provider == ProviderKind::ClaudeCode)
            .then(|| ClaudeCode { command: s.effective_url.clone(), model: s.model.clone() });
        let base = match s.provider {
            ProviderKind::Anthropic | ProviderKind::ClaudeCode => {
                s.effective_url.trim_end_matches('/').trim_end_matches("/v1").to_owned()
            }
            kind => openai_base(kind, &s.effective_url),
        };
        let embed = (!s.embed_model.is_empty()).then(|| {
            if s.embed_url.trim().is_empty() {
                // Same server (Anthropic has no embeddings: local Ollama then).
                match s.provider {
                    ProviderKind::Anthropic | ProviderKind::ClaudeCode => (
                        openai_base(ProviderKind::Ollama, ProviderKind::Ollama.default_url()),
                        None,
                        s.embed_model.clone(),
                    ),
                    _ => (base.clone(), config.api_key.clone(), s.embed_model.clone()),
                }
            } else {
                let url = s.embed_url.trim();
                // An Ollama address (port 11434) gets its /v1.
                let kind = if url.contains(":11434") { ProviderKind::Ollama } else { ProviderKind::OpenAi };
                let same = url.trim_end_matches('/') == s.effective_url.trim_end_matches('/');
                (
                    openai_base(kind, url),
                    if same { config.api_key.clone() } else { None },
                    s.embed_model.clone(),
                )
            }
        });
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // Local models can be slow on a laptop.
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| Error::Ai(e.to_string()))?;
        Ok(Self {
            kind: s.provider,
            base,
            key: config.api_key.clone(),
            model: s.model.clone(),
            embed,
            http,
            claude,
        })
    }

    pub fn is_claude_code(&self) -> bool {
        self.claude.is_some()
    }

    pub(crate) fn claude_code(&self) -> Option<&ClaudeCode> {
        self.claude.as_ref()
    }

    pub fn can_embed(&self) -> bool {
        self.embed.is_some()
    }

    /// A short name for messages: "Ollama", "Anthropic"…
    fn name(&self) -> &'static str {
        match self.kind {
            ProviderKind::Ollama => "Ollama",
            ProviderKind::OpenAi => "The AI server",
            ProviderKind::Anthropic => "Anthropic",
            ProviderKind::ClaudeCode => "Claude Code",
        }
    }

    fn post(&self, url: String, key: Option<&str>) -> reqwest::RequestBuilder {
        let mut req = self.http.post(url);
        match self.kind {
            ProviderKind::Anthropic => {
                if let Some(key) = key {
                    req = req.header("x-api-key", key);
                }
                req = req.header("anthropic-version", "2023-06-01");
            }
            _ => {
                if let Some(key) = key {
                    req = req.bearer_auth(key);
                }
            }
        }
        req
    }

    /// Sends a chat request; turns failures into readable errors.
    async fn send(&self, req: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        self.send_to(req, self.name(), &self.base).await
    }

    async fn send_to(
        &self,
        req: reqwest::RequestBuilder,
        name: &str,
        base: &str,
    ) -> Result<reqwest::Response> {
        let response = req.send().await.map_err(|e| {
            if e.is_connect() {
                Error::Ai(format!("{name} can't be reached at {base}. Is it running?"))
            } else if e.is_timeout() {
                Error::Ai(format!("{name} took too long to answer."))
            } else {
                Error::Ai(format!("{name}: {e}"))
            }
        })?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        let detail =
            body["error"]["message"].as_str().or(body["error"].as_str()).unwrap_or_default().to_owned();
        let ollama = base.contains(":11434") || self.kind == ProviderKind::Ollama;
        Err(Error::Ai(match status.as_u16() {
            401 | 403 => format!("{name} refused the API key. Check it in Settings › AI. {detail}"),
            404 if detail.contains("model") || ollama => {
                let model = detail.split('"').nth(1).unwrap_or(&self.model).to_owned();
                if ollama {
                    format!("Model “{model}” not found. Install it with `ollama pull {model}`.")
                } else {
                    format!("Model “{model}” not found. {detail}")
                }
            }
            429 => format!("{name} is rate-limiting requests; try again in a moment. {detail}"),
            _ => format!("{name} answered {status}. {detail}"),
        }))
    }

    // ---- Chat ------------------------------------------------------------------------

    /// The whole answer at once (used with tools).
    pub async fn complete(&self, request: &Request) -> Result<Reply> {
        if let Some(claude) = &self.claude {
            let text = claude.run(&request.system, &flatten(request), Run::default(), None).await?;
            return Ok(Reply { text: text.trim().to_owned(), calls: Vec::new() });
        }
        let (url, body) = self.chat_body(request, false);
        let response = self.send(self.post(url, self.key.as_deref()).json(&body)).await?;
        let body: Value = response.json().await.map_err(|e| Error::Ai(format!("unreadable answer: {e}")))?;
        Ok(match self.kind {
            ProviderKind::Anthropic => anthropic_reply(&body),
            _ => openai_reply(&body),
        })
    }

    /// The answer piece by piece, sent to `out` as it's written. Returns
    /// the whole text at the end (summaries, drafts, the brief: no tools).
    pub async fn stream(&self, request: &Request, out: &mpsc::Sender<String>) -> Result<String> {
        Ok(self.stream_reply(request, out).await?.text)
    }

    /// Like [`Client::complete`], but the text is sent to `out` as it's
    /// written; tool calls, which arrive in pieces too, are put together and
    /// returned at the end with the whole text.
    ///
    /// If `out` is closed (nobody reads any more), it stops asking and
    /// returns what it has: the caller sees `out.is_closed()`.
    pub async fn stream_reply(&self, request: &Request, out: &mpsc::Sender<String>) -> Result<Reply> {
        if let Some(claude) = &self.claude {
            let text = claude.run(&request.system, &flatten(request), Run::default(), Some(out)).await?;
            return Ok(Reply { text: text.trim().to_owned(), calls: Vec::new() });
        }
        let (url, body) = self.chat_body(request, true);
        let mut response = self.send(self.post(url, self.key.as_deref()).json(&body)).await?;
        let mut lines = SseLines::default();
        let mut reply = StreamedReply::default();
        while let Some(chunk) =
            response.chunk().await.map_err(|e| Error::Ai(format!("{}: {e}", self.name())))?
        {
            for data in lines.push(&chunk) {
                let Ok(event) = serde_json::from_str::<Value>(&data) else { continue };
                let piece =
                    reply.add(self.kind, &event).map_err(|e| Error::Ai(format!("{}: {e}", self.name())))?;
                // `if let … && …`: only a non-empty piece is sent.
                if let Some(piece) = piece
                    && out.send(piece).await.is_err()
                {
                    // The reader left (closed the page): stop asking.
                    return Ok(reply.finish());
                }
            }
        }
        Ok(reply.finish())
    }

    fn chat_body(&self, r: &Request, stream: bool) -> (String, Value) {
        match self.kind {
            ProviderKind::Anthropic => {
                let messages: Vec<Value> = r
                    .messages
                    .iter()
                    .map(|m| match m {
                        Message::User(text) => json!({ "role": "user", "content": text }),
                        Message::Assistant { text, calls } => {
                            let mut content = Vec::new();
                            if !text.is_empty() {
                                content.push(json!({ "type": "text", "text": text }));
                            }
                            for c in calls {
                                content.push(json!({ "type": "tool_use", "id": c.id, "name": c.name, "input": c.input }));
                            }
                            json!({ "role": "assistant", "content": content })
                        }
                        Message::ToolResults(results) => json!({
                            "role": "user",
                            "content": results.iter().map(|(id, content)| json!({
                                "type": "tool_result", "tool_use_id": id, "content": content
                            })).collect::<Vec<_>>(),
                        }),
                    })
                    .collect();
                let mut body = json!({
                    "model": self.model,
                    "max_tokens": r.max_tokens.max(256),
                    "system": r.system,
                    "messages": messages,
                    "stream": stream,
                });
                if !r.tools.is_empty() {
                    body["tools"] = r
                        .tools
                        .iter()
                        .map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.schema }))
                        .collect();
                }
                (format!("{}/v1/messages", self.base), body)
            }
            _ => {
                let mut messages = vec![json!({ "role": "system", "content": r.system })];
                for m in &r.messages {
                    match m {
                        Message::User(text) => messages.push(json!({ "role": "user", "content": text })),
                        Message::Assistant { text, calls } => {
                            let mut msg = json!({ "role": "assistant", "content": text });
                            if !calls.is_empty() {
                                msg["tool_calls"] = calls
                                    .iter()
                                    .map(|c| {
                                        json!({
                                            "id": c.id, "type": "function",
                                            "function": { "name": c.name, "arguments": c.input.to_string() },
                                        })
                                    })
                                    .collect();
                            }
                            messages.push(msg);
                        }
                        Message::ToolResults(results) => {
                            for (id, content) in results {
                                messages
                                    .push(json!({ "role": "tool", "tool_call_id": id, "content": content }));
                            }
                        }
                    }
                }
                let mut body = json!({ "model": self.model, "messages": messages, "stream": stream });
                if !r.tools.is_empty() {
                    body["tools"] = r
                        .tools
                        .iter()
                        .map(|t| json!({
                            "type": "function",
                            "function": { "name": t.name, "description": t.description, "parameters": t.schema },
                        }))
                        .collect();
                }
                (format!("{}/chat/completions", self.base), body)
            }
        }
    }

    // ---- Embeddings and models ----------------------------------------------------------

    /// One vector per text, each of length 1 (so comparing two is a dot
    /// product).
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let Some((base, key, model)) = &self.embed else {
            return Err(Error::Ai(
                "Choose an embedding model in Settings › AI for search by meaning.".into(),
            ));
        };
        let mut req = self.http.post(format!("{base}/embeddings"));
        if let Some(key) = key {
            req = req.bearer_auth(key);
        }
        let response = self
            .send_to(req.json(&json!({ "model": model, "input": texts })), "The embeddings server", base)
            .await?;
        let body: Value = response.json().await.map_err(|e| Error::Ai(format!("unreadable answer: {e}")))?;
        let mut rows: Vec<(usize, Vec<f32>)> = body["data"]
            .as_array()
            .ok_or_else(|| Error::Ai("The embeddings answer has no data.".into()))?
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let index = d["index"].as_u64().map_or(i, |n| n as usize);
                let v = d["embedding"]
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_f64).map(|x| x as f32).collect())
                    .unwrap_or_default();
                (index, normalize(v))
            })
            .collect();
        rows.sort_by_key(|(i, _)| *i);
        if rows.len() != texts.len() {
            return Err(Error::Ai("The embeddings answer doesn't match the request.".into()));
        }
        Ok(rows.into_iter().map(|(_, v)| v).collect())
    }

    /// The models the server offers (for the Settings page).
    pub async fn models(&self) -> Result<Vec<String>> {
        if self.claude.is_some() {
            // Claude Code's model aliases (empty = its default).
            return Ok(vec!["haiku".into(), "opus".into(), "sonnet".into()]);
        }
        let url = match self.kind {
            ProviderKind::Anthropic => format!("{}/v1/models?limit=100", self.base),
            _ => format!("{}/models", self.base),
        };
        let mut req = self.http.get(url);
        req = match (self.kind, &self.key) {
            (ProviderKind::Anthropic, Some(key)) => {
                req.header("x-api-key", key).header("anthropic-version", "2023-06-01")
            }
            (ProviderKind::Anthropic, None) => req.header("anthropic-version", "2023-06-01"),
            (_, Some(key)) => req.bearer_auth(key),
            (_, None) => req,
        };
        let body: Value = self.send(req).await?.json().await.map_err(|e| Error::Ai(e.to_string()))?;
        let mut ids: Vec<String> = body["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["id"].as_str().map(str::to_owned))
            .collect();
        ids.sort();
        Ok(ids)
    }
}

/// A conversation as one prompt (for Claude Code, which takes text).
fn flatten(request: &Request) -> String {
    if let [Message::User(text)] = request.messages.as_slice() {
        return text.clone();
    }
    let mut out = String::new();
    for m in &request.messages {
        match m {
            Message::User(text) => out.push_str(&format!("User: {text}\n\n")),
            Message::Assistant { text, .. } => out.push_str(&format!("Assistant: {text}\n\n")),
            Message::ToolResults(_) => {}
        }
    }
    out
}

/// Scales a vector to length 1.
pub fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

/// Splits a Server-Sent Events body into its `data:` payloads.
///
/// The body arrives in chunks of any size: a chunk may end in the middle of a
/// line (even in the middle of a UTF-8 character), so the bytes after the
/// last new line wait in `pending` for the next chunk.
#[derive(Default)]
struct SseLines {
    pending: Vec<u8>,
}

impl SseLines {
    /// Adds a chunk; returns the complete `data:` payloads it finished.
    fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(chunk);
        let mut payloads = Vec::new();
        while let Some(end) = self.pending.iter().position(|&b| b == b'\n') {
            // `drain` removes the line from the buffer and gives it to us.
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            if let Some(data) = line.trim().strip_prefix("data:") {
                let data = data.trim();
                if !data.is_empty() && data != "[DONE]" {
                    payloads.push(data.to_owned());
                }
            }
        }
        payloads
    }
}

/// A tool call being received: its arguments come as pieces of a JSON text.
#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
    /// Ollama sometimes sends the arguments as an object, all at once.
    object: Option<Value>,
}

/// A streamed answer being put together, event by event.
#[derive(Default)]
struct StreamedReply {
    text: String,
    /// By the index the server gives each call (calls can be interleaved).
    calls: Vec<(u64, PartialCall)>,
}

impl StreamedReply {
    fn call(&mut self, index: u64) -> &mut PartialCall {
        // Find it, or add it: two steps, because the borrow checker won't let
        // us return a borrow from the search and then push in the same branch.
        if let Some(pos) = self.calls.iter().position(|(i, _)| *i == index) {
            return &mut self.calls[pos].1;
        }
        self.calls.push((index, PartialCall::default()));
        &mut self.calls.last_mut().expect("just pushed").1
    }

    /// Reads one event of either dialect. Returns the piece of text it
    /// carries (if any), or the server's error message.
    fn add(&mut self, kind: ProviderKind, event: &Value) -> std::result::Result<Option<String>, String> {
        if let Some(message) = event["error"]["message"].as_str() {
            return Err(message.to_owned());
        }
        let piece = match kind {
            ProviderKind::Anthropic => {
                let index = event["index"].as_u64().unwrap_or_default();
                match event["type"].as_str() {
                    Some("content_block_start") if event["content_block"]["type"] == "tool_use" => {
                        let block = &event["content_block"];
                        let call = self.call(index);
                        call.id = block["id"].as_str().unwrap_or_default().to_owned();
                        call.name = block["name"].as_str().unwrap_or_default().to_owned();
                        None
                    }
                    Some("content_block_delta") => match event["delta"]["type"].as_str() {
                        Some("input_json_delta") => {
                            let part = event["delta"]["partial_json"].as_str().unwrap_or_default();
                            self.call(index).arguments.push_str(part);
                            None
                        }
                        _ => event["delta"]["text"].as_str(),
                    },
                    _ => None,
                }
            }
            _ => {
                let delta = &event["choices"][0]["delta"];
                for (n, part) in delta["tool_calls"].as_array().into_iter().flatten().enumerate() {
                    let call = self.call(part["index"].as_u64().unwrap_or(n as u64));
                    if let Some(id) = part["id"].as_str() {
                        call.id = id.to_owned();
                    }
                    let f = &part["function"];
                    if let Some(name) = f["name"].as_str() {
                        call.name.push_str(name);
                    }
                    match &f["arguments"] {
                        Value::String(piece) => call.arguments.push_str(piece),
                        Value::Null => {}
                        object => call.object = Some(object.clone()),
                    }
                }
                delta["content"].as_str()
            }
        };
        let piece = piece.filter(|p| !p.is_empty()).map(str::to_owned);
        if let Some(piece) = &piece {
            self.text.push_str(piece);
        }
        Ok(piece)
    }

    fn finish(self) -> Reply {
        let calls = self
            .calls
            .into_iter()
            .filter(|(_, c)| !c.name.is_empty())
            .enumerate()
            .map(|(n, (_, c))| ToolCall {
                id: if c.id.is_empty() { format!("call_{n}") } else { c.id },
                name: c.name,
                // Empty arguments (a tool without any) are `{}`; unreadable ones too,
                // and the tool then says what it's missing.
                input: c.object.unwrap_or_else(|| serde_json::from_str(&c.arguments).unwrap_or(json!({}))),
            })
            .collect();
        Reply { text: self.text.trim().to_owned(), calls }
    }
}

fn openai_reply(body: &Value) -> Reply {
    let message = &body["choices"][0]["message"];
    let calls = message["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, c)| {
            let f = &c["function"];
            // Arguments come as a JSON string (Ollama sometimes sends an object).
            let input = match &f["arguments"] {
                Value::String(s) => serde_json::from_str(s).unwrap_or(json!({})),
                Value::Null => json!({}),
                other => other.clone(),
            };
            Some(ToolCall {
                id: c["id"].as_str().map_or_else(|| format!("call_{i}"), str::to_owned),
                name: f["name"].as_str()?.to_owned(),
                input,
            })
        })
        .collect();
    Reply { text: message["content"].as_str().unwrap_or_default().trim().to_owned(), calls }
}

fn anthropic_reply(body: &Value) -> Reply {
    let mut reply = Reply::default();
    for block in body["content"].as_array().into_iter().flatten() {
        match block["type"].as_str() {
            Some("text") => reply.text.push_str(block["text"].as_str().unwrap_or_default()),
            Some("tool_use") => reply.calls.push(ToolCall {
                id: block["id"].as_str().unwrap_or_default().to_owned(),
                name: block["name"].as_str().unwrap_or_default().to_owned(),
                input: block["input"].clone(),
            }),
            _ => {}
        }
    }
    reply.text = reply.text.trim().to_owned();
    reply
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tool_calls_in_both_dialects() {
        let openai = json!({"choices": [{"message": {"content": null, "tool_calls": [
            {"id": "c1", "type": "function", "function": {"name": "search_mail", "arguments": "{\"query\":\"budget\"}"}}
        ]}}]});
        let r = openai_reply(&openai);
        assert_eq!(r.calls[0].name, "search_mail");
        assert_eq!(r.calls[0].input["query"], "budget");

        let anthropic = json!({"content": [
            {"type": "text", "text": "Let me look. "},
            {"type": "tool_use", "id": "tu1", "name": "list_events", "input": {"from": "2026-10-15"}}
        ]});
        let r = anthropic_reply(&anthropic);
        assert_eq!(r.text, "Let me look.");
        assert_eq!(r.calls[0].input["from"], "2026-10-15");
    }

    /// Feeds a recorded stream in chunks of `size` bytes, like a slow network.
    fn replay(kind: ProviderKind, body: &str, size: usize) -> (Vec<String>, Reply) {
        let mut lines = SseLines::default();
        let mut reply = StreamedReply::default();
        let mut pieces = Vec::new();
        for chunk in body.as_bytes().chunks(size) {
            for data in lines.push(chunk) {
                let event: Value = serde_json::from_str(&data).unwrap();
                pieces.extend(reply.add(kind, &event).unwrap());
            }
        }
        (pieces, reply.finish())
    }

    // What OpenAI (and Ollama, Mistral…) send: a tool call whose name and
    // arguments are spread over several events, then the end.
    const OPENAI_TOOL_STREAM: &str = r#"data: {"choices":[{"delta":{"role":"assistant","content":"Let me "}}]}

data: {"choices":[{"delta":{"content":"look."}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"search_mail","arguments":""}}]}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\":"}}]}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"budget été\"}"}}]}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"unread_mail","arguments":""}}]}}]}

data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;

    #[test]
    fn puts_streamed_openai_tool_calls_together() {
        // Every chunk size, down to one byte: the result must not depend on where the network cuts.
        for size in [1, 3, 7, 64, 10_000] {
            let (pieces, reply) = replay(ProviderKind::OpenAi, OPENAI_TOOL_STREAM, size);
            assert_eq!(pieces, ["Let me ", "look."], "chunks of {size}");
            assert_eq!(reply.text, "Let me look.");
            assert_eq!(reply.calls.len(), 2);
            assert_eq!(reply.calls[0].id, "call_a");
            assert_eq!(reply.calls[0].input["query"], "budget été");
            assert_eq!(reply.calls[1].name, "unread_mail");
            assert_eq!(reply.calls[1].input, json!({}));
        }
    }

    #[test]
    fn reads_ollama_arguments_sent_as_an_object() {
        let body = r#"data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"list_events","arguments":{"from":"2026-11-09"}}}]}}]}

"#;
        let (_, reply) = replay(ProviderKind::Ollama, body, 5);
        assert_eq!(reply.calls[0].id, "call_0", "Ollama may leave the id out");
        assert_eq!(reply.calls[0].input["from"], "2026-11-09");
    }

    #[test]
    fn puts_streamed_anthropic_tool_calls_together() {
        let body = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1"}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Checking your week."}}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"list_events","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"from\": \"2026-"}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"11-09\"}"}}

event: message_stop
data: {"type":"message_stop"}

"#;
        for size in [1, 13, 4096] {
            let (pieces, reply) = replay(ProviderKind::Anthropic, body, size);
            assert_eq!(pieces, ["Checking your week."]);
            assert_eq!(reply.calls.len(), 1);
            assert_eq!(reply.calls[0].id, "toolu_1");
            assert_eq!(reply.calls[0].input["from"], "2026-11-09");
        }
    }

    #[test]
    fn a_streamed_error_is_an_error() {
        let mut reply = StreamedReply::default();
        let event = json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}});
        assert_eq!(reply.add(ProviderKind::Anthropic, &event), Err("Overloaded".to_owned()));
    }

    #[test]
    fn ollama_gets_its_openai_path() {
        assert_eq!(openai_base(ProviderKind::Ollama, "http://localhost:11434/"), "http://localhost:11434/v1");
        assert_eq!(openai_base(ProviderKind::Ollama, "http://box:11434/v1"), "http://box:11434/v1");
        assert_eq!(
            openai_base(ProviderKind::OpenAi, "https://api.mistral.ai/v1"),
            "https://api.mistral.ai/v1"
        );
    }

    #[test]
    fn normalizes() {
        let v = normalize(vec![3.0, 4.0]);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
    }
}
