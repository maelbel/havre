<div align="center">

<img src="docs/assets/logo.svg" alt="Havre logo" width="88" height="88">

# Havre

**Your mail, calendars and everyday apps — self-hosted, private, and working together.**

[![CI](https://github.com/maelbel/havre/actions/workflows/ci.yml/badge.svg)](https://github.com/maelbel/havre/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/maelbel/havre?sort=semver)](https://github.com/maelbel/havre/releases/latest)
[![Docker image](https://img.shields.io/badge/docker-ghcr.io%2Fmaelbel%2Fhavre-2496ED.svg?logo=docker&logoColor=white)](docs/installation.md#with-the-ready-made-image)
[![License: AGPL v3](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024-orange.svg?logo=rust)](https://www.rust-lang.org)

[Installation](docs/installation.md) · [User guide](docs/user-guide.md) · [Roadmap](docs/ROADMAP.md) ·
[Contributing](CONTRIBUTING.md)

<picture>
  <source media="(prefers-color-scheme: light)" srcset="docs/assets/screenshots/mail-light.webp">
  <img src="docs/assets/screenshots/mail-dark.webp" alt="Havre Mail: the inbox, grouped by day, with a meeting invitation open">
</picture>

</div>

## About

Havre is a self-hosted home for the apps you use every day. It brings all your mail accounts and calendars into one
calm interface, and lets the apps share what they know: the people you write to, the meetings you have, the files you
exchange. It runs on your own server as a single Rust binary with a SQLite database, talks directly to your mail and
calendar providers, and loads nothing from third-party servers.

> [!NOTE]
> Havre is young (0.x). It is used day to day, but things still change between releases: read the
> [changelog](CHANGELOG.md) before updating, and keep backups (Havre makes nightly ones for you). Grab the
> [latest release](https://github.com/maelbel/havre/releases/latest).

### Why Havre

- **Yours, end to end** — one Rust binary and one SQLite file on your own machine; it talks straight to your mail
  and calendar providers and loads nothing from third-party servers. Credentials are encrypted at rest.
- **Apps that know each other** — a person links to your mail with them, your meetings, notes, tasks and files;
  an invitation is answered from Mail; a note becomes tasks.
- **Calm by design** — one inbox for every account, one agenda for every calendar, a Today page, and Ctrl+K to
  reach anything.
- **AI if you want it, never in charge** — off unless you add it, local with Ollama if you prefer, and it only
  *proposes*: nothing is sent, created or deleted without your click.

## Features

- **Mail**
  - Several accounts in one inbox: Microsoft 365, Gmail and any [IMAP](https://www.rfc-editor.org/rfc/rfc9051) server,
    with OAuth2 sign-in where the provider requires it
  - New mail pushed in a second (IMAP IDLE), instant local search, search by meaning with AI
  - Stars, labels, snooze, actions on several messages, undo
  - Writing with drafts saved as you type, also in the server's Drafts folder, attachments and recipient
    suggestions
- **Agenda**
  - Microsoft 365, Google Calendar and [CalDAV](https://www.rfc-editor.org/rfc/rfc4791) calendars side by side, plus
    Havre's own
  - Day, week and month views; repeating events, kept in your time zone; meeting invitations answered straight
    from Mail
- **People and Today**
  - Everyone you write to or meet, with your shared mail, meetings, notes, tasks and files — no address book to keep
  - A daily overview: meetings, invitations waiting for an answer, unread mail, tasks due
- **Apps you add**
  - **Tasks** — synced with Microsoft To Do, Google Tasks and CalDAV, with reminders and quick add (“Call Thomas fri
    10:00”)
  - **Notes** — Markdown notes linked to meetings, people and emails
  - **Files** — every attachment in your mail, with copies and versions grouped
- **AI, entirely optional**
  - Summaries, reply drafts, a morning brief and an assistant that answers from your mail and calendar
  - Runs locally with [Ollama](https://ollama.com), or with OpenAI-compatible providers, Anthropic or Claude Code
  - Only *proposes* actions: nothing is sent, created or changed without your confirmation
- **Made for self-hosting**
  - Docker image, nightly backups with a recovery key, credentials encrypted at rest
  - Havre Desktop for Windows, macOS and Linux (tray, notifications, default mail app), and an installable web
    app for your phone with push notifications
  - An [MCP](https://modelcontextprotocol.io) endpoint so AI tools can use your data, with your permission
  - A Ctrl+K command palette, keyboard shortcuts, dark and light themes

## Screenshots

<table>
  <tr>
    <td width="33%"><img src="docs/assets/screenshots/today.webp" alt="Today: the day's meetings, a short brief, tasks due and unread mail"></td>
    <td width="33%"><img src="docs/assets/screenshots/agenda.webp" alt="Agenda: the week view, with an invitation found in mail"></td>
    <td width="33%"><img src="docs/assets/screenshots/people.webp" alt="People: a contact's page with your mail, files, tasks and notes together"></td>
  </tr>
  <tr>
    <td align="center"><sub>Today</sub></td>
    <td align="center"><sub>Agenda</sub></td>
    <td align="center"><sub>People</sub></td>
  </tr>
  <tr>
    <td width="33%"><img src="docs/assets/screenshots/tasks.webp" alt="Tasks: upcoming to-dos with dates and the people they are about"></td>
    <td width="33%"><img src="docs/assets/screenshots/notes.webp" alt="Notes: a Markdown note for a meeting, with to-dos that become tasks"></td>
    <td width="33%"><img src="docs/assets/screenshots/files.webp" alt="Files: every attachment from your mail, versions grouped, with a preview"></td>
  </tr>
  <tr>
    <td align="center"><sub>Tasks</sub></td>
    <td align="center"><sub>Notes</sub></td>
    <td align="center"><sub>Files</sub></td>
  </tr>
  <tr>
    <td width="33%"><img src="docs/assets/screenshots/command-palette.webp" alt="Ctrl+K: people, notes, files and mail found as you type"></td>
    <td width="33%"><img src="docs/assets/screenshots/ai.webp" alt="AI: the assistant answers from your notes and tasks and prepares actions you confirm"></td>
    <td width="33%" align="center"><img src="docs/assets/screenshots/phone.webp" alt="Havre on a phone: the Today page as an installed web app" width="62%"></td>
  </tr>
  <tr>
    <td align="center"><sub>Ctrl+K</sub></td>
    <td align="center"><sub>AI, with your click</sub></td>
    <td align="center"><sub>On your phone</sub></td>
  </tr>
</table>

<sub>Screenshots use made-up people and data.</sub>

## Getting started

You need Docker with Compose:

```sh
git clone https://github.com/maelbel/havre.git
cd havre
cp .env.example .env
docker compose up -d --build
```

Open <http://localhost:9988>, choose your Havre password and add a mail account in **Settings**. Most providers only
need an email address and a password; Microsoft 365 and Google sign-in need a [one-time setup](docs/providers.md).
To look around without a mailbox, set `HAVRE_DEMO=1` in `.env`.

Building from source, configuration, HTTPS and backups are covered in the [installation guide](docs/installation.md).

## Documentation

- [Installation](docs/installation.md) — Docker, from source, configuration, HTTPS, backups and restore
- [Providers](docs/providers.md) — connecting Microsoft 365 / Outlook and Gmail / Google Workspace
- [User guide](docs/user-guide.md) — every app, keyboard shortcuts, AI, Claude Code and MCP
- [Havre Desktop](desktop/README.md) — installers, notifications, default mail app
- [Development](docs/development.md) — code layout, local development, learning Rust with the codebase

## Architecture

```
  Browser / phone (PWA)        Havre Desktop (Tauri)        AI tools (MCP clients)
            └──────────────────────────┼─────────────────────────────┘
                                       ▼
              havre (Axum): JSON API /api · MCP /mcp · the Nuxt UI
                                       ▼
              havre-core: sync engines, agenda, people, tasks, notes, files, AI
                 │                                │
                 ▼                                ▼
          SQLite (one file)        IMAP / SMTP · Microsoft Graph · Google · CalDAV
```

The server only parses requests; all the logic lives in `havre-core`, plain Rust with no HTTP. The code is written
to be read by someone learning Rust — see [Development](docs/development.md).

## Built with

[Rust](https://www.rust-lang.org) · [Axum](https://github.com/tokio-rs/axum) · [Tokio](https://tokio.rs) ·
[sqlx](https://github.com/launchbadge/sqlx) and SQLite · [Nuxt](https://nuxt.com), [Vue](https://vuejs.org) and [Nuxt UI](https://ui.nuxt.com) ·
[Tauri](https://tauri.app)

## Roadmap

Mail, Agenda, People, Today, Tasks, Notes, Files, AI, Havre Desktop and off-site backups are built. Next up:
**several users on one Havre** — every piece of data already has an owner; adding family members and signing in
with Authentik (OpenID Connect) come next. See the full [roadmap](docs/ROADMAP.md).

## Contributing

Contributions, bug reports and ideas are welcome. Please read the [contributing guide](CONTRIBUTING.md) (setup,
branches, Conventional Commits) and the [code of conduct](CODE_OF_CONDUCT.md). Notable changes are listed in
the [changelog](CHANGELOG.md).

## Security

Found a vulnerability? Please report it privately as described in the [security policy](SECURITY.md) — not in a public
issue.

## License

Havre is free software, released under the [GNU Affero General Public License v3.0 or later](LICENSE).
