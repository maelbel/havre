//! Where mail comes from: your accounts (each with its sync engine and local
//! store), or the built-in demo mailbox.
//!
//! The API handlers only talk to [`MailSource`], so they don't care which
//! one is active. This is a small example of a Rust `enum` holding data.

use std::sync::{Arc, Mutex};

use havre_core::{
    Error, Result,
    accounts::{AccountManager, AccountView, OAuthRequest, SignInStart, SignInState},
    compose::{Draft, Drafts, NewDraft, SendOutcome},
    mail::{
        self, ActionOutcome, AttachmentData, ListQuery, MessageDetail, MessagePage, RenderOptions,
        demo::DemoStore,
    },
    oauth::Provider,
    store::{Account, AccountUpdate, Contact, FolderInfo, NewAccount},
    sync::{AccountEvent, Action, SyncState, SyncStatus},
};
use tokio::sync::broadcast;

/// The demo has a single, pretend account.
pub const DEMO_ACCOUNT: i64 = 1;

#[derive(Clone)]
pub enum MailSource {
    /// In-memory sample mailbox. `Arc<Mutex<…>>` = shared between requests,
    /// one request at a time may change it.
    Demo(Arc<Mutex<DemoStore>>),
    /// Real accounts, kept in sync in the background.
    Accounts(Arc<AccountManager>),
}

impl MailSource {
    pub fn demo() -> Self {
        Self::Demo(Arc::new(Mutex::new(DemoStore::default())))
    }

    // ---- Accounts -------------------------------------------------------------

    pub async fn accounts(&self) -> Result<Vec<AccountView>> {
        match self {
            Self::Demo(store) => {
                let unread = lock(store).folders().first().map_or(0, |f| f.unread);
                Ok(vec![AccountView {
                    account: Account {
                        id: DEMO_ACCOUNT,
                        name: "Demo".into(),
                        email: "mael@example.org".into(),
                        color: "work".into(),
                        imap_host: "demo".into(),
                        imap_port: 993,
                        imap_user: "demo".into(),
                        auth: "password".into(),
                        sender_name: String::new(),
                        smtp_host: "demo".into(),
                        smtp_port: 465,
                        smtp_security: "tls".into(),
                        can_send: false,
                        can_calendar: false,
                        can_tasks: false,
                    },
                    status: idle_status(),
                    unread,
                }])
            }
            Self::Accounts(manager) => manager.accounts().await,
        }
    }

    pub async fn add_account(&self, new: NewAccount) -> Result<Account> {
        self.manager()?.add(new).await
    }

    pub async fn update_account(&self, id: i64, update: AccountUpdate) -> Result<Account> {
        self.manager()?.update(id, update).await
    }

    pub async fn remove_account(&self, id: i64) -> Result<()> {
        self.manager()?.remove(id).await
    }

    pub async fn start_oauth(&self, request: OAuthRequest) -> Result<SignInStart> {
        self.manager()?.start_oauth(request).await
    }

    /// The provider sent the browser back (to the pop-up window).
    pub async fn oauth_callback(&self, flow: &str, code: Option<&str>, error: Option<&str>) -> Result<()> {
        self.manager()?.oauth_callback(flow, code, error).await
    }

    pub async fn poll_oauth(&self, flow: &str) -> Result<SignInState> {
        self.manager()?.poll_oauth(flow).await
    }

    // ---- Writing ---------------------------------------------------------------

    pub fn drafts(&self) -> Result<Drafts> {
        Ok(self.manager()?.drafts())
    }

    pub async fn new_draft(&self, start: NewDraft) -> Result<Draft> {
        self.manager()?.new_draft(start).await
    }

    pub async fn send_draft(&self, id: i64) -> Result<SendOutcome> {
        self.manager()?.send_draft(id).await
    }

    pub async fn contacts(&self, query: &str) -> Result<Vec<Contact>> {
        match self {
            Self::Demo(_) => Ok(Vec::new()),
            Self::Accounts(m) => m.database().contacts(query, 8).await,
        }
    }

    /// Is OAuth2 with this provider configured in `.env`?
    pub fn oauth_enabled(&self, provider: Provider) -> bool {
        match self {
            Self::Demo(_) => false,
            Self::Accounts(m) => m.database().oauth_app(provider).is_some(),
        }
    }

    pub fn manager(&self) -> Result<&Arc<AccountManager>> {
        match self {
            Self::Demo(_) => {
                Err(Error::Config("accounts can't be changed in demo mode (HAVRE_DEMO=1)".into()))
            }
            Self::Accounts(manager) => Ok(manager),
        }
    }

    /// The account to use when the request doesn't say: the first one.
    pub async fn default_account(&self) -> Result<i64> {
        match self {
            Self::Demo(_) => Ok(DEMO_ACCOUNT),
            Self::Accounts(manager) => manager
                .database()
                .accounts()
                .await?
                .first()
                .map(|a| a.id)
                .ok_or_else(|| Error::NotFound("no mail account yet: add one in Settings".into())),
        }
    }

    // ---- Mail -------------------------------------------------------------------

    pub async fn folders(&self, account: i64) -> Result<Vec<FolderInfo>> {
        match self {
            Self::Demo(store) => Ok(lock(store).folders()),
            Self::Accounts(m) => m.engine(account).await?.folders().await,
        }
    }

    /// `remote`: search on the mail server (covers message bodies) instead
    /// of the local store (sender and subject, instant).
    pub async fn messages(
        &self,
        account: i64,
        folder: &str,
        query: &ListQuery,
        remote: bool,
    ) -> Result<MessagePage> {
        match self {
            Self::Demo(store) => Ok(lock(store).page(folder, query)),
            Self::Accounts(m) => {
                let engine = m.engine(account).await?;
                if remote {
                    engine.search_server(folder, query).await
                } else {
                    engine.page(folder, query).await
                }
            }
        }
    }

    pub async fn message(
        &self,
        account: i64,
        folder: &str,
        uid: u32,
        mark_read: bool,
        options: &RenderOptions<'_>,
    ) -> Result<MessageDetail> {
        match self {
            Self::Demo(store) => {
                let mut store = lock(store);
                let raw = store.raw(folder, uid).ok_or_else(|| not_found(uid))?;
                let unread = store.is_unread(uid) && !mark_read;
                if mark_read {
                    store.set_seen(uid, true);
                }
                Ok(MessageDetail::parse(uid, folder, unread, &raw, options))
            }
            Self::Accounts(m) => m.engine(account).await?.message(folder, uid, mark_read, options).await,
        }
    }

    pub async fn set_seen(&self, account: i64, folder: &str, uid: u32, seen: bool) -> Result<()> {
        match self {
            Self::Demo(store) => {
                lock(store).set_seen(uid, seen);
                Ok(())
            }
            Self::Accounts(m) => m.engine(account).await?.set_seen(folder, uid, seen).await,
        }
    }

    pub async fn apply(
        &self,
        account: i64,
        action: Action,
        folder: &str,
        uids: &[u32],
        to: Option<&str>,
    ) -> Result<ActionOutcome> {
        match self {
            Self::Demo(store) => {
                let mut store = lock(store);
                Ok(match action {
                    Action::Archive => store.archive(folder, uids),
                    Action::Trash => store.trash(folder, uids),
                    Action::Move => store.move_to(
                        folder,
                        uids,
                        to.ok_or_else(|| Error::Config("`to` is required to move messages".into()))?,
                    ),
                })
            }
            Self::Accounts(m) => m.engine(account).await?.apply(action, folder, uids, to).await,
        }
    }

    pub async fn attachment(
        &self,
        account: i64,
        folder: &str,
        uid: u32,
        index: usize,
    ) -> Result<AttachmentData> {
        match self {
            Self::Demo(store) => {
                let raw = lock(store).raw(folder, uid).ok_or_else(|| not_found(uid))?;
                mail::extract_attachment(&raw, index).ok_or_else(|| not_found(uid))
            }
            Self::Accounts(m) => m.engine(account).await?.attachment(folder, uid, index).await,
        }
    }

    /// Syncs now and waits (the refresh button). Instant in demo mode.
    pub async fn sync_now(&self, account: i64, folder: Option<String>) -> Result<()> {
        match self {
            Self::Demo(_) => Ok(()),
            Self::Accounts(m) => m.engine(account).await?.sync_now(folder).await,
        }
    }

    pub async fn status(&self, account: i64) -> Result<SyncStatus> {
        match self {
            Self::Demo(_) => Ok(idle_status()),
            Self::Accounts(m) => Ok(m.engine(account).await?.status()),
        }
    }

    /// Live changes of every account. The demo has none: its receiver just
    /// never yields.
    pub fn subscribe(&self) -> broadcast::Receiver<AccountEvent> {
        match self {
            Self::Demo(_) => broadcast::channel(1).1,
            Self::Accounts(m) => m.subscribe(),
        }
    }
}

fn idle_status() -> SyncStatus {
    SyncStatus { state: SyncState::Idle, last_success: None, error: None, backfilling: false, push: false }
}

/// Locks the demo store. A poisoned lock (a panic while it was held) is
/// recovered: demo data is not precious.
fn lock(store: &Mutex<DemoStore>) -> std::sync::MutexGuard<'_, DemoStore> {
    store.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn not_found(uid: u32) -> Error {
    Error::NotFound(format!("message {uid}"))
}
