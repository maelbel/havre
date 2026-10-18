//! Claude Code as the AI: Havre runs the `claude` command line in headless
//! mode (`claude -p`) and reads its answer. It uses the Claude plan you
//! signed in with in Claude Code (run `claude` once, on the machine where
//! Havre runs), so no API key is stored in Havre.
//!
//! How a call works:
//! - the prompt goes to the process's standard input (no length limit, no
//!   quoting problems);
//! - `--output-format stream-json` makes it print one JSON object per line:
//!   text pieces (`stream_event` / `text_delta`) and, at the end, a
//!   `result` saying whether it worked;
//! - Claude Code's own tools (files, shell, web) are switched off
//!   (`--tools ""`), and your settings and projects are not loaded
//!   (`--setting-sources ""`, an empty working directory).
//!
//! For the assistant, Claude Code gets **Havre's tools over MCP** (see
//! `crate::mcp`): it calls back into this server with a one-time token.
//!
//! Setting it up happens in Settings › AI, without a terminal:
//! - [`find`] looks for the `claude` command (PATH, then where the
//!   installers put it), so the "Command" field can stay empty;
//! - [`ClaudeCode::status`] runs `claude --version` and `claude auth status`;
//! - [`LoginFlow`] runs `claude auth login` for you: it reads the sign-in
//!   address the CLI prints, you sign in at Anthropic, paste the code
//!   shown, and Havre types it into the CLI.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::mpsc,
};

use crate::{Error, Result};

/// How to run Claude Code.
#[derive(Debug, Clone)]
pub struct ClaudeCode {
    /// The command (`claude`, or a full path).
    pub command: String,
    /// `sonnet`, `opus`, `haiku`, a full model name, or empty for Claude
    /// Code's default.
    pub model: String,
}

/// Extra options for one run.
#[derive(Debug, Default)]
pub struct Run<'a> {
    /// `--mcp-config` JSON: Havre's tools for the assistant.
    pub mcp_config: Option<&'a str>,
    /// Tools Claude Code may call without asking (`mcp__havre`).
    pub allowed_tools: Option<&'a str>,
    pub max_turns: Option<u32>,
}

impl ClaudeCode {
    /// Runs one request; sends text pieces to `out` as they arrive (if
    /// given) and returns the whole answer.
    pub async fn run(
        &self,
        system: &str,
        prompt: &str,
        options: Run<'_>,
        out: Option<&mpsc::Sender<String>>,
    ) -> Result<String> {
        // An empty folder: nothing (CLAUDE.md, settings) leaks into the run.
        let dir = std::env::temp_dir().join("havre-claude-code");
        std::fs::create_dir_all(&dir)?;

        let mut args: Vec<String> = vec![
            "-p".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--include-partial-messages".into(),
            "--no-session-persistence".into(),
            "--setting-sources".into(),
            String::new(),
            "--tools".into(),
            String::new(),
            "--strict-mcp-config".into(),
            "--system-prompt".into(),
            system.into(),
        ];
        if !self.model.is_empty() {
            args.extend(["--model".into(), self.model.clone()]);
        }
        if let Some(config) = options.mcp_config {
            args.extend(["--mcp-config".into(), config.into()]);
        }
        if let Some(tools) = options.allowed_tools {
            args.extend(["--allowedTools".into(), tools.into()]);
        }
        if let Some(n) = options.max_turns {
            args.extend(["--max-turns".into(), n.to_string()]);
        }

        let mut child = self.spawn(&args, &dir)?;
        // The prompt on standard input, then close it (end of prompt).
        let mut stdin = child.stdin.take().expect("piped stdin");
        let prompt = prompt.to_owned();
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
        let mut stderr = child.stderr.take().expect("piped stderr");
        let stderr_task = tokio::spawn(async move {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text).await;
            text
        });

        let stdout = child.stdout.take().expect("piped stdout");
        let mut lines = BufReader::new(stdout).lines();
        let mut streamed = String::new();
        let mut result: Option<(bool, String)> = None;
        let read = async {
            while let Some(line) = lines.next_line().await? {
                let Ok(event) = serde_json::from_str::<Value>(&line) else { continue };
                match event["type"].as_str() {
                    Some("stream_event") => {
                        let e = &event["event"];
                        if e["type"] == "content_block_delta"
                            && e["delta"]["type"] == "text_delta"
                            && event["parent_tool_use_id"].is_null()
                        {
                            let piece = e["delta"]["text"].as_str().unwrap_or_default();
                            streamed.push_str(piece);
                            if let Some(out) = out
                                && out.send(piece.to_owned()).await.is_err()
                            {
                                // The reader left: stop.
                                return Ok::<bool, std::io::Error>(false);
                            }
                        }
                        // A new answer after a tool call: keep only the last one.
                        if e["type"] == "message_start" && out.is_none() {
                            streamed.clear();
                        }
                    }
                    Some("result") => {
                        let is_error = event["is_error"].as_bool().unwrap_or(false)
                            || event["subtype"].as_str().is_some_and(|s| s != "success");
                        let text = event["result"].as_str().unwrap_or_default().to_owned();
                        result = Some((is_error, text));
                    }
                    _ => {}
                }
            }
            Ok(true)
        };
        // Long questions with tools take time; never more than 5 minutes.
        let finished = tokio::time::timeout(Duration::from_secs(300), read).await;
        let _ = child.start_kill();
        let stderr = stderr_task.await.unwrap_or_default();
        match finished {
            Err(_) => return Err(Error::Ai("Claude Code took too long to answer.".into())),
            Ok(Err(e)) => return Err(Error::Ai(format!("Claude Code: {e}"))),
            Ok(Ok(false)) => return Ok(streamed),
            Ok(Ok(true)) => {}
        }
        match result {
            Some((false, text)) => Ok(if text.is_empty() { streamed } else { text }),
            Some((true, text)) => Err(Error::Ai(explain(&text, &stderr))),
            None => Err(Error::Ai(explain("", &stderr))),
        }
    }

    fn spawn(&self, args: &[String], dir: &Path) -> Result<tokio::process::Child> {
        let program = find(&self.command).ok_or_else(|| not_found(&self.command))?;
        command(&program)
            .args(args)
            .current_dir(dir)
            .spawn()
            .map_err(|e| Error::Ai(format!("Claude Code can't be started ({}): {e}", program.display())))
    }

    /// Installed? Which version? Signed in?
    pub async fn status(&self) -> ClaudeStatus {
        let Some(program) = find(&self.command) else {
            return ClaudeStatus { command: None, version: None, logged_in: false, auth_method: None };
        };
        let run = |args: &'static [&'static str]| {
            let program = program.clone();
            async move {
                let out =
                    tokio::time::timeout(Duration::from_secs(20), command(&program).args(args).output())
                        .await;
                match out {
                    Ok(Ok(out)) if out.status.success() => {
                        Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
                    }
                    _ => None,
                }
            }
        };
        let (version, auth) = tokio::join!(run(&["--version"]), run(&["auth", "status", "--json"]));
        let auth: Value = auth.and_then(|a| serde_json::from_str(&a).ok()).unwrap_or(Value::Null);
        ClaudeStatus {
            command: Some(program.display().to_string()),
            // "2.1.300 (Claude Code)" → "2.1.300"
            version: version.map(|v| v.split_whitespace().next().unwrap_or_default().to_owned()),
            logged_in: auth["loggedIn"].as_bool().unwrap_or(false),
            auth_method: auth["authMethod"].as_str().map(str::to_owned),
        }
    }

    /// `claude auth logout`.
    pub async fn logout(&self) -> Result<()> {
        let program = find(&self.command).ok_or_else(|| not_found(&self.command))?;
        let out = command(&program).args(["auth", "logout"]).output().await?;
        if !out.status.success() {
            return Err(Error::Ai(format!(
                "Claude Code couldn't sign out: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }
}

/// What Settings › AI shows about Claude Code.
#[derive(Debug, Clone, Serialize)]
pub struct ClaudeStatus {
    /// Where the `claude` command was found (`None`: not installed).
    pub command: Option<String>,
    pub version: Option<String>,
    pub logged_in: bool,
    /// `oauth_token` (your Claude plan), `api_key`…
    pub auth_method: Option<String>,
}

/// A `claude` process, set up the same way every time.
fn command(program: &Path) -> Command {
    let mut c = Command::new(program);
    c
        // Started from inside a Claude Code session (development): the
        // child must not think it's nested.
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        // An API key in the environment would bill the API instead of
        // your Claude plan (and hide the sign-in).
        .env_remove("ANTHROPIC_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    c
}

/// Finds the `claude` program: the configured command (a path, or a name
/// looked up in PATH), else where the installers put it.
pub fn find(configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    let name = if configured.is_empty() { "claude" } else { configured };
    // A path: use it if it exists.
    if name.contains('/') || name.contains('\\') {
        let path = PathBuf::from(name);
        return path.is_file().then_some(path);
    }
    // Windows programs have an extension: claude.exe (native installer),
    // claude.cmd (npm).
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd"), name.to_owned()]
    } else {
        vec![name.to_owned()]
    };
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    // Not always in the server's PATH (a service, another shell profile).
    if let Some(home) =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
    {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".claude").join("local"));
        dirs.push(home.join(".npm-global").join("bin"));
        if cfg!(windows) {
            dirs.push(home.join("AppData").join("Roaming").join("npm"));
        }
    }
    if !cfg!(windows) {
        dirs.extend(["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin"].map(PathBuf::from));
    }
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

/// Signing Claude Code in from Havre: `claude auth login` runs in the
/// background, waiting for the code you get at Anthropic.
#[derive(Default)]
pub struct LoginFlow {
    pending: tokio::sync::Mutex<Option<Pending>>,
}

struct Pending {
    child: tokio::process::Child,
    /// Everything the CLI printed (standard output and errors).
    output: Arc<Mutex<String>>,
    started: Instant,
}

impl LoginFlow {
    /// Starts `claude auth login` and returns the address to sign in at.
    pub async fn start(&self, claude: &ClaudeCode) -> Result<String> {
        let mut pending = self.pending.lock().await;
        if let Some(mut old) = pending.take() {
            // A sign-in left half-way: start again.
            let _ = old.child.kill().await;
        }
        let program = find(&claude.command).ok_or_else(|| not_found(&claude.command))?;
        let mut child = command(&program).args(["auth", "login"]).spawn()?;
        let output = Arc::new(Mutex::new(String::new()));
        if let Some(out) = child.stdout.take() {
            tokio::spawn(pump(out, Arc::clone(&output)));
        }
        if let Some(err) = child.stderr.take() {
            tokio::spawn(pump(err, Arc::clone(&output)));
        }
        // The CLI prints the address, then waits for the code. Wait for
        // the prompt (or a moment after the address), 15 seconds at most.
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut url_seen: Option<Instant> = None;
        let url = loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let text = output.lock().unwrap().clone();
            let url = sign_in_url(&text);
            if url.is_some() && url_seen.is_none() {
                url_seen = Some(Instant::now());
            }
            let prompt = text.to_lowercase().contains("paste code");
            if let Some(url) = url
                && (prompt || url_seen.is_some_and(|t| t.elapsed() > Duration::from_millis(1500)))
            {
                break url;
            }
            let exited = child.try_wait()?.is_some();
            if exited || Instant::now() > deadline {
                let _ = child.kill().await;
                let said = text.trim().chars().take(400).collect::<String>();
                return Err(Error::Ai(if said.is_empty() {
                    "Claude Code didn't give a sign-in address.".into()
                } else {
                    format!("Claude Code didn't give a sign-in address: {said}")
                }));
            }
        };
        *pending = Some(Pending { child, output, started: Instant::now() });
        Ok(url)
    }

    /// Types the code from Anthropic's page into the waiting CLI. Returns
    /// whether it worked, and what the CLI said.
    pub async fn complete(&self, code: &str) -> Result<(bool, String)> {
        let mut pending = self.pending.lock().await;
        let Some(mut p) = pending.take() else {
            return Err(Error::Config("No sign-in in progress: click Sign in again.".into()));
        };
        if p.started.elapsed() > Duration::from_secs(600) {
            let _ = p.child.kill().await;
            return Err(Error::Config("This sign-in expired: click Sign in again.".into()));
        }
        let mut stdin =
            p.child.stdin.take().ok_or_else(|| Error::Ai("Claude Code isn't waiting for a code.".into()))?;
        stdin.write_all(format!("{}\n", code.trim()).as_bytes()).await?;
        stdin.flush().await?;
        drop(stdin);
        let status = match tokio::time::timeout(Duration::from_secs(30), p.child.wait()).await {
            Ok(status) => status?,
            Err(_) => {
                let _ = p.child.kill().await;
                return Err(Error::Ai("Claude Code didn't finish signing in in time.".into()));
            }
        };
        // Let the readers catch the last lines.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let said = p.output.lock().unwrap().clone();
        Ok((status.success(), said))
    }

    /// Gives up a sign-in in progress.
    pub async fn cancel(&self) {
        if let Some(mut p) = self.pending.lock().await.take() {
            let _ = p.child.kill().await;
        }
    }
}

/// Copies a process's output into `into`, as it comes.
async fn pump(mut from: impl tokio::io::AsyncRead + Unpin, into: Arc<Mutex<String>>) {
    let mut buf = [0u8; 1024];
    while let Ok(n) = from.read(&mut buf).await {
        if n == 0 {
            break;
        }
        into.lock().unwrap().push_str(&String::from_utf8_lossy(&buf[..n]));
    }
}

/// The first `https://…` address in the CLI's output.
fn sign_in_url(text: &str) -> Option<String> {
    let start = text.find("https://")?;
    let url: String = text[start..].chars().take_while(|c| !c.is_whitespace() && *c != '\u{1b}').collect();
    Some(url)
}

fn not_found(command: &str) -> Error {
    let command = if command.trim().is_empty() { "claude" } else { command.trim() };
    Error::Ai(format!(
        "Claude Code isn't installed where Havre runs (no “{command}” found). Settings › AI shows how to \
         install it."
    ))
}

/// Turns Claude Code's failure into something to act on.
fn explain(result: &str, stderr: &str) -> String {
    let text = if result.trim().is_empty() { stderr.trim() } else { result.trim() };
    let lower = text.to_lowercase();
    if lower.contains("login")
        || lower.contains("log in")
        || lower.contains("api key")
        || lower.contains("auth")
    {
        return format!(
            "Claude Code isn't signed in on the machine running Havre. Run `claude` there once and sign in. ({text})"
        );
    }
    if text.is_empty() {
        return "Claude Code stopped without an answer.".into();
    }
    format!("Claude Code: {}", text.chars().take(400).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_sign_in_address() {
        let out = "Browser didn't open? Use the url below to sign in:\n\nhttps://claude.ai/oauth/authorize?code=true&x=1\n\nPaste code here if prompted > ";
        assert_eq!(sign_in_url(out).as_deref(), Some("https://claude.ai/oauth/authorize?code=true&x=1"));
        assert_eq!(sign_in_url("nothing"), None);
    }

    #[test]
    fn explains_sign_in_problems() {
        assert!(explain("Invalid API key · Please run /login", "").contains("isn't signed in"));
        assert_eq!(explain("", ""), "Claude Code stopped without an answer.");
    }

    /// A fake `claude` (a shell script) printing what the real one prints.
    #[cfg(unix)]
    #[tokio::test]
    async fn reads_streamed_answers() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("havre-fake-claude-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("claude");
        std::fs::write(
            &script,
            r#"#!/bin/sh
cat > /dev/null
echo '{"type":"system","subtype":"init"}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello "}},"parent_tool_use_id":null}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"there"}},"parent_tool_use_id":null}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"Hello there"}'
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cc = ClaudeCode { command: script.to_string_lossy().into(), model: String::new() };
        let (tx, mut rx) = mpsc::channel(8);
        let text = cc.run("sys", "hi", Run::default(), Some(&tx)).await.unwrap();
        drop(tx);
        assert_eq!(text, "Hello there");
        assert_eq!(rx.recv().await.as_deref(), Some("Hello "));

        let missing = ClaudeCode { command: "/nonexistent/claude".into(), model: String::new() };
        let err = missing.run("s", "p", Run::default(), None).await.unwrap_err().to_string();
        assert!(err.contains("isn't installed"), "{err}");
    }
}
