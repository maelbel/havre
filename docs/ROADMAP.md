# Roadmap

Each step teaches something and leaves the app working.

## 0 · Foundations ✅
- Cargo workspace: `havre-core`, `havre-server`, `havre-cli`.
- Nuxt SPA served by the Rust binary, design tokens from the mockup.
- IMAP over TLS: list the latest INBOX messages (CLI + `/api/mail/recent`).
- Demo mode, Docker image.

## 1 · Read mail for real ✅
- ✅ Full message: text/HTML body (sanitized, sandboxed), embedded images, attachments.
- ✅ Folders with special roles (Inbox, Sent…), including French and UTF-7 names.
- ✅ Mark as read / unread; remote images blocked by default.
- ✅ Archive, delete (Trash, then permanent), move — `MOVE`, or `COPY` + expunge on older servers.
- ✅ Pagination ("load older") and server-side search (IMAP `SEARCH`).
- Later: ~~undo after archive/delete, selecting several messages at once~~ (step 8).
- *Learned:* MIME with `mail-parser`, HTML sanitizing (`ammonia`), UIDs vs sequence numbers.

## 2 · Store and sync ✅
- ✅ SQLite with `sqlx` + migrations: folders, headers, cached message bodies.
- ✅ Background sync: new mail, flags, deletions, UIDVALIDITY resets, newest-first backfill.
- ✅ IMAP IDLE: new mail pushed in ~1 s; live UI updates with Server-Sent Events.
- ✅ Unread counts per folder, sync status, instant local search.
- ✅ Several accounts, added in Settings (provider presets, login tested first); passwords encrypted.
- ✅ Body cache size limit (`HAVRE_CACHE_MB`).
- Later: CONDSTORE/QRESYNC for very large mailboxes; ~~a unified inbox across accounts~~ (*All inboxes*, step 6b).
- *Learned:* `sqlx`, background tasks with `tokio::spawn`, channels, SSE.

## 3 · Login, then write mail ✅
- ✅ OAuth2 sign-in for Microsoft 365 / Outlook.com and Gmail / Google Workspace (window like Thunderbird, PKCE, XOAUTH2).
- ✅ **Login for Havre itself** (single owner): first-run password, Argon2, session cookies, throttling, Settings › Security.
- Later: several users, passkeys.
- ✅ Compose, reply, reply all, forward with SMTP (`lettre`): password or OAuth2 (`XOAUTH2`), copy filed in Sent, `\Answered` flag.
- ✅ Drafts saved as you type, attachments, recipient suggestions; SMTP settings and sender name per account.
- Later: rich text editor, signatures, drafts synced to the server's Drafts folder, undo send, send later.

## 4 · Agenda ✅
- ✅ Day, week and month views; create, edit, move between calendars, delete; guests.
- ✅ Microsoft 365 (Graph) and Google Calendar through the mail account's OAuth2 sign-in; CalDAV servers.
- ✅ Local copy kept in sync (5 min, live updates); Havre's own local calendar.
- ✅ `.ics` invitations detected in mail → "Add to Agenda".
- ✅ Reply to invitations (accept / maybe / decline) and a shared People list with Mail: see step 5.
- Later: create repeating events, drag to move, reminders.
- *Learned:* REST APIs, WebDAV/XML, iCalendar and `RRULE` expansion, time zones.

## 5 · App platform ✅
- ✅ App manifests (id, name, icon, route, what each app *provides* and *uses*); App library page; the dock
  comes from the server, apps can be hidden and reordered.
- ✅ **Mail × Agenda:** an invitation in Mail shows a card (time, conflicts) with Accept / Maybe / Decline —
  answered through Microsoft or Google when the meeting is in that calendar, otherwise by emailing the
  organizer a standard iCalendar reply (and adding it to your calendar). Pending invitations in the Agenda
  and on Today, answered there too.
- ✅ **People:** built from mail senders and meeting guests (no address book to fill in); a page per person
  with recent mail and meetings; suggestions for recipients and guests.
- ✅ **Today:** home page with today / tomorrow, invitations waiting for an answer, unread mail from every
  inbox.
- Later: third-party apps (manifest + permissions, sandboxed), an action registry (for the AI step), types
  shared Rust → TypeScript with `ts-rs`.
- *Learned:* SQL over JSON (`json_each`), iTIP (RFC 5546) replies, designing shared contracts between modules.

## 6 · AI (optional, off by default) ✅
- ✅ Providers: Ollama (local), any OpenAI-compatible API (OpenAI, Mistral, Groq, OpenRouter, LM Studio…),
  Anthropic. Chosen in Settings › AI; API key encrypted; streaming answers.
- ✅ Summaries of a message (S), reply drafts from a short instruction, a morning brief on Today.
- ✅ Search by meaning (embeddings, indexed in the background, kept in SQLite).
- ✅ Ctrl+K / ⌘K command palette (go to, actions, people, mail) with the assistant inside: it uses the apps'
  data as tools and only *proposes* events and emails — you confirm.
- ✅ Claude Code: as an AI provider (`claude -p`, your Claude plan), Havre as an MCP server for Claude Code and
  other AI apps (tokens in Settings › Claude Code), and the repo set up for coding with it (`CLAUDE.md`,
  `.claude/`).
- Later: thread-level summaries, more tools (move/archive mail, answer invitations).
- *Learned:* traits vs. enums for providers, HTTP streaming (SSE in, chunked text out), tool calling,
  embeddings and cosine similarity, prompt injection and why actions need confirmation.

## 6b · Matching the design ✅
Every screen rebuilt to the design (dock, Today, Mail, Agenda, Apps, Settings, the assistant).
- ✅ Mail: All inboxes, previews, attachment and invitation chips (digested from the body, prefetched),
  day groups, Unread / Invites filters, "n of N", reply bar, the sender's card (next meetings,
  conversations, shared files).
- ✅ Agenda: "Week N", New event in the toolbar, Found in Mail (invitations still to answer or add),
  event panel with People, Related emails and Files.
- ✅ Apps: status card (version, memory, data), installed / add to your dock.
- ✅ Settings split in pages; Settings › AI assistant: where the model runs, what it can read, brief time,
  MCP switch, activity log. The assistant proposes moves too, checks conflicts, and logs what you confirm.
- Not built yet (in the design, needing new features): the Files app (Notes came with step 11); summaries of whole threads; an Attachments read permission for the AI.
  (Starred, Labels and Snooze came with step 8, backups with step 9, Tasks with step 10.)
- *Learned:* digesting MIME once and storing what lists need, cross-account keys in the UI, ISO week numbers.

## 7 · Desktop ✅
- ✅ Tauri 2 app (`desktop/`) that connects to your server: a "Connect to your Havre" page, the address
  remembered, the server's pages in the window (other sites in the browser, attachments to Downloads).
- ✅ Tray with the unread count, New message, Open at login, Change server; closing keeps it in the tray.
- ✅ New-mail notifications when the window isn't in front.
- ✅ `mailto:` handler (single instance, `?compose=mailto:` in the web app), default-mail-app registration on
  Windows (installer hooks), Linux (.desktop + xdg-mime) and macOS.
- ✅ Sign-ins with Google / Microsoft in the system browser.
- ✅ Installers built by GitHub Actions for Windows (NSIS), macOS (Apple silicon and Intel) and Linux
  (deb, rpm, AppImage).
- Later: code signing (Windows) and notarization (macOS), auto-update, clicking a notification opens the
  message, the Windows taskbar badge.
- *Learned:* Tauri's commands and capabilities (permissions granted at run time to one origin), OS
  integration per platform (`cfg!`), a separate Cargo workspace for a crate with system dependencies.

## 8 · Organizing mail ✅
- ✅ Starred: the IMAP `\Flagged` flag, synced both ways; a Starred list across accounts.
- ✅ Labels: Havre's own, kept by `Message-ID` so they follow a message when it moves; a list per label;
  create from the label menu, edit (rename, color, delete) from the sidebar.
- ✅ Snooze: away from the folder until a time, then back unread and on top of All inboxes (a server loop
  wakes them, so it works with no window open); a Snoozed list.
- ✅ Several messages at once (tick, Ctrl/⌘-click, Shift-click, `x`): archive, delete, star, label, snooze,
  move, read/unread.
- ✅ Undo after archive, move or delete (found again by `Message-ID` in the folder they went to).
- Later: labels mirrored to IMAP keywords where the server allows them, Gmail labels as Havre labels,
  snoozing a whole conversation.
- *Learned:* IMAP flags and `UID STORE`, `UID SEARCH HEADER`, a key that survives moves (`Message-ID`) vs
  one that doesn't (UID), background loops holding a `Weak` reference, optimistic UI with rollback.

## 9 · Backups & safety ✅
- ✅ Nightly backups of the database (`VACUUM INTO`, checked with `integrity_check`, written then renamed),
  the last N kept; time, count and on/off in Settings › Backups; "Back up now", download, delete.
- ✅ Backups leave out what can be rebuilt (mail bodies, search index, AI cache, sessions) and the key.
- ✅ Recovery key: `havre key` prints it; a fingerprint in the page and in each backup tells which key a
  backup needs. Never sent to the browser.
- ✅ `havre backup` / `havre restore <file> [--key …] [--key-mismatch-ok]`; the old database is kept.
- ✅ Health: a Backup line in the Apps status card (orange when the last one failed or is old).
- ✅ Exports: each calendar as `.ics`.
- Later: copying backups off-site (S3, WebDAV) from Havre, encrypted backup files, restoring from the page,
  exporting People as vCard.
- *Learned:* SQLite online copies (`VACUUM INTO`), atomic writes (temp file + rename), Unix permissions
  from Rust, a small CLI with plain `std::env::args`, scheduling in local time with `chrono`.

## 10 · Tasks ✅
- ✅ A Tasks app (in the library, added to the dock in one click), designed first (desktop, light, phone, and in
  the other apps): views (Today, Upcoming, From Mail, No date, Done), lists by account, a details panel.
- ✅ Two-way sync with Microsoft To Do (Graph), Google Tasks and CalDAV `VTODO`, plus Havre's own lists; what a
  provider can't keep (the due time, Google's reminders) stays in Havre. Sync on/off per account.
- ✅ Quick add that understands days, times and `#list`, in English and French (a small parser, tested).
- ✅ Links Havre keeps: the email (by `Message-ID`, found again after a move), the person, the meeting.
- ✅ In the other apps: a Task button and `T` in Mail, tasks on their day in the Agenda, *Due today* and the brief in
  Today, a person's tasks in People, `task …` and task search in Ctrl+K.
- ✅ Reminders: rung by the server, shown as a toast and a notification (browser, Havre Desktop).
- Later: steps inside a task (To Do's checklist, Google's subtasks), repeating tasks, CalDAV reminders (`VALARM`),
  To Do's "linked email" for tasks made in Outlook, ~~the assistant proposing tasks~~ (step 13).
- *Learned:* `BEGIN IMMEDIATE` (SQLite's write lock up front), `Option` chains and `let … else` in a hand-written
  parser, provider differences behind one small interface (`Keeps`), dates as days vs moments.

## 11 · Notes ✅
- ✅ A Notes app (in the library, added to the dock in one click), designed first (desktop, light, phone, and in the
  other apps): views (All, Pinned, Meetings, People), the people you have notes about, the next meetings.
- ✅ Markdown read by Havre into blocks (headings, lists, to-dos, quotes, code, links), drawn by the page — no HTML
  from a note ever reaches the browser. Read / Edit, saved as you type, pin, download as `.md`.
- ✅ Links: one meeting (by iCalendar UID, per occurrence for repeating ones), any number of people, one email (by
  `Message-ID`).
- ✅ To-dos: tick a `- [ ]` line; *Make a task* (linked to the meeting and person), ticked with the line.
- ✅ In the other apps: *Your notes* and *Take notes* in the Agenda's event panel (the meeting's title, people and
  headings), Today's meetings, the *Note* button and `N` in Mail, a person's notes in People, `note …` and note search
  in Ctrl+K, a count on the App library card.
- Later: a search index for notes, attaching any meeting
  or email from the note itself, images in notes, ticking a task in Tasks ticking its line back.
- *Learned:* a hand-written Markdown reader (byte positions in UTF-8 `&str`, `strip_prefix`, `?` on `Option`), enums
  with data as tagged JSON (`#[serde(tag = "kind")]`), `ON DELETE CASCADE` across tables, saving as you type
  (debounce, flush on leave).

## 12 · AI as an app, and the AI tab ✅
- ✅ Redesigned first (the canvas: the App library with AI to add, Today and Mail with and without it, Settings ›
  Your apps › AI, a board comparing each AI part with and without the app).
- ✅ AI is an app in the library (`ai` manifest): it's on while it's in the dock. The old on/off switch is gone
  (whoever had AI on keeps it, as an installed app). Without it: no AI tab, brief, summaries, drafts or *Ask* in
  Ctrl+K, no Settings › AI, and nothing runs in the background.
- ✅ Connections (MCP) moved to its own Settings page: other AI apps can use Havre without the AI app.
- ✅ The AI tab: conversations kept in the database (turns, with each answer's steps, citations and proposals),
  grouped by day, renamed, deleted; the cited email opened on the right; proposals confirmed per answer and
  remembered as done; a phone layout.
- ✅ Ctrl+K chats are kept as conversations too: *Continue in the AI app →*. Answers and proposals are drawn by the
  same code in both (`useAssistant.ts`, `AiPlan.vue`).
- Later: attachments as a permission, streaming answers word by word
  in the AI tab, searching conversations, deleting conversations when the app is removed.
- ✅ Settings › Your apps: a page for every app in the dock — its settings (Tasks' sync moved there from Accounts),
  or where they are, plus what it shares, works with, keeps, where its data lives, its keys.
- ✅ Resizable panes on wide screens (Reka UI's Splitter, styled with Havre's tokens): Mail, Notes, the AI tab,
  Tasks, People; keyboard, double-click to reset, the left column folds, widths remembered per browser. Designed
  first (`Splitters` board).
- *Learned:* a value computed in SQL from another table (a subquery for `installed`), JSON in a column
  (`serde_json::Value`), a transaction for two rows that belong together, moving shared UI into a composable and a
  component. A Vue `provide`/`inject` pair (SplitGroup → its panes and
  handles) and a `shallowReactive` Map.

## 13 · Notes and tasks in the assistant ✅
- ✅ Designed first (`AINotesTasks` board: an answer citing a note and a task, task and note proposals, the
  settings, the MCP tools).
- ✅ Tools: `search_notes` (by words or person; the best partial match when not every word is in a note),
  `read_note` (the text, and open to-dos with their line), `list_tasks` (to do or done, due by a day, by person).
  Citations `[n1]` and `[t4]`, shown as cards that open the note or the task.
- ✅ Proposals, confirmed by you: `propose_task` (from scratch, or from a note's to-do — then made like the note's
  *Make a task*, linked and ticked together) and `propose_note`.
- ✅ Settings › AI › What it can read: Notes and Tasks (on by default, migration `0016`), shown — and the tools given
  to the model — only while that app is in the dock. MCP gets the three read tools.
- ✅ Fix: Today said *Inbox zero* while the dock showed unread mail — snoozed unread messages were still counted in
  the inbox's counter (sidebar and dock). They're left out until they come back.
- Later: changing a note or a task from the assistant (append to a note, tick a task), notes in the search index.
- *Learned:* an async closure in a test (`async |…|`), `sort_by_key` with `std::cmp::Reverse`, scoring partial
  matches, keeping the model's prompt honest (it's only told about what it may use).

## 14 · Files ✅
- ✅ A Files app (in the library, added to the dock in one click), designed first (desktop, light, phone, and in the
  other apps).
- ✅ Every attachment of your mail in one list, received and sent: newest first by month, filters by kind, by
  person, Received / Sent, search. The same file in several emails shows once (*Also in*); versions of a file are
  linked (`version_key`: `devis-v2.pdf` ↔ `devis.pdf`). Images have a preview; download goes through Mail.
- ✅ Files stay on the mail server: Havre keeps the list only. With Files in the dock, the sync engines read older
  Inbox and Sent mail once (200 a pass) to note their attachments, without caching the emails. How far back:
  Settings › Your apps › Files (2 000, 10 000, everything; migration `0017`), with a progress bar.
- ✅ In the other apps: *All files from …* in People, *An earlier version came … · Compare in Files* under an email's
  attachments, a Files section in Ctrl+K. Without the app, none of it shows and nothing is read in the background.
- Later: the assistant finding files (a `search_files` tool), files from Agenda invitations, attaching a file from
  Files when writing an email, previews for PDFs.
- *Learned:* SQL over JSON with `json_each` (one row per attachment), a partial index (`WHERE files IS NOT NULL`),
  grouping in Rust with a `HashMap` keyed by a tuple, `LIMIT -1` in SQLite, background work that only keeps what it
  needs.
