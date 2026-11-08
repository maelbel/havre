# Changelog

All notable changes to Havre are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). The detailed plan is in [docs/ROADMAP.md](docs/ROADMAP.md).

## [Unreleased]

### Added

- **Off-site copies of the backups** (Settings › Backups › Off-site copy): after each backup, Havre seals it with
  the recovery key (XChaCha20-Poly1305, chunked and authenticated) and sends it to a WebDAV folder (Nextcloud, a
  NAS) or an S3-compatible bucket (Backblaze B2, MinIO, Scaleway, AWS), keeping the last ones there. *Test the
  connection*, *Copy off-site* on each backup, retries, an orange status after a day of failures.
  `havre restore <file>.havre --key …` restores one on a new machine.

### Changed

- Mail's sidebar: one section per account (an accordion), its Inbox first and then its folders, instead of the
  inboxes on one side and the current account's folders on the other.
- While you read an email, its subject and sender stay at the top (smaller once you scroll).
- Everything clickable shows the pointer cursor.
- Notifications (toasts) appear in the bottom right corner.

### Fixed

- An HTML email had its own scrollbar inside the reader's (it was 2 px taller than its frame): it now scrolls with
  the reader, even when the email fixes its own height.

## [0.1.0] - 2026-11-07

The first release.

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
- **Drafts on the mail server**: drafts are also saved in each account's Drafts folder (other mail apps see them),
  and drafts started elsewhere can be finished in Havre.
- **Repeating events** in every calendar (Microsoft, Google, CalDAV, Havre's): every day, week, month or year,
  ending on a date or after a number of times, kept in your time zone; change or delete only one, this and the
  following ones, or all.
- **Ctrl+K** palette, **MCP** endpoint for AI apps (Claude Code and others), **Havre Desktop** (Windows, macOS,
  Linux), nightly **backups** with a recovery key, **Docker** image.
- A **new look**, from Mael Belliard's design system: Ink, Slate, Limestone and brass, Jost with Josefin Sans and
  JetBrains Mono (bundled), corners of 4 px at most, no shadows; primary buttons in ink or limestone, brass for
  links, focus and small marks. Same in Havre Desktop's connect page.
- **Releases** from a signed tag: Linux downloads (x86_64, arm64 for the Raspberry Pi), the image on
  `ghcr.io/maelbel/havre` (amd64, arm64) and the Desktop installers, with checksums and build attestations.

### Changed

- **On a phone**, Havre is laid out for the thumb: a bottom dock (Today, your first apps, *More*), search at the top
  of each screen and full screen, Mail's folders in a sheet with a *Write* button, a shorter message toolbar,
  Settings as a menu, the Agenda one day at a time under a week strip, People's lists picked with chips, and the
  views of Tasks, Notes, Files and AI as chips that scroll. Wider screens are unchanged.
- The web UI is built on [Nuxt UI](https://ui.nuxt.com) (buttons, fields, menus, dialogs, toasts, the Ctrl+K
  palette, resizable panes), themed with Havre's colours; icons and fonts are still bundled, nothing is fetched.

### Fixed

- A bad `HAVRE_BIND` (a name like `localhost`, quotes, a Windows line ending) stopped Havre with "invalid socket
  address syntax"; spaces and line endings are now ignored, and anything else is explained.
- Settings › Accounts is shown as the current section when `/settings` is opened directly.
- Opening `/login` while signed in, or with a `next` address pointing to another site, now goes to a Havre page.

### Security

- Mail passwords, refresh tokens and API keys encrypted at rest; session and MCP tokens stored hashed.
- Email HTML sanitised and sandboxed; the assistant only proposes changes, confirmed by the user.
- The web UI loads nothing from other servers.

[Unreleased]: https://github.com/maelbel/havre/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/maelbel/havre/releases/tag/v0.1.0
