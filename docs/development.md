# Development

How the code is organised and how to work on it. The rules for branches, commits and pull requests are in
[CONTRIBUTING.md](../CONTRIBUTING.md).

## Prerequisites

- **Rust** (stable) — install with [rustup](https://rustup.rs).
  On Windows, rustup asks for the *Visual Studio C++ Build Tools*: accept,
  and tick "Desktop development with C++".
- **Node.js 22+** and **pnpm** — after installing Node, run `corepack enable`.
- An editor with **rust-analyzer** and **Vue** support (VS Code extensions
  `rust-lang.rust-analyzer` and `Vue.volar`).

## Layout

```
havre/
├─ crates/
│  ├─ core/     havre-core   — the logic: mail, agenda, people, tasks, notes, files, AI. Plain Rust, no HTTP.
│  ├─ server/   havre        — HTTP server: JSON API on /api, MCP on /mcp, serves the web UI.
│  └─ cli/      havre-cli    — try the core from a terminal.
├─ web/         Nuxt app in SPA mode → static files served by the Rust server.
├─ desktop/     Havre Desktop (Tauri 2), a window onto your server.
├─ Dockerfile   One image, one process.
└─ docs/        Guides and the roadmap.
```

`havre-core` knows nothing about HTTP or UI, so the server and the CLI both reuse it.

## Day-to-day

Two terminals:

```sh
# 1 — API on :8080 (restart after Rust changes; or `cargo install cargo-watch`
#     then `cargo watch -x "run -p havre-server"`)
cargo run -p havre-server

# 2 — UI on :3000 with hot reload; /api is forwarded to :8080
cd web && pnpm dev
```

Before committing:

```sh
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
cd web && pnpm typecheck && pnpm build
```

The command-line tool reads one account from `.env` — handy to check a connection from the terminal:

```sh
cargo run -p havre-cli -- mail folders
cargo run -p havre-cli -- mail list --folder INBOX --limit 20
cargo run -p havre-cli -- mail show <uid>
```

Gmail works with OAuth2 or an app password (see below).
Listing never changes anything; opening a message in the web UI marks it as
read, like any mail client (`mail show` in the CLI does not).

## Adding an app

An app is three pieces: a `Manifest` in `crates/core/src/apps.rs` (id,
name, icon, page, what it provides and uses), its logic in `havre-core`
with routes in `crates/server/src/`, and a page in `web/app/pages/`. The
dock, the Apps page and Ctrl+K pick it up from the manifest.

## Coding with Claude Code

Set up for it on the development machine (these files are git-ignored: they
stay on your disk):

- `CLAUDE.md` — architecture, commands, conventions (and that Maël is
  learning Rust: explanations welcome);
- `.claude/settings.json` — allowed `cargo` / `pnpm` / `git` read commands,
  `.env` and `data/` blocked, and `cargo fmt` after each edit;
- slash commands: `/check` (fmt, clippy, tests, typecheck, build),
  `/migration <change>`, `/route <METHOD /path …>`, `/explain <file or
  concept>`;
- a `rust-reviewer` subagent to review a change before calling it done;
- `.mcp.json` — Havre's own MCP server while developing: set
  `HAVRE_MCP_TOKEN` (a key from Settings › Connections) and, if not
  `http://localhost:8080/mcp`, `HAVRE_MCP_URL`.

Personal settings go in `.claude/settings.local.json`.

## Learning Rust with this codebase

Read in this order — each file is short and commented:

1. `crates/core/src/error.rs` — one error type, `?` and `thiserror`.
2. `crates/core/src/config.rs` — structs, `impl`, `Result`, env vars.
3. `crates/core/src/mail/summary.rs` — parsing, `Option`, and **tests**
   (`cargo test`).
4. `crates/core/src/mail/folder.rs` — `enum`s, `match`, sorting.
5. `crates/core/src/mail/sanitize.rs` — closures, `Arc`, sharing state safely.
6. `crates/core/src/mail/imap.rs` — `async`/`.await`, TLS, streams.
7. `crates/core/src/store.rs` — SQL with `sqlx`, transactions, migrations.
   `crates/core/src/secrets.rs` — encryption with a key file.
8. `crates/core/src/sync.rs` — background tasks, channels (`mpsc`,
   `broadcast`, `oneshot`, `watch`), `tokio::select!`, shared state with `Arc`.
   `crates/core/src/accounts.rs` — one engine per account, started and stopped.
   `crates/core/src/auth.rs` — password hashing (Argon2), sessions.
   `crates/core/src/agenda/` — calendars: REST APIs (`graph.rs`, `google.rs`),
   WebDAV + XML (`caldav.rs`), iCalendar and repeating events (`ics.rs`),
   recurrence rules and time zones (`repeat.rs`: enums with `serde`, `let`-chains,
   wall-clock time across daylight saving with `chrono-tz`).
   `crates/core/src/compose.rs` — building MIME messages and sending them
   with `lettre`; drafts; `impl` blocks spread over modules.
   `crates/core/src/oauth.rs` — OAuth 2.0 with `reqwest` (HTTP + JSON), a
   trait implementation (`Authenticator`), caching a token behind a `Mutex`.
9. `crates/core/src/people.rs` — SQL over JSON (`json_each`), one query
   serving several apps. `crates/core/src/apps.rs` — `const` data
   (manifests) plus stored choices. `crates/core/src/invitations.rs` — two
   modules (Mail, Agenda) working together. `crates/core/src/notes/markdown.rs`
   — a small hand-written reader: `&str` slicing by bytes, `strip_prefix`,
   `Option` and `?`, and enums with data serialized for the browser.
10. `crates/core/src/ai/` — talking to AI servers: two API dialects behind
   one type (`provider.rs`), streaming responses piece by piece, tool
   calling (`assistant.rs`), embeddings and similarity search (`index.rs`),
   a background task (`index_loop`), running a program and reading its
   output line by line (`claude_code.rs`, `tokio::process`).
   `crates/core/src/mcp.rs` — a small JSON-RPC protocol by hand.
11. `crates/server/src/mail_source.rs` and `api.rs` — Axum handlers, enums
   with data, HTTP errors, Server-Sent Events. `auth.rs` — middlewares,
   cookies. `platform.rs` — small handlers over the core. `ai.rs` —
   streaming a response with `Body::from_stream` and `futures::stream`.

Good first exercises: add a `--unread` flag to `mail list`; show the number
of unread messages next to each folder (IMAP `STATUS`); add `mail archive <uid>`
to the CLI (the core function already exists).
