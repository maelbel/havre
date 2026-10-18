//! Keeping the local store in sync with the mail server.
//!
//! Two background tasks run for the account:
//!
//! - **The worker** syncs folders: on start, every few minutes, when asked
//!   (refresh button, after an action), and when the IDLE task hears news.
//!   It is the only task that writes headers to the store.
//! - **The IDLE task** keeps a connection open on the INBOX with IMAP `IDLE`:
//!   the server pushes "something changed" and we sync right away, so new
//!   mail appears within a second or two.
//!
//! The web UI is told about changes through [`SyncEvent`]s (sent to the
//! browser with Server-Sent Events by the server crate).
//!
//! The engine is also the entry point for everything the API needs
//! (pages, messages, actions): it reads the store first and only asks the
//! mail server when it must.

use std::{
    collections::HashSet,
    sync::{Arc, RwLock},
    time::{Duration, SystemTime},
};

use async_imap::{
    extensions::idle::IdleResponse,
    types::{Fetch, Mailbox},
};
use futures::TryStreamExt;
use serde::Serialize;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::{
    Error, Result,
    config::ImapConfig,
    mail::{
        self, ActionOutcome, FolderRole, ListQuery, MessageDetail, MessagePage, RenderOptions,
        imap::{Session, connect, folders_in_session, is_flagged, is_unread, summaries},
    },
    store::{Flags, FolderInfo, FolderState, Store},
};

/// Headers fetched per round trip. Small enough that the first screen of a
/// big mailbox shows up quickly, big enough to backfill fast.
const BATCH: u32 = 500;
/// How often everything is re-checked when nothing tells us to.
/// Recent inbox messages downloaded per sync pass…
const PREFETCH: usize = 25;
/// …among the newest this many.
const PREFETCH_DEPTH: i64 = 150;
const POLL_EVERY: Duration = Duration::from_secs(5 * 60);
/// With the Files app: older emails read per sync pass to find their
/// attachments, asked for this many at a time.
const FILES_PER_PASS: i64 = 200;
const FILES_BATCH: usize = 50;
/// While older headers are still being downloaded, continue soon.
const BACKFILL_EVERY: Duration = Duration::from_secs(2);
/// After an error, wait before trying again.
const RETRY_EVERY: Duration = Duration::from_secs(30);
/// Every N passes, ignore fingerprints and check every flag (catches the rare
/// changes a fingerprint can miss).
const FULL_CHECK_EVERY: u32 = 12;

/// Something the web UI may want to react to.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncEvent {
    /// The folder list or counters changed.
    Folders,
    /// Messages in this folder changed (new, deleted, read…).
    Folder { path: String },
    /// The sync status changed.
    Status { status: SyncStatus },
    /// An account was added, edited or removed.
    Accounts,
    /// Calendars or events changed (the Agenda).
    Agenda,
    /// Task lists or tasks changed.
    Tasks,
    /// A task's reminder time has come.
    Reminder { task: i64, title: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncState {
    Starting,
    Syncing,
    Idle,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SyncStatus {
    pub state: SyncState,
    /// Unix time of the last successful sync.
    pub last_success: Option<i64>,
    pub error: Option<String>,
    /// Older messages are still being downloaded.
    pub backfilling: bool,
    /// New mail is pushed by the server (IMAP IDLE is connected).
    pub push: bool,
}

/// What the worker should sync.
enum Job {
    All { force: bool },
    Folder(String),
}

struct Request {
    job: Job,
    /// Told when the job is done (or failed).
    done: Option<oneshot::Sender<Result<()>>>,
}

/// A [`SyncEvent`] and the account it is about. All accounts share one
/// channel, so the web UI needs a single connection.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AccountEvent {
    pub account: i64,
    #[serde(flatten)]
    pub event: SyncEvent,
}

pub struct SyncEngine {
    account: i64,
    config: ImapConfig,
    store: Store,
    events: broadcast::Sender<AccountEvent>,
    /// Set to `true` to stop the background tasks (account removed or edited).
    stop: watch::Sender<bool>,
    status: RwLock<SyncStatus>,
    requests: mpsc::UnboundedSender<Request>,
    /// Serializes "download older headers" between the worker and `page()`.
    backfill: tokio::sync::Mutex<()>,
}

impl SyncEngine {
    /// Creates the engine for one account and starts its background tasks.
    /// Events go to `events`, shared by all accounts.
    pub fn start(
        account: i64,
        config: ImapConfig,
        store: Store,
        events: broadcast::Sender<AccountEvent>,
    ) -> Arc<Self> {
        let (requests, inbox) = mpsc::unbounded_channel();
        let engine = Arc::new(Self {
            account,
            config,
            store,
            events,
            stop: watch::channel(false).0,
            status: RwLock::new(SyncStatus {
                state: SyncState::Starting,
                last_success: None,
                error: None,
                backfilling: false,
                push: false,
            }),
            requests,
            backfill: tokio::sync::Mutex::new(()),
        });
        tokio::spawn(Arc::clone(&engine).worker(inbox));
        tokio::spawn(Arc::clone(&engine).idle_loop());
        engine
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn config(&self) -> &ImapConfig {
        &self.config
    }

    /// Changes as they happen. Each subscriber gets every event.
    /// Stops the background tasks. In-flight requests still finish.
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }

    /// Resolves when [`stop`](Self::stop) is called.
    async fn stopped(&self) {
        let mut rx = self.stop.subscribe();
        let _ = rx.wait_for(|stop| *stop).await;
    }

    pub fn status(&self) -> SyncStatus {
        self.status.read().map(|s| s.clone()).unwrap_or_else(|e| e.into_inner().clone())
    }

    /// Syncs now (one folder, or everything) and waits until it is done.
    pub async fn sync_now(&self, folder: Option<String>) -> Result<()> {
        let (done, wait) = oneshot::channel();
        let job = match folder {
            Some(path) => Job::Folder(path),
            None => Job::All { force: true },
        };
        self.requests
            .send(Request { job, done: Some(done) })
            .map_err(|_| Error::Config("the sync task has stopped".into()))?;
        wait.await.map_err(|_| Error::Config("the sync task has stopped".into()))?
    }

    /// Asks for a sync without waiting.
    pub fn request_sync(&self, folder: Option<String>) {
        let job = match folder {
            Some(path) => Job::Folder(path),
            None => Job::All { force: false },
        };
        let _ = self.requests.send(Request { job, done: None });
    }

    fn emit(&self, event: SyncEvent) {
        // `send` fails only when nobody listens: that's fine.
        let _ = self.events.send(AccountEvent { account: self.account, event });
    }

    fn update_status(&self, change: impl FnOnce(&mut SyncStatus)) {
        let status = {
            let mut guard = self.status.write().unwrap_or_else(|e| e.into_inner());
            let before = guard.clone();
            change(&mut guard);
            if *guard == before {
                return;
            }
            guard.clone()
        };
        self.emit(SyncEvent::Status { status });
    }

    // ---- API operations ---------------------------------------------------

    pub async fn folders(&self) -> Result<Vec<FolderInfo>> {
        self.store.folders().await
    }

    /// A page of messages. Reads the store; if older headers are not
    /// downloaded yet, fetches them from the server now (and keeps them).
    pub async fn page(&self, folder: &str, query: &ListQuery) -> Result<MessagePage> {
        let page = self.store.page(folder, query).await?;
        let missing = query.limit as usize - page.messages.len().min(query.limit as usize);
        let state = self.store.folder_state(folder).await?;
        if missing == 0 || state.complete || query.search.is_some() || state.newest_uid.is_none() {
            return Ok(page);
        }

        // Not downloaded yet: extend the stored range downwards, batch by
        // batch, until this page is full. Always from the oldest stored UID,
        // so the stored range never has holes.
        let _guard = self.backfill.lock().await;
        for _ in 0..10 {
            let state = self.store.folder_state(folder).await?;
            if state.complete {
                break;
            }
            let remote = mail::fetch_messages(
                &self.config,
                folder,
                &ListQuery { limit: BATCH, before_uid: state.oldest_synced_uid, search: None },
            )
            .await?;
            self.store.upsert_messages(folder, &remote.messages).await?;
            let oldest = remote.messages.iter().map(|m| m.uid).min().or(state.oldest_synced_uid);
            self.store.set_backfill(folder, oldest, !remote.has_more).await?;
            let page = self.store.page(folder, query).await?;
            if page.messages.len() >= query.limit as usize {
                return Ok(page);
            }
        }
        self.store.page(folder, query).await
    }

    /// Server-side search: covers message bodies, not only headers.
    pub async fn search_server(&self, folder: &str, query: &ListQuery) -> Result<MessagePage> {
        mail::fetch_messages(&self.config, folder, query).await
    }

    /// Opens a message: from the cache when possible, else from the server
    /// (then cached). With `mark_read`, it is marked as read everywhere.
    pub async fn message(
        &self,
        folder: &str,
        uid: u32,
        mark_read: bool,
        options: &RenderOptions<'_>,
    ) -> Result<MessageDetail> {
        let raw = match self.store.body(folder, uid).await? {
            Some(raw) => {
                if mark_read && self.store.is_unread(folder, uid).await? == Some(true) {
                    mail::set_seen(&self.config, folder, uid, true).await?;
                }
                raw
            }
            None => {
                // One round trip: download and flag as read together.
                let (raw, _) = mail::fetch_raw_message(&self.config, folder, uid, mark_read).await?;
                self.store.store_body(folder, uid, &raw).await?;
                raw
            }
        };
        if mark_read && self.store.is_unread(folder, uid).await? == Some(true) {
            self.store.set_unread(folder, uid, false).await?;
            self.emit(SyncEvent::Folders);
        }
        Ok(MessageDetail::parse(uid, folder, false, &raw, options))
    }

    pub async fn attachment(&self, folder: &str, uid: u32, index: usize) -> Result<mail::AttachmentData> {
        let raw = self.raw(folder, uid).await?;
        mail::extract_attachment(&raw, index)
            .ok_or_else(|| Error::NotFound(format!("attachment {index} of message {uid}")))
    }

    /// The whole message as received (from the local copy when cached).
    /// Doesn't mark it as read.
    pub async fn raw(&self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        if let Some(raw) = self.store.body(folder, uid).await? {
            return Ok(raw);
        }
        let (raw, _) = mail::fetch_raw_message(&self.config, folder, uid, false).await?;
        self.store.store_body(folder, uid, &raw).await?;
        Ok(raw)
    }

    /// Read/unread: the server first (it is the truth), then the store.
    pub async fn set_seen(&self, folder: &str, uid: u32, seen: bool) -> Result<()> {
        mail::set_seen(&self.config, folder, uid, seen).await?;
        self.store.set_unread(folder, uid, !seen).await?;
        self.emit(SyncEvent::Folders);
        Ok(())
    }

    /// Read/unread on several messages at once.
    pub async fn set_seen_many(&self, folder: &str, uids: &[u32], seen: bool) -> Result<()> {
        mail::set_flag(&self.config, folder, uids, "\\Seen", seen).await?;
        for uid in uids {
            self.store.set_unread(folder, *uid, !seen).await?;
        }
        self.emit(SyncEvent::Folder { path: folder.to_owned() });
        self.emit(SyncEvent::Folders);
        Ok(())
    }

    /// Star / unstar: the server first, then the store.
    pub async fn set_flagged(&self, folder: &str, uids: &[u32], flagged: bool) -> Result<()> {
        mail::set_flag(&self.config, folder, uids, "\\Flagged", flagged).await?;
        self.store.set_flagged(folder, uids, flagged).await?;
        self.emit(SyncEvent::Folder { path: folder.to_owned() });
        Ok(())
    }

    /// Adds or removes a Havre label (kept in Havre, by Message-ID).
    pub async fn set_label(&self, folder: &str, uids: &[u32], label: i64, on: bool) -> Result<()> {
        self.store.set_label(folder, uids, label, on).await?;
        self.emit(SyncEvent::Folder { path: folder.to_owned() });
        Ok(())
    }

    /// Snoozes until `until` (unix seconds), or wakes up now (`None`).
    pub async fn snooze(&self, folder: &str, uids: &[u32], until: Option<i64>) -> Result<()> {
        if let Some(until) = until
            && until <= unix_now()
        {
            return Err(Error::Config("Choose a time in the future.".into()));
        }
        self.store.snooze(folder, uids, until).await?;
        self.emit(SyncEvent::Folder { path: folder.to_owned() });
        self.emit(SyncEvent::Folders);
        Ok(())
    }

    /// A snooze is over: unread again, and back on top.
    pub async fn wake(&self, folder: &str, uid: u32) -> Result<()> {
        // Unread on the server too, so your phone shows it again.
        if let Err(err) = mail::set_flag(&self.config, folder, &[uid], "\\Seen", false).await {
            tracing::debug!(%err, "marking a snoozed message unread");
        }
        self.store.set_unread(folder, uid, true).await?;
        self.store.mark_back(folder, uid).await?;
        self.emit(SyncEvent::Folder { path: folder.to_owned() });
        self.emit(SyncEvent::Folders);
        Ok(())
    }

    /// Undoes an archive / move / trash: finds the messages (by Message-ID)
    /// in `from`, where they went, and moves them back to `to`.
    pub async fn undo_move(&self, from: &str, to: &str, message_ids: &[String]) -> Result<usize> {
        let uids = mail::find_by_message_id(&self.config, from, message_ids).await?;
        if uids.is_empty() {
            return Err(Error::Config("These messages aren't there any more: nothing to undo.".into()));
        }
        mail::move_messages(&self.config, from, &uids, to).await?;
        self.store.remove_messages(from, &uids).await?;
        self.emit(SyncEvent::Folders);
        self.request_sync(Some(to.to_owned()));
        Ok(uids.len())
    }

    /// Archive / trash / move, then update the store without waiting for
    /// the next sync. Also returns the messages' Message-IDs (for Undo).
    pub async fn apply_undoable(
        &self,
        action: Action,
        folder: &str,
        uids: &[u32],
        to: Option<&str>,
    ) -> Result<(ActionOutcome, Vec<String>)> {
        let ids = self.store.message_ids(folder, uids).await?;
        Ok((self.apply(action, folder, uids, to).await?, ids))
    }

    /// Archive / trash / move, then update the store without waiting for
    /// the next sync.
    pub async fn apply(
        &self,
        action: Action,
        folder: &str,
        uids: &[u32],
        to: Option<&str>,
    ) -> Result<ActionOutcome> {
        let outcome = match action {
            Action::Archive => mail::archive_messages(&self.config, folder, uids).await?,
            Action::Trash => mail::trash_messages(&self.config, folder, uids).await?,
            Action::Move => {
                let to = to.ok_or_else(|| Error::Config("`to` is required to move messages".into()))?;
                mail::move_messages(&self.config, folder, uids, to).await?
            }
        };
        self.store.remove_messages(folder, uids).await?;
        self.emit(SyncEvent::Folder { path: folder.to_owned() });
        self.emit(SyncEvent::Folders);
        match &outcome {
            // The messages got new UIDs over there: let the sync pick them up.
            // (Archive may also have created the folder: resync the list.)
            ActionOutcome::Moved { .. } if action == Action::Archive => self.request_sync(None),
            ActionOutcome::Moved { to } => self.request_sync(Some(to.clone())),
            ActionOutcome::Deleted => {}
        }
        Ok(outcome)
    }

    // ---- The worker ---------------------------------------------------------

    async fn worker(self: Arc<Self>, mut inbox: mpsc::UnboundedReceiver<Request>) {
        let mut next_wait = Duration::ZERO; // first pass right away
        let mut passes: u32 = 0;
        loop {
            // Wait for a request or for the timer, whichever comes first.
            let first = tokio::select! {
                request = inbox.recv() => match request {
                    Some(r) => r,
                    None => return, // engine dropped
                },
                _ = self.stopped() => return,
                _ = tokio::time::sleep(next_wait) => Request { job: Job::All { force: false }, done: None },
            };
            // Merge everything else already queued into one pass.
            let mut batch = vec![first];
            while let Ok(more) = inbox.try_recv() {
                batch.push(more);
            }
            let all = batch.iter().any(|r| matches!(r.job, Job::All { .. }));
            let force = batch.iter().any(|r| matches!(r.job, Job::All { force: true }))
                || passes.is_multiple_of(FULL_CHECK_EVERY);
            let folders: Vec<String> = batch
                .iter()
                .filter_map(|r| match &r.job {
                    Job::Folder(p) => Some(p.clone()),
                    Job::All { .. } => None,
                })
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();

            self.update_status(|s| s.state = SyncState::Syncing);
            let result = if all { self.sync_all(force).await } else { self.sync_folders(&folders).await };
            // Recent inbox mail: download it so the list can show previews,
            // attachments and invitations (and opening it is instant).
            if result.is_ok()
                && let Err(err) = self.prefetch_recent().await
            {
                tracing::debug!(%err, "background download of recent mail");
            }
            if result.is_ok()
                && let Err(err) = self.look_for_files().await
            {
                tracing::debug!(%err, "looking for files in older mail");
            }

            next_wait = match &result {
                Ok(backfilling) => {
                    if all {
                        passes += 1;
                    }
                    let now = unix_now();
                    self.update_status(|s| {
                        s.state = SyncState::Idle;
                        s.error = None;
                        s.last_success = Some(now);
                        if all {
                            s.backfilling = *backfilling;
                        }
                    });
                    if *backfilling { BACKFILL_EVERY } else { POLL_EVERY }
                }
                Err(err) => {
                    tracing::warn!("sync failed: {err}");
                    let message = err.to_string();
                    self.update_status(|s| {
                        s.state = SyncState::Error;
                        s.error = Some(message);
                    });
                    RETRY_EVERY
                }
            };

            for request in batch {
                if let Some(done) = request.done {
                    let _ = done.send(match &result {
                        Ok(_) => Ok(()),
                        Err(e) => Err(Error::Config(e.to_string())),
                    });
                }
            }
        }
    }

    /// Downloads the newest inbox messages not downloaded yet (at most
    /// [`PREFETCH`] per pass, the newest [`PREFETCH_DEPTH`] in all).
    async fn prefetch_recent(&self) -> Result<()> {
        let folders = self.store.folders().await?;
        let Some(inbox) = folders.iter().find(|f| f.folder.role == FolderRole::Inbox) else { return Ok(()) };
        let path = inbox.folder.path.clone();
        // Already downloaded (before previews existed): no need to ask the server.
        let digested = self.store.digest_cached(&path, PREFETCH_DEPTH).await?;
        let newest: Vec<u32> = self.store.missing_bodies(&path, PREFETCH_DEPTH).await?;
        let batch: Vec<u32> = newest.into_iter().take(PREFETCH).collect();
        if batch.is_empty() {
            if digested > 0 {
                self.emit(SyncEvent::Folder { path });
            }
            return Ok(());
        }
        let fetched = mail::imap::fetch_raw_many(&self.config, &path, &batch).await?;
        for (uid, raw) in &fetched {
            self.store.store_body(&path, *uid, raw).await?;
        }
        if !fetched.is_empty() {
            self.emit(SyncEvent::Folder { path });
        }
        Ok(())
    }

    /// With the Files app in the dock: reads older Inbox and Sent mail, a
    /// little each pass, to note its attachments. Only that note is kept
    /// (the email isn't cached), so the cache of opened mail stays as it is.
    async fn look_for_files(&self) -> Result<()> {
        let Some(depth) = self.store.files_depth().await? else { return Ok(()) };
        let folders = self.store.folders().await?;
        let mut left = FILES_PER_PASS;
        for info in folders.iter().filter(|f| matches!(f.folder.role, FolderRole::Inbox | FolderRole::Sent)) {
            let path = &info.folder.path;
            let uids = self.store.undigested(path, depth, left).await?;
            for batch in uids.chunks(FILES_BATCH) {
                let fetched = mail::imap::fetch_raw_many(&self.config, path, batch).await?;
                for (uid, raw) in &fetched {
                    self.store.save_digest(path, *uid, raw).await?;
                }
                for uid in batch.iter().filter(|u| !fetched.iter().any(|(f, _)| f == *u)) {
                    self.store.mark_no_files(path, *uid).await?;
                }
            }
            left -= uids.len() as i64;
            if left <= 0 {
                break;
            }
        }
        Ok(())
    }

    /// Folder list + every folder. Returns `true` while backfill is pending.
    async fn sync_all(&self, force: bool) -> Result<bool> {
        let mut session = connect(&self.config).await?;
        let mut folders = folders_in_session(&mut session).await?;
        mail::sort_folders(&mut folders);
        let before = self.store.folders().await?;
        self.store.replace_folders(&folders).await?;
        let mut folders_changed = before.len() != folders.len()
            || before.iter().zip(&folders).any(|(a, b)| a.folder.path != b.path || a.folder.name != b.name);

        let condstore = session.capabilities().await?.has_str("CONDSTORE");
        let mut backfilling = false;
        // INBOX first: it's what people look at.
        folders.sort_by_key(|f| f.role != FolderRole::Inbox);
        for folder in &folders {
            let result = self.sync_folder(&mut session, &folder.path, condstore, force).await?;
            backfilling |= result.backfilling;
            folders_changed |= result.counts_changed;
        }
        session.logout().await?;
        if folders_changed {
            self.emit(SyncEvent::Folders);
        }
        Ok(backfilling)
    }

    async fn sync_folders(&self, paths: &[String]) -> Result<bool> {
        let mut session = connect(&self.config).await?;
        let condstore = session.capabilities().await?.has_str("CONDSTORE");
        let mut backfilling = false;
        let mut counts_changed = false;
        for path in paths {
            let result = self.sync_folder(&mut session, path, condstore, true).await?;
            backfilling |= result.backfilling;
            counts_changed |= result.counts_changed;
        }
        session.logout().await?;
        if counts_changed {
            self.emit(SyncEvent::Folders);
        }
        Ok(backfilling)
    }

    /// Syncs one folder. The heart of the engine.
    async fn sync_folder(
        &self,
        session: &mut Session,
        path: &str,
        condstore: bool,
        force: bool,
    ) -> Result<FolderResult> {
        // 1. STATUS: cheap counters. If nothing changed since last time, stop.
        let items = if condstore {
            "(MESSAGES UNSEEN UIDNEXT UIDVALIDITY HIGHESTMODSEQ)"
        } else {
            "(MESSAGES UNSEEN UIDNEXT UIDVALIDITY)"
        };
        let status = session.status(path, items).await?;
        let fingerprint = fingerprint(&status);
        let mut state = self.store.folder_state(path).await?;
        let unchanged = self.store.fingerprint(path).await?.as_deref() == Some(fingerprint.as_str());
        if unchanged && state.complete && !force {
            return Ok(FolderResult::default());
        }
        let total = status.exists;
        let unread = status.unseen.unwrap_or(0);
        let counts_changed = !unchanged;
        self.store.set_counts(path, total, unread).await?;

        // 2. Open the folder read-only.
        let mailbox = session.examine(path).await?;
        let validity = mailbox.uid_validity.unwrap_or(0);
        let mut changed = 0;
        if state.uid_validity != Some(validity) {
            // First sync, or the server renumbered everything.
            if state.uid_validity.is_some() {
                changed += 1;
            }
            self.store.reset_folder(path, validity).await?;
            state = FolderState { uid_validity: Some(validity), ..Default::default() };
        }

        if mailbox.exists == 0 {
            changed += self.store.reconcile(path, 1, u32::MAX, &[]).await?;
            self.store.set_backfill(path, None, true).await?;
        } else {
            match state.newest_uid {
                // 3a. Never synced: the newest batch, by sequence number.
                None => {
                    let first = mailbox.exists.saturating_sub(BATCH - 1).max(1);
                    let fetches =
                        fetch_all(session.fetch(format!("{first}:{}", mailbox.exists), HEADERS).await?)
                            .await?;
                    let messages = summaries(&fetches);
                    changed += self.store.upsert_messages(path, &messages).await?;
                    let oldest = messages.iter().map(|m| m.uid).min();
                    self.store.set_backfill(path, oldest, first == 1).await?;
                    state.oldest_synced_uid = oldest;
                    state.complete = first == 1;
                }
                Some(newest) => {
                    // 3b. New mail: everything above the newest UID we have.
                    let fetches =
                        fetch_all(session.uid_fetch(format!("{}:*", newest + 1), HEADERS).await?).await?;
                    // `n:*` always returns the last message, even if it's older.
                    let fresh: Vec<_> = summaries(&fetches).into_iter().filter(|m| m.uid > newest).collect();
                    changed += self.store.upsert_messages(path, &fresh).await?;

                    // 4. Flags and deletions for what we already have.
                    let oldest = state.oldest_synced_uid.unwrap_or(1);
                    let flags =
                        fetch_all(session.uid_fetch(format!("{oldest}:{newest}"), "(UID FLAGS)").await?)
                            .await?;
                    let present: Vec<(u32, Flags)> = flags
                        .iter()
                        .filter_map(|f| {
                            Some((f.uid?, Flags { unread: is_unread(f), flagged: is_flagged(f) }))
                        })
                        .collect();
                    changed += self.store.reconcile(path, oldest, newest, &present).await?;

                    // Messages stored before Havre kept Message-IDs: read theirs.
                    let missing = self.store.missing_message_ids(path, MESSAGE_ID_BATCH).await?;
                    if !missing.is_empty() {
                        let set = missing.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
                        // The whole header (small), parsed like any new message's.
                        let fetches =
                            fetch_all(session.uid_fetch(set, "(UID BODY.PEEK[HEADER])").await?).await?;
                        let mut found = std::collections::HashSet::new();
                        for f in &fetches {
                            let (Some(uid), Some(header)) = (f.uid, f.header()) else { continue };
                            let parsed = mail::MessageSummary::from_headers(uid, false, header);
                            self.store
                                .set_message_id(path, uid, parsed.message_id.as_deref().unwrap_or(""))
                                .await?;
                            found.insert(uid);
                        }
                        // Gone meanwhile, or no answer: don't ask again and again.
                        for uid in missing.iter().filter(|u| !found.contains(u)) {
                            self.store.set_message_id(path, *uid, "").await?;
                        }
                    }
                }
            }

            // 5. Backfill: one older batch per pass, so new mail never waits.
            //    (Under a lock: `page()` may be extending the same range.)
            if !state.complete {
                let _guard = self.backfill.lock().await;
                let fresh = self.store.folder_state(path).await?;
                if !fresh.complete {
                    let oldest = fresh.oldest_synced_uid.unwrap_or(1);
                    let mut older: Vec<u32> = if oldest > 1 {
                        session.uid_search(format!("UID 1:{}", oldest - 1)).await?.into_iter().collect()
                    } else {
                        Vec::new()
                    };
                    older.sort_unstable_by(|a, b| b.cmp(a));
                    let complete = older.len() <= BATCH as usize;
                    older.truncate(BATCH as usize);
                    if !older.is_empty() {
                        let set = older.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
                        let fetches = fetch_all(session.uid_fetch(set, HEADERS).await?).await?;
                        changed += self.store.upsert_messages(path, &summaries(&fetches)).await?;
                    }
                    let new_oldest = older.last().copied().or(fresh.oldest_synced_uid);
                    self.store.set_backfill(path, new_oldest, complete).await?;
                    state.complete = complete;
                } else {
                    state.complete = true;
                }
            }
        }

        self.store.set_fingerprint(path, &fingerprint).await?;
        if changed > 0 {
            tracing::debug!(folder = path, changed, "folder synced");
            self.emit(SyncEvent::Folder { path: path.to_owned() });
        }
        Ok(FolderResult { backfilling: !state.complete, counts_changed })
    }

    // ---- IDLE: new mail pushed by the server --------------------------------

    async fn idle_loop(self: Arc<Self>) {
        tokio::select! {
            _ = self.idle_forever() => {}
            _ = self.stopped() => {}
        }
    }

    async fn idle_forever(&self) {
        // Let the first sync create the folder list.
        tokio::time::sleep(Duration::from_secs(3)).await;
        loop {
            match self.idle_once().await {
                Ok(false) => {
                    tracing::info!("the server does not support IDLE: polling only");
                    return;
                }
                Ok(true) => {}
                Err(err) => tracing::debug!("IDLE connection lost: {err}"),
            }
            self.update_status(|s| s.push = false);
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

    /// Holds an IDLE connection until it breaks. `Ok(false)` = unsupported.
    async fn idle_once(&self) -> Result<bool> {
        let inbox = self
            .store
            .folders()
            .await?
            .into_iter()
            .find(|f| f.folder.role == FolderRole::Inbox)
            .map(|f| f.folder.path)
            .unwrap_or_else(|| "INBOX".into());

        let mut session = connect(&self.config).await?;
        if !session.capabilities().await?.has_str("IDLE") {
            return Ok(false);
        }
        session.examine(&inbox).await?;
        self.update_status(|s| s.push = true);

        loop {
            let mut idle = session.idle();
            idle.init().await?;
            // Servers drop idle connections after ~30 min: renew every 25.
            let (wait, stop) = idle.wait_with_timeout(Duration::from_secs(25 * 60));
            let response = wait.await;
            drop(stop); // must live until `wait` finishes, or it interrupts it
            session = idle.done().await?;
            if let IdleResponse::NewData(_) = response? {
                // Several notifications often arrive together: let them settle.
                tokio::time::sleep(Duration::from_millis(300)).await;
                self.request_sync(Some(inbox.clone()));
            }
        }
    }
}

/// What a user can do to messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Archive,
    Trash,
    Move,
}

#[derive(Default)]
struct FolderResult {
    backfilling: bool,
    counts_changed: bool,
}

const HEADERS: &str = "(UID FLAGS RFC822.HEADER)";

/// Message-IDs read per folder and pass, for messages stored before step 8.
const MESSAGE_ID_BATCH: i64 = 500;

/// Collects an IMAP response stream into a Vec.
async fn fetch_all(
    stream: impl futures::Stream<Item = async_imap::error::Result<Fetch>>,
) -> Result<Vec<Fetch>> {
    Ok(stream.try_collect().await?)
}

/// A short summary of STATUS: if it is the same as last time, the folder
/// did not change (new mail changes UIDNEXT, reads change UNSEEN, deletions
/// change MESSAGES, and with CONDSTORE any flag change bumps HIGHESTMODSEQ).
fn fingerprint(status: &Mailbox) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        status.uid_validity.unwrap_or(0),
        status.uid_next.unwrap_or(0),
        status.exists,
        status.unseen.unwrap_or(0),
        status.highest_modseq.unwrap_or(0)
    )
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
