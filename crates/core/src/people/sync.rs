//! Keeps People and your address books in step.
//!
//! - Every 15 minutes (and on demand, and when an account changes), each
//!   account or server where you switched contacts on is asked what changed
//!   ([`super::google`], [`super::graph`], [`super::carddav`]); each contact
//!   that changed is merged into Havre's field by field ([`People::apply_remote`]).
//! - Then whatever changed in Havre since the last sync is sent back.
//! - When you edit someone, [`ContactSync::push`] sends it right away, in the
//!   background: the page doesn't wait for Google.
//!
//! Havre never deletes a contact from an address book.

use std::{collections::HashMap, sync::Arc, time::Duration};

use serde::Serialize;
use tokio::sync::{Mutex, Notify, broadcast};

use super::{
    Details, Key, People,
    books::{Book, Link, Photo, RemoteBook, RemoteContact},
    carddav, google, graph, now,
};
use crate::{
    Error, Result,
    agenda::store::CalDavLogin,
    oauth::http,
    oauth::{OAuthSession, Provider, Resource},
    store::Database,
    sync::{AccountEvent, SyncEvent},
};

const EVERY: Duration = Duration::from_secs(15 * 60);
/// A photo that wasn't there is looked for again after a week.
const PHOTO_RETRY: i64 = 7 * 86_400;

/// What an address book says changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    pub contacts: Vec<RemoteContact>,
    /// Remote ids of deleted contacts.
    pub deleted: Vec<String>,
    /// `contacts` is the whole book: any linked contact not in it is gone.
    pub full: bool,
    /// Where to start next time.
    pub token: Option<String>,
}

/// An account or CalDAV server, for Settings › People.
#[derive(Debug, Clone, Serialize)]
pub struct ContactSource {
    /// `account:3` or `caldav:2`.
    pub key: String,
    /// `microsoft`, `google` or `caldav`.
    pub kind: String,
    pub name: String,
    pub enabled: bool,
    /// The sign-in allows contacts (`false`: reconnect the account).
    pub allowed: bool,
    /// The account's id, to reconnect it.
    pub account: Option<i64>,
    pub books: Vec<Book>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContactSettings {
    pub sources: Vec<ContactSource>,
    /// Where details of someone in no address book go (`None`: Havre only).
    pub default_book: Option<i64>,
}

/// How to talk to one address book.
enum Access {
    Google(String),
    Microsoft(String),
    CardDav(CalDavLogin),
}

pub struct ContactSync {
    db: Database,
    people: People,
    events: broadcast::Sender<AccountEvent>,
    /// Graph / Google tokens, per mail account.
    sessions: Mutex<HashMap<i64, Arc<OAuthSession>>>,
    wake: Notify,
    /// One sync (or push) at a time.
    syncing: Mutex<()>,
}

impl ContactSync {
    /// Starts the background sync. `events` is the channel the web UI
    /// listens to (shared with Mail, the Agenda and Tasks).
    pub fn start(db: Database, events: broadcast::Sender<AccountEvent>) -> Arc<Self> {
        let sync = Arc::new(Self {
            people: db.people(),
            db,
            events,
            sessions: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            syncing: Mutex::new(()),
        });

        // An account added, reconnected or removed: fresh tokens, sync soon.
        // The loops hold a `Weak` reference, so they stop with Havre.
        let mut changes = sync.events.subscribe();
        let listener = Arc::downgrade(&sync);
        tokio::spawn(async move {
            loop {
                let change = match changes.recv().await {
                    Ok(change) => change,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let Some(sync) = listener.upgrade() else { break };
                if change.event == SyncEvent::Accounts {
                    sync.sessions.lock().await.remove(&change.account);
                    sync.wake.notify_one();
                }
            }
        });

        let background = Arc::downgrade(&sync);
        tokio::spawn(async move {
            loop {
                let Some(sync) = background.upgrade() else { break };
                if let Err(err) = sync.sync_all().await {
                    tracing::warn!("contacts sync: {err}");
                }
                let wake = async { sync.wake.notified().await };
                tokio::select! {
                    _ = tokio::time::sleep(EVERY) => {}
                    _ = wake => {}
                }
            }
        });
        sync
    }

    fn announce(&self) {
        let _ = self.events.send(AccountEvent { account: 0, event: SyncEvent::People });
    }

    // ---- Syncing -------------------------------------------------------------------------

    /// Every source: its address books, then what changed in each.
    pub async fn sync_all(&self) -> Result<()> {
        let _guard = self.syncing.lock().await;
        for account in self.db.accounts().await? {
            let Some(provider) = Provider::parse(&account.auth) else { continue };
            let source = provider.as_str();
            let key = format!("account:{}", account.id);
            if !account.can_contacts || !self.people.source_enabled(&key).await? {
                self.people.remove_books(source, Some(account.id), None).await?;
                continue;
            }
            // One address book per account: Google Contacts, or Outlook's main Contacts folder.
            let book = match provider {
                Provider::Google => RemoteBook {
                    remote_id: "people/me".into(),
                    name: "Google Contacts".into(),
                    read_only: false,
                },
                Provider::Microsoft => RemoteBook {
                    remote_id: "contacts".into(),
                    name: "Outlook contacts".into(),
                    read_only: false,
                },
            };
            let book = RemoteBook { name: format!("{} ({})", book.name, account.name), ..book };
            self.people.replace_books(source, Some(account.id), None, &[book]).await?;
        }
        for server in self.db.agenda().caldav_accounts().await? {
            let key = format!("caldav:{}", server.id);
            if !self.people.source_enabled(&key).await? {
                self.people.remove_books("carddav", None, Some(server.id)).await?;
                continue;
            }
            let listed = match self.db.agenda().caldav_login(server.id).await {
                Ok(login) => carddav::books(&login).await,
                Err(err) => Err(err),
            };
            match listed {
                Ok(books) => self.people.replace_books("carddav", None, Some(server.id), &books).await?,
                Err(err) => {
                    tracing::warn!(server = server.name, "can't list address books: {err}");
                    self.people.set_source_error("carddav", None, Some(server.id), &err.to_string()).await?;
                }
            }
        }

        for book in self.people.books().await? {
            let result = self.sync_book(&book).await;
            if let Err(err) = &result {
                tracing::warn!(book = book.name, "{err}");
                self.people.set_book_synced(book.id, None, Some(&err.to_string())).await?;
            }
        }
        self.announce();
        Ok(())
    }

    async fn sync_book(&self, book: &Book) -> Result<()> {
        let access = self.access(book).await?;
        let links = self.people.links(book.id).await?;
        let changes = match &access {
            Access::Google(token) => match google::changes(token, book.sync_token.as_deref()).await? {
                Some(changes) => Some(changes),
                // The token expired (a week without syncing): read it all again.
                None => google::changes(token, None).await?,
            },
            Access::Microsoft(token) => Some(graph::changes(token).await?),
            Access::CardDav(login) => {
                let known = links.iter().map(|l| (l.remote_id.clone(), l.etag.clone())).collect();
                carddav::changes(login, &book.remote_id, book.sync_token.as_deref(), &known).await?
            }
        };

        if let Some(changes) = &changes {
            self.people.apply_changes(book, &links, changes).await?;
        }

        // Havre's changes since the last sync go back.
        if !book.read_only {
            for (link, details) in self.people.pending(book.id).await? {
                if let Err(err) = self.send(&access, &link, &details).await {
                    tracing::warn!(book = book.name, "can't update a contact: {err}");
                }
            }
        }
        let token = changes.and_then(|c| c.token);
        self.people.set_book_synced(book.id, token.as_deref(), None).await
    }

    /// Writes Havre's details over one linked contact.
    async fn send(&self, access: &Access, link: &Link, details: &Details) -> Result<()> {
        let remote = match access {
            Access::Google(token) => {
                google::update(token, &link.remote_id, link.etag.as_deref(), details).await?
            }
            Access::Microsoft(token) => graph::update(token, &link.remote_id, details).await?,
            Access::CardDav(login) => {
                let href = link.href.as_deref().unwrap_or(&link.remote_id);
                let raw =
                    link.raw.as_deref().ok_or_else(|| Error::Config("this vCard wasn't read yet".into()))?;
                carddav::update(login, href, link.etag.as_deref(), raw, details).await?
            }
        };
        self.people.pushed(link, &remote).await
    }

    /// Sends one person's details to their address books now (after an
    /// edit). Someone in none goes to your default address book, if you
    /// chose one and there's something to put there.
    pub async fn push(&self, contact: i64) -> Result<()> {
        let _guard = self.syncing.lock().await;
        let (details, emails) = self.people.details_of(contact).await?;
        let links = self.people.links_of(contact).await?;
        if links.is_empty() {
            let Some(book) = self.people.default_book().await? else { return Ok(()) };
            if details == Details::default() {
                return Ok(());
            }
            let book = self.people.book(book).await?;
            let remote = match self.access(&book).await? {
                Access::Google(token) => google::create(&token, &details, &emails).await?,
                Access::Microsoft(token) => graph::create(&token, &details, &emails).await?,
                Access::CardDav(login) => carddav::create(&login, &book.remote_id, &details, &emails).await?,
            };
            self.people.link_new(contact, &book, &remote).await?;
        }
        for link in links.iter().filter(|l| l.synced != details) {
            let book = self.people.book(link.book).await?;
            if book.read_only {
                continue;
            }
            let access = self.access(&book).await?;
            if let Err(err) = self.send(&access, link, &details).await {
                self.people.set_book_synced(book.id, None, Some(&err.to_string())).await?;
                return Err(err);
            }
        }
        self.announce();
        Ok(())
    }

    /// [`ContactSync::push`] in the background, for the person behind `key`.
    pub fn push_later(self: &Arc<Self>, contact: Option<i64>) {
        let Some(contact) = contact else { return };
        let sync = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(err) = sync.push(contact).await {
                tracing::warn!("can't update an address book: {err}");
            }
        });
    }

    /// Syncs everything now and waits (the refresh button).
    pub async fn sync_now(&self) -> Result<()> {
        self.sessions.lock().await.clear();
        self.sync_all().await
    }

    async fn token(&self, account: i64, provider: Provider) -> Result<String> {
        let session = {
            let mut sessions = self.sessions.lock().await;
            match sessions.get(&account) {
                Some(session) => Arc::clone(session),
                None => {
                    let app = self.db.oauth_app(provider).cloned().ok_or_else(|| {
                        Error::Config(format!("OAuth2 with {} isn't set up in .env", provider.name()))
                    })?;
                    // The Graph / Google APIs token (the same as the Agenda's).
                    let session = Arc::new(OAuthSession::for_resource(
                        app,
                        account,
                        self.db.clone(),
                        Resource::Calendar,
                    ));
                    sessions.insert(account, Arc::clone(&session));
                    session
                }
            }
        };
        session.access_token().await
    }

    async fn access(&self, book: &Book) -> Result<Access> {
        Ok(match (book.source.as_str(), book.account, book.caldav) {
            ("google", Some(account), _) => Access::Google(self.token(account, Provider::Google).await?),
            ("microsoft", Some(account), _) => {
                Access::Microsoft(self.token(account, Provider::Microsoft).await?)
            }
            ("carddav", _, Some(server)) => Access::CardDav(self.db.agenda().caldav_login(server).await?),
            _ => return Err(Error::Config(format!("address book {} has no source", book.id))),
        })
    }

    // ---- Photos ------------------------------------------------------------------------------

    /// Someone's photo, from the first of their address books that has one
    /// (fetched once, then kept).
    pub async fn photo(&self, key: &str) -> Result<Option<Photo>> {
        let contact = match Key::parse(key)? {
            Key::Contact(id) => id,
            Key::Address(email) => match self.people.contact_of(&email).await? {
                Some(id) => id,
                None => return Ok(None),
            },
        };
        for source in self.people.photo_sources(contact).await? {
            match &source.stored {
                Some(photo) if !photo.data.is_empty() => return Ok(Some(photo.clone())),
                // Looked for recently, none.
                Some(_) if source.fetched_at.is_some_and(|at| at > now() - PHOTO_RETRY) => continue,
                _ => {}
            }
            let Some(url) = &source.photo_url else { continue };
            let fetched = match (source.source.as_str(), source.account) {
                ("microsoft", Some(account)) => {
                    let token = self.token(account, Provider::Microsoft).await?;
                    graph::photo(&token, &source.remote_id).await?
                }
                _ => fetch(url).await,
            };
            let (mime, data) = fetched.unwrap_or_default();
            self.people.store_photo(source.link, &mime, &data).await?;
            if !data.is_empty() && mime.starts_with("image/") {
                return Ok(Some(Photo { mime, data }));
            }
        }
        Ok(None)
    }

    // ---- Settings ----------------------------------------------------------------------

    pub async fn settings(&self) -> Result<ContactSettings> {
        let books = self.people.books().await?;
        let mine = |source: &str, account: Option<i64>, caldav: Option<i64>| {
            books
                .iter()
                .filter(|b| b.source == source && b.account == account && b.caldav == caldav)
                .cloned()
                .collect::<Vec<_>>()
        };
        let mut sources = Vec::new();
        for account in self.db.accounts().await? {
            let Some(provider) = Provider::parse(&account.auth) else { continue };
            let key = format!("account:{}", account.id);
            sources.push(ContactSource {
                enabled: self.people.source_enabled(&key).await?,
                key,
                kind: provider.as_str().to_owned(),
                name: account.name.clone(),
                allowed: account.can_contacts,
                account: Some(account.id),
                books: mine(provider.as_str(), Some(account.id), None),
            });
        }
        for server in self.db.agenda().caldav_accounts().await? {
            let key = format!("caldav:{}", server.id);
            sources.push(ContactSource {
                enabled: self.people.source_enabled(&key).await?,
                key,
                kind: "caldav".into(),
                name: server.name.clone(),
                allowed: true,
                account: None,
                books: mine("carddav", None, Some(server.id)),
            });
        }
        Ok(ContactSettings { sources, default_book: self.people.default_book().await? })
    }

    /// Turns contacts sync on or off for one account or server, and syncs.
    pub async fn set_source(&self, key: &str, enabled: bool) -> Result<ContactSettings> {
        if !self.settings().await?.sources.iter().any(|s| s.key == key) {
            return Err(Error::NotFound(format!("contact source {key}")));
        }
        self.people.set_source_enabled(key, enabled).await?;
        self.sync_all().await?;
        self.settings().await
    }

    pub async fn set_default_book(&self, book: Option<i64>) -> Result<ContactSettings> {
        self.people.set_default_book(book).await?;
        self.settings().await
    }
}

/// A photo at a public address (Google's, or a vCard's link).
async fn fetch(url: &str) -> Option<(String, Vec<u8>)> {
    let response = http().get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mime = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_owned();
    Some((mime, response.bytes().await.ok()?.to_vec()))
}
