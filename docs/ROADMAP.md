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
- Later: ~~rich text editor~~ (step 25), ~~signatures, undo send, send later~~ (step 24). (Drafts on the server: step 17.)

## 4 · Agenda ✅
- ✅ Day, week and month views; create, edit, move between calendars, delete; guests.
- ✅ Microsoft 365 (Graph) and Google Calendar through the mail account's OAuth2 sign-in; CalDAV servers.
- ✅ Local copy kept in sync (5 min, live updates); Havre's own local calendar.
- ✅ `.ics` invitations detected in mail → "Add to Agenda".
- ✅ Reply to invitations (accept / maybe / decline) and a shared People list with Mail: see step 5.
- Later: drag to move, reminders. (Repeating events: step 16.)
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
- Later: restoring from the page, exporting People as vCard. (Off-site copies: step 18.)
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
  Files when writing an email, ~~previewing files (PDFs, text, audio and video) in Files and under an email's
  attachments~~ (step 27).
- *Learned:* SQL over JSON with `json_each` (one row per attachment), a partial index (`WHERE files IS NOT NULL`),
  grouping in Rust with a `HashMap` keyed by a tuple, `LIMIT -1` in SQLite, background work that only keeps what it
  needs.

## 15 · The whole UI on Nuxt UI ✅
- ✅ Nuxt UI 4 (on Reka UI and Tailwind CSS 4) wearing Havre's look: its colour, text, border and radius variables
  point at Havre's tokens, dark first, light too; a few components tuned in `app.config.ts`. Approved first on the
  canvas from real screenshots (*Nuxt UI · Havre theme*).
- ✅ Every control moved: buttons, fields, selects, date and time fields (`UInputDate`, `UInputTime`, `UCalendar`),
  switches, checkboxes, radio cards, segmented views (`UTabs`), menus (`UDropdownMenu`, `UPopover`), dialogs
  (`UModal`: the event editor, labels, activity), toasts (`useToast`), badges, key hints, the Settings menu
  (`UNavigationMenu`), the AI composer (`UChatPrompt`) and Ctrl+K (`UCommandPalette`). Resizable panes on
  `USplitter` (`SplitView`). Page layouts stay Havre's own.
- ✅ Havre's icons are one list (`web/icons.ts`) used by `HIcon` and by Nuxt UI (`i-havre-*`), including Nuxt UI's
  own chevrons, close and check. Nothing is fetched from other servers (no Iconify API, no Google Fonts): checked
  in the browser tests.
- ✅ Fixes: signing in from `/login` stayed on the sign-in page (the server answers with `/login/`); `web/dist` and
  `Claude outputs/` kept out of git.
- Later: tooltips (`UTooltip`) instead of `title` attributes; recipient chips on `UInputTags`.
- *Learned:* CSS cascade layers (unlayered rules beat any layer, so plain element rules go in `@layer base`),
  Tailwind's `sm:` classes winning over a plain one, an Iconify icon collection built from code, a design system
  mapped onto another one's variables.

## 16 · Repeating events ✅
- ✅ The editor's *Repeat* row (Nuxt UI radio cards, checkbox days, number fields): every day / week (days) / month
  (the same date, or "the third Monday", "the last Friday") / year, every N, ending never, on a date or after N
  times, with the rule in a sentence. The event panel says how it repeats; the week and month views show a ↻.
- ✅ Changing or deleting an occurrence: *only this event*, *this and the following ones* (the series is ended the
  day before, a new one starts, "after N times" keeps counting) or *all events* (the series moves as the edited
  occurrence did). A dialog, a bottom sheet on phones.
- ✅ In Microsoft 365 (the series master's `recurrence`), Google Calendar (`RRULE` lines), CalDAV and Havre's own
  calendar (one iCalendar implementation: modified occurrences, `EXDATE`, a rule cut with `UNTIL`).
- ✅ Kept in your time zone (`DTSTART;TZID=…`, Microsoft's `recurrenceTimeZone`, Google's `timeZone`): a weekly
  10:00 stays at 10:00 after the clocks change. Rules Havre can't describe are shown and kept as they are.
- ✅ Safety: every rule is built by Havre from checked fields (no rule text from the browser); 1–99, at most 730
  times, time zones from the IANA list; deleting several needs an explicit choice.
- Later: the assistant proposing repeating events, drag to move, reminders.
- *Learned:* recurrence rules (`RRULE`, `RECURRENCE-ID`, `EXDATE`, `UNTIL` in UTC), wall-clock time versus UTC
  across daylight saving (`chrono-tz`), three APIs' ways to describe the same series, Rust's `let`-chains.

## 17 · Drafts on the mail server ✅
- ✅ Every draft also has a copy in its account's Drafts folder (`APPEND` with `\Draft`), written 15 s after the last
  change and when the window closes, so other mail apps see it. One copy per draft: each version replaces the
  previous one, found by the draft's own `Message-ID` (`UID EXPUNGE`, never a folder-wide `EXPUNGE`).
- ✅ Sending or discarding removes the copy; moving the draft to another account moves it.
- ✅ A draft started elsewhere opens in the compose window (*Edit draft*): recipients, `Bcc`, subject, text,
  attachments; saving replaces that copy.
- ✅ The compose window says where the draft is saved; *Unsent* lists the drafts not on the server (with why).
  Failures are kept and retried every 2 minutes: Havre's copy stays the reference.
- Later: changes made elsewhere appearing live in an open compose window, rich text drafts, creating a Drafts folder
  when a server has none.
- *Learned:* IMAP `APPEND`, `UIDPLUS` and `UID EXPUNGE`, searching by header, a background loop with a `Weak`
  pointer, a version counter instead of a "dirty" flag (no change lost while a save is in flight).

## 18 · Off-site backups ✅
- ✅ After each backup, a sealed copy sent to a WebDAV folder (Nextcloud, a NAS) or an S3-compatible bucket
  (Backblaze B2, MinIO, Scaleway, AWS); the last N kept there, other files never touched.
- ✅ Sealed: XChaCha20-Poly1305 in STREAM mode (64 KiB chunks, counter and last-chunk flag, authenticated
  header), with a key derived from the recovery key (HMAC-SHA256): one key to keep, unreadable off-site,
  a changed or cut copy refused.
- ✅ Settings › Backups › Off-site copy: WebDAV or S3, *Test the connection*, on/off, copies to keep, last copy
  or the error; *Copy off-site* on each backup. HTTPS only (plain HTTP to the same machine); the secret is
  encrypted and write-only.
- ✅ Retries (every 15 minutes, then hourly), kept across restarts; the status card turns orange after a day.
- ✅ `havre restore <file>.havre --key …` on a new machine, no off-site credentials needed.
- Later: restoring from an off-site copy in the page, several destinations, SFTP, encrypting local backups,
  checking old copies now and then.
- *Learned:* authenticated encryption on a stream, deriving keys, AWS Signature V4 (canonical requests,
  HMAC chains), WebDAV verbs (`PROPFIND`, `MKCOL`, `MOVE`), parsing XML answers, a retry queue in SQLite.

## 19 · Live answers ✅
- ✅ The assistant (AI tab, Ctrl+K) writes its answer as the model writes it, steps shown as they start; text the
  model wrote before using a tool is replaced by the real answer (`restart`). Citations become links at the end.
- ✅ Tool calls read from a stream too, in both dialects: OpenAI's `delta.tool_calls` fragments (Ollama's
  arguments sent as an object), Anthropic's `tool_use` blocks and `input_json_delta`; Claude Code's text as it
  comes, its steps through the MCP server.
- ✅ `POST /ai/ask` and `/ai/conversations/ask` answer with NDJSON lines when asked (`Accept:
  application/x-ndjson`), JSON otherwise. Stop, or a closed page, drops the receiver: the model stops, nothing is
  saved.
- ✅ New proposals, confirmed by you: `propose_note_append` (text added at the end of a note) and
  `propose_task_done` (tick a task), for notes and tasks the assistant found, with their apps in the dock.
- ✅ Tested on recorded streams cut at every byte, and with a pretend model server (tokio's `TcpListener`).
- Later: changing a note's existing text, the assistant deleting anything (never without a click).
- *Learned:* putting a protocol together from pieces of any size (buffer until a full line, JSON from fragments),
  `tokio::join!` and `async move` blocks, an unbounded channel and an `AtomicBool` shared between tasks,
  cancelling by dropping a `Receiver`; on the web, `ReadableStream`, `AbortController`, and why a click's default
  action can still run after its handler changed the button (`preventDefault`).

## 20 · Havre on your phone ✅
- ✅ Installable web app: `manifest.webmanifest`, icons (any, maskable, Apple touch) drawn from Havre's icon,
  theme colours for both themes, a hand-written service worker (`sw.js`) that caches nothing but an offline page.
- ✅ Web Push written by Havre (`push/crypto.rs`): RFC 8291 encryption (P-256 ECDH, HKDF from HMAC, AES-128-GCM)
  checked against the RFC's own example, VAPID (RFC 8292) as an ES256 JWT; the VAPID key made on first use and
  kept encrypted with `Secrets`. New dependencies: `p256`, `aes-gcm` (RustCrypto).
- ✅ What's pushed: task reminders (when the server rings them), new mail in an inbox (unread, from the last day,
  not snoozed, newer than a per-inbox mark set at startup, so a first sync never floods); more than three at
  once make one notification; the sender and subject can be left out.
- ✅ Settings › Notifications: turn on (permission, subscribe), send a test, what to send, devices with Remove; a
  device where Havre is open in front gets nothing (the page says so every 30 s); 404/410 from the push service
  removes the device.
- ✅ Tested with a pretend push service (headers, decrypting what it received, 410), and in Chromium (manifest,
  service worker, its push and click handlers).
- Later: ~~notification buttons (Archive, Mark as read)~~, ~~agenda reminders~~ (step 21), ~~new invitations~~
  (step 26).
- *Learned:* elliptic-curve Diffie–Hellman, HKDF, AES-GCM and ECDSA in practice, JWTs, testing against an RFC's
  example; `OnceCell`; service workers, the Push and Notifications APIs, web app manifests.

## 21 · Event reminders, and dragging events ✅
- ✅ A reminder per event (minutes before the start; all-day: before midnight your time), in the editor and the
  event panel, with a default for new events (and the AI's) in Settings › Calendars.
- ✅ Synced both ways: Microsoft (`isReminderOn`, `reminderMinutesBeforeStart`), Google (`reminders.overrides`,
  the calendar's `defaultReminders` for `useDefault`), CalDAV and Havre's calendar (the first `VALARM` relative
  to the start; others kept — icalendar can't remove a child, so the event is rebuilt without it).
- ✅ Rung every 30 s, exactly once per occurrence (`event_reminders_rung`, by calendar, provider id and start, so
  a sync rewriting rows doesn't ring twice and a moved event rings again); not for declined meetings; more than
  15 minutes late is skipped. A toast with *Join*, Havre Desktop's notification, a push (its own setting).
- ✅ Dragging in the week and day views: move (time and day), resize from the bottom edge, 15-minute steps, a
  ghost with the times, scrolling at the edges; press and hold on touch; Alt+arrows from the keyboard. Repeating
  events ask the scope, meetings with guests ask before Microsoft or Google email them; Undo.
- ✅ Tested: each provider's format, VALARMs kept and replaced, the loop (once, after a sync, moved, declined,
  late, all-day at 9:00 Paris time), push; in Chromium the drags, Undo, resize, keyboard, a phone's hold and swipe.
- Later: several reminders per event, dragging in the month view and the all-day band, snoozing a reminder.
- *Learned:* "exactly once" bookkeeping that survives rewritten rows, one idea mapped onto three APIs and an RFC
  (`VALARM`, `TRIGGER`), `Option<Option<T>>` for "missing / null / value"; pointer capture, long-press and
  blocking a scroll with a non-passive `touchmove`, a click's default action after a handler, keyboard
  alternatives to dragging.

## 22 · People: your details, a better list, keeping in touch ✅
- ✅ Your details for a person (`contacts`): name, company, role, phones, a birthday with or without a year, a
  note, favourite, and *person / automated* when Havre guessed wrong. One person, several addresses
  (`contact_emails`): merge, separate.
- ✅ `messages.recipients` (To and Cc), filled for new mail and, in the background, for mail already there: the
  people you write to count, and "last email you sent", "waiting for your reply" can be told.
- ✅ The list: sort (recent, frequent, A–Z), filters (everyone, favourites, waiting, newsletters & robots), by
  company (yours, else the domain; not public mail domains), rows with last contact and marks.
- ✅ A person's page: details, ★, Edit, ⋯ (merge, separate, not a person), keeping in touch. Today: birthdays in
  the week, two favourites gone quiet for 30 days.
- ✅ Tested: sightings both ways and meetings, your names first, merging, robots by pattern and by choice,
  waiting (and not after you replied, after 30 days, or from robots), birthdays (Feb 29), recipients from headers;
  in Chromium the list, editing, merging, Today, a phone, dark and light.
- *Learned:* one person with several addresses (a join table, a unique address), grouping in Rust after one SQL
  pass, backfilling a new column without blocking the app, dates without a year.

## 23 · Contacts sync ✅
- ✅ Google Contacts (People API, sync tokens, a full read when one expires), Outlook contacts (Graph, the main
  folder, skipped when `changeKey` is unchanged), CardDAV address books (discovery from the CalDAV server,
  `getctag`, ETags, `addressbook-multiget`, `PUT` with `If-Match`). Off until switched on per account.
- ✅ A three-way merge field by field (the details as last synced on each link): each side's changes reach the
  other, the address book wins a field changed on both, the page says *updated from …*. Edits go out right away,
  in the background; someone in no address book can go to a default one.
- ✅ One Havre contact in several books; deleted there or the account switched off: the people that only came
  from it go (favourites stay). Nothing is ever deleted from an address book.
- ✅ People without an address (`/people/~42`): listed, searched (phones too), edited, given an address later
  (their tasks and notes follow). Photos (inline in vCards, Google's links, Graph's on demand), kept small.
- ✅ A hand-written vCard reader and writer (2.1, 3.0, 4.0; folding, escaping, quoted-printable, Apple's
  no-year birthdays); writing back keeps every line Havre doesn't know. `.vcf` import and export.
- ✅ Tested: the vCard flavours, each provider's format, the merge table, listings (full, changes, deleted,
  unchanged ETags), people without an address; end to end against a Radicale server (both ways, a new person to
  the default book, `%40` in card names, deletion, switching off); in Chromium the list, a page without an
  address, adding one, a new person, the settings, a phone, dark and light.
- Later: other Outlook folders, contact groups, postal addresses on the page, writing photos back.
- *Learned:* incremental sync with tokens (and when they expire), a three-way merge, a line-based format by hand
  (folding at byte boundaries, escaping), conditional requests (`If-Match`), percent-encoding when comparing
  URLs, a page remounted on a new address (state in the URL instead).

## 24 · Signatures, undo send, send later ✅
- ✅ A plain-text signature per account, below a `-- ` line; above the quote in replies and forwards (or not);
  swapped when *From* changes unless edited; grey in the HTML version. A body that's only the signature counts as
  empty (no Drafts copy, not listed).
- ✅ One outbox for both: a draft with a time to send. *Send* = now + the undo delay (Settings › Your apps › Mail);
  *Undo* / *Don't send* clear it; *Send later* picks the time (tomorrow 8:00 / 13:00, Monday 8:00, any). A loop
  every 2 s sends what's due with the usual `send_draft`.
- ✅ Never twice: an atomic claim (`UPDATE … WHERE send_state = 'waiting'`); a message caught mid-send by a
  restart is marked *may have gone*, never resent on its own. Scheduled messages leave the server's Drafts
  folder. Failures stay in *Scheduled* with the reason, announced (toast, Desktop, push); late sends said so.
- ✅ Tested: signatures (new, reply, off in replies, swap, edited, HTML), the outbox (due times, the claim,
  undo, interrupted, settings, recipients checked at *Send*), the failure push; end to end against a TLS SMTP
  server (undo before the time, sent after it, *too late* to undo, send later, a failure with the server down,
  sending again); in Chromium the signature, *From* swap, the menu, the picker, *Scheduled*, *Don't send*, the
  settings, the night of a clock change (8:00 stays 8:00), dark, light, phone.
- Later: ~~rich text in messages and signatures~~ (step 25), several signatures per account, aliases, follow-up
  reminders.
- *Learned:* a job queue in SQLite (a column as the schedule, a loop as the worker), "at most once" with an atomic
  claim and its affected-rows count, what to do after a crash in the middle of something that can't be undone,
  delays done on the server, local time for "tomorrow at 8" across a clock change, `trim` in SQLite.

## 25 · Rich text in Mail ✅
- ✅ The compose window's text is rich text (Nuxt UI's `UEditor`, on TipTap): bold, italic, underline,
  strikethrough, links (`Ctrl+K`), lists, quotes, clear formatting; Markdown-like shortcuts while typing. The
  editor only has what the server keeps, and loads only when a compose window or a signature opens.
- ✅ Never trusting the browser: `compose/html.rs` cleans what it sends with `ammonia` and a short allowlist
  (`https`, `http`, `mailto` links only). The HTML is the draft's source; its plain text is made from it by hand
  (lists, `text <address>` links, `> ` quotes, the `-- ` line) for the text part, search, the AI and plain mode.
- ✅ Replies and forwards quote the original's cleaned HTML (or its text as paragraphs) in a `<blockquote>`;
  drafts from other apps keep their formatting. Signatures are formatted too (bold, italic, links), a block of
  their own in the message (`<div data-signature>`), still swapped on *From*. *Write messages in*: rich or plain.
- ✅ Tested: cleaning (scripts, events, styles, `javascript:` links), HTML → text (nested lists, quotes in quotes,
  entities, a space between two tags), replies, signatures (inserted, swapped from the editor's HTML, edited, plain
  mode); in Chromium the toolbar, the shortcuts, a link, pasting a styled page, a reply, the signature editor,
  plain mode, a long message, dark, light, phone; the sent message's two parts read back from an SMTP server.
- Later: images in messages, colours and fonts, tables, templates.
- *Learned:* allowlist sanitizing on the server, walking HTML to write text, one source of truth with a derived
  form, how an editor's schema limits what it can produce, lazy-loaded components, Vue's single-root rule for
  `class` and scoped styles.

## 26 · Notifications that act ✅
- ✅ Buttons on push notifications, done on the server without opening Havre: *Archive* / *Mark as read* (new
  mail), *Mark all as read* (several), *Done* / *In 1 hour* (tasks), *Join* or *In 5 / 10 min* (events), *Accept* /
  *Decline* (invitations). Chrome, Edge and Android show them; Safari and Firefox don't (tapping still works).
- ✅ `push/actions.rs`: a notification carries its buttons and a `Target` (a `serde` tagged enum: the email, task,
  event or invitation); `POST /api/push/action` takes them back from the service worker with the session cookie.
  The server only knows Havre's buttons (`Action::parse`), refuses a button on the wrong target (`check`), checks the
  email is still there, then uses the app's own functions (archive, flags, `set_done`, `invitations::respond`).
- ✅ Invitations: new mail is watched until its body is read; an invitation (read from Havre's copy, not the mail
  server) replaces the email's notification (the same tag). Not your own meetings, cancellations or updates you
  already answered. A switch in Settings › Notifications (migration `0027`).
- ✅ Snoozing: a `push_snoozed` row; every 30 s the due ones are deleted, then announced again on the event channel,
  so the reminder rings everywhere a reminder rings (push, the open page, Desktop), once, unless the task is done.
- ✅ Tested: names and targets (unknown buttons, wrong pairs, the JSON), each notification's buttons, *Join* only for
  `https`, snoozes coming back once and not for a done task, an invitation replacing its email (and not for your own
  meeting), mail buttons only on mail still there; in Chromium the service worker's own handlers (buttons shown,
  *Done* ticking the task, snoozing, a missing email, a wrong or unknown button, *Join*, signed out), the settings.
- Later: buttons on Havre Desktop's notifications, replying from a notification, choosing the buttons.
- *Learned:* notification actions and `notificationclick`, a small action API that never trusts its payload, `serde`
  tagged enums, matching on a pair, "once for every device" with a table, a notification replacing another (tags).

## 27 · File previews ✅
- ✅ A preview window for attachments, in Mail (tap an attachment; the arrow downloads it) and Files (the pane shows
  the first page or the start of a text; *Open* shows it larger): PDFs (pages drawn as you scroll, page counter, zoom,
  *Fit width*), text (UTF-8, else Latin-1; the first 1 MB), audio and video players, images. Full screen on a phone.
- ✅ Nothing changed on the server: attachments are still forced downloads with a sandbox header. The page fetches the
  bytes and shows them itself (Vue's escaping, `blob:` URLs released on close, pdf.js on a canvas). HTML and SVG are
  never previewed; above 25 MB, download only.
- ✅ New dependency: `pdfjs-dist` (Mozilla's reader), its legacy build (the modern one needs JavaScript features some
  browsers lack), lazy-loaded with its worker; its fonts, decoders and character maps served by Havre. Its optional
  native canvas (`@napi-rs/canvas`, for Node only) is skipped (`ignoredOptionalDependencies`).
- ✅ Tested: which files get a preview (Node's test runner, `pnpm test`, now in CI); in Chromium, a five-page PDF (drawn,
  counter, zoom, scrolling to the end), a PDF with JavaScript (nothing ran), UTF-8 and Latin-1 text with a `<script>`
  line shown as text, an MP3 sent as `octet-stream`, a WebM video, HTML and `.docx` download only, the `blob:` released,
  pdf.js loaded only when a PDF opens and only from Havre, the Files pane and window, phone, dark and light.
- Later: Word, Excel and PowerPoint previews, searching in a PDF, previews of your own attachments when writing.
- *Learned:* why strangers' files never get your origin, `fetch` + `TextDecoder` + `Blob` + object URLs, Web Workers,
  `IntersectionObserver` for drawing on demand, sizing placeholders so scrolling lands right, lazy components and
  dynamic `import()`, polyfilled "legacy" builds.
