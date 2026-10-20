# Changelog

All notable changes to Havre are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). The detailed plan is in [docs/ROADMAP.md](docs/ROADMAP.md).

## [Unreleased]

First version, not released yet (0.1.0).

### Added

- **Mail**: several IMAP accounts (password or Microsoft / Google sign-in), live sync (IMAP IDLE), local search and
  search by meaning, writing with SMTP (drafts, attachments, recipient suggestions), All inboxes, stars, labels,
  snooze, actions on several messages, undo.
- **Agenda**: Microsoft 365, Google Calendar and CalDAV calendars plus Havre's own; day, week and month views;
  invitations answered from Mail or the Agenda.
- **People**: built from senders and meeting guests, with each person's mail, meetings, notes, tasks and files.
- **Today**: today and tomorrow, invitations waiting for an answer, unread mail, tasks due.
- **Apps you add**: Tasks (Microsoft To Do, Google Tasks, CalDAV, reminders), Notes (Markdown, linked to meetings
  and people), Files (every attachment, copies and versions), AI (local with Ollama or a provider you choose; the AI
  tab, summaries, drafts, a morning brief, an assistant that proposes and never acts alone).
- **Repeating events** in every calendar (Microsoft, Google, CalDAV, Havre's): every day, week, month or year,
  ending on a date or after a number of times, kept in your time zone; change or delete only one, this and the
  following ones, or all.
- **Ctrl+K** palette, **MCP** endpoint for AI apps (Claude Code and others), **Havre Desktop** (Windows, macOS,
  Linux), nightly **backups** with a recovery key, **Docker** image.

### Changed

- The web UI is built on [Nuxt UI](https://ui.nuxt.com) (buttons, fields, menus, dialogs, toasts, the Ctrl+K
  palette, resizable panes), themed with Havre's colours; icons and fonts are still bundled, nothing is fetched.

### Fixed

- Opening `/login` while signed in, or with a `next` address pointing to another site, now goes to a Havre page.

### Security

- Mail passwords, refresh tokens and API keys encrypted at rest; session and MCP tokens stored hashed.
- Email HTML sanitised and sandboxed; the assistant only proposes changes, confirmed by the user.
- The web UI loads nothing from other servers.

[Unreleased]: https://github.com/maelbel/havre/commits/main
