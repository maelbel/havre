//! Keeps the Agenda in sync, and writes changes back to the providers.
//!
//! - Every 5 minutes (and on demand), each calendar source is asked for its
//!   list of calendars, then each shown calendar for its events from two
//!   months ago to a year ahead.
//! - Looking further back or ahead fetches that period on the spot.
//! - Creating, changing or deleting an event goes to the provider first
//!   (it decides, e.g. the event id); the local copy follows.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
use tokio::sync::{Mutex, Notify, broadcast};

use super::{
    AgendaStore, Calendar, Event, EventInput, RemoteCalendar, RemoteEvent, caldav, day_start, google, graph,
    ics,
    store::{CalDavAccount, CalDavLogin, CalendarRow},
    validate,
};
use crate::{
    Error, Result,
    oauth::{OAuthSession, Provider, Resource},
    store::Database,
    sync::{AccountEvent, SyncEvent},
};

const DAY: i64 = 86_400;
const EVERY: Duration = Duration::from_secs(5 * 60);

/// Where a calendar's events live.
enum Source {
    Local,
    Microsoft(String),
    Google(String),
    CalDav(CalDavLogin),
}

pub struct Agenda {
    db: Database,
    store: AgendaStore,
    events: broadcast::Sender<AccountEvent>,
    /// Calendar tokens, per mail account.
    sessions: Mutex<HashMap<i64, Arc<OAuthSession>>>,
    wake: Notify,
    /// One full sync at a time.
    syncing: Mutex<()>,
}

pub(crate) fn random_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or_default()
}

/// The period kept in sync in the background.
fn default_window() -> (i64, i64) {
    let today = day_start(now());
    (today - 60 * DAY, today + 366 * DAY)
}

impl Agenda {
    /// Starts the background sync. `events` is the channel the web UI
    /// listens to (shared with mail).
    pub fn start(db: Database, events: broadcast::Sender<AccountEvent>) -> Arc<Self> {
        let agenda = Arc::new(Self {
            store: db.agenda(),
            db,
            events,
            sessions: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            syncing: Mutex::new(()),
        });
        // An account was added, reconnected or removed: refresh soon, with
        // fresh tokens.
        let mut changes = agenda.events.subscribe();
        let listener = Arc::downgrade(&agenda);
        tokio::spawn(async move {
            loop {
                let change = match changes.recv().await {
                    Ok(change) => change,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let Some(agenda) = listener.upgrade() else { break };
                if change.event == SyncEvent::Accounts {
                    agenda.forget_account(change.account).await;
                }
            }
        });
        let background = Arc::clone(&agenda);
        tokio::spawn(async move {
            loop {
                if let Err(err) = background.sync_all().await {
                    tracing::warn!("agenda sync: {err}");
                }
                tokio::select! {
                    _ = tokio::time::sleep(EVERY) => {}
                    _ = background.wake.notified() => {}
                }
            }
        });
        agenda
    }

    pub fn store(&self) -> &AgendaStore {
        &self.store
    }

    fn announce(&self) {
        let _ = self.events.send(AccountEvent { account: 0, event: SyncEvent::Agenda });
    }

    /// Syncs soon, in the background (after an account change).
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    // ---- Reading ----------------------------------------------------------------------

    /// Events in `[from, to)`. Fetches the period first if it was never
    /// synced (looking far back or ahead).
    pub async fn events(&self, from: i64, to: i64) -> Result<Vec<Event>> {
        if to <= from || to - from > 120 * DAY {
            return Err(Error::Config("ask for at most 120 days at a time".into()));
        }
        for row in self.store.calendar_rows().await? {
            let covered = matches!((row.sync_from, row.sync_to), (Some(a), Some(b)) if a <= from && b >= to);
            if row.calendar.enabled
                && row.calendar.source != "local"
                && !covered
                && row.calendar.error.is_none()
            {
                // Fetch a bit more than asked, to make browsing smooth.
                let (wanted_from, wanted_to) = (day_start(from) - 31 * DAY, day_start(to) + 32 * DAY);
                if let Err(err) = self.sync_calendar(&row, wanted_from, wanted_to).await {
                    tracing::warn!(calendar = row.calendar.name, "{err}");
                }
            }
        }
        self.store.events(from, to).await
    }

    // ---- Syncing ---------------------------------------------------------------------------

    /// Refreshes every source: its list of calendars, then their events.
    pub async fn sync_all(&self) -> Result<()> {
        let _guard = self.syncing.lock().await;
        let (from, to) = default_window();
        let mut changed = false;

        for account in self.db.accounts().await? {
            let Some(provider) = Provider::parse(&account.auth) else { continue };
            if !account.can_calendar {
                // Signed in before the Agenda existed: nothing to show until
                // the account is reconnected.
                self.store.remove_account_calendars(account.id).await?;
                continue;
            }
            let listed = match self.token(account.id, provider).await {
                Ok(token) => match provider {
                    Provider::Microsoft => graph::calendars(&token).await,
                    Provider::Google => google::calendars(&token).await,
                },
                Err(err) => Err(err),
            };
            changed |= self.apply_listing(provider.as_str(), Some(account.id), None, listed).await?;
        }
        for server in self.store.caldav_accounts().await? {
            let listed = match self.store.caldav_login(server.id).await {
                Ok(login) => caldav::calendars(&login).await,
                Err(err) => Err(err),
            };
            changed |= self.apply_listing("caldav", None, Some(server.id), listed).await?;
        }

        for row in self.store.calendar_rows().await? {
            if row.calendar.enabled && row.calendar.source != "local" {
                match self.sync_calendar(&row, from, to).await {
                    Ok(()) => changed = true,
                    Err(err) => {
                        tracing::warn!(calendar = row.calendar.name, "{err}");
                        self.store.set_error(row.calendar.id, Some(&err.to_string())).await?;
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.announce();
        }
        Ok(())
    }

    /// Saves a source's calendar list, or its error on its calendars.
    async fn apply_listing(
        &self,
        source: &str,
        account: Option<i64>,
        caldav: Option<i64>,
        listed: Result<Vec<RemoteCalendar>>,
    ) -> Result<bool> {
        match listed {
            Ok(calendars) => {
                self.store.replace_calendars(source, account, caldav, &calendars).await?;
                Ok(true)
            }
            Err(err) => {
                tracing::warn!(source, "can't list calendars: {err}");
                for row in self.store.calendar_rows().await? {
                    if row.calendar.account == account
                        && row.calendar.caldav == caldav
                        && row.calendar.source == source
                    {
                        self.store.set_error(row.calendar.id, Some(&err.to_string())).await?;
                    }
                }
                Ok(false)
            }
        }
    }

    async fn sync_calendar(&self, row: &CalendarRow, from: i64, to: i64) -> Result<()> {
        let read_only = row.calendar.read_only;
        let id = &row.remote_id;
        let events: Vec<RemoteEvent> = match self.source(&row.calendar).await? {
            Source::Local => return Ok(()),
            Source::Microsoft(token) => graph::events(&token, id, from, to, read_only).await?,
            Source::Google(token) => google::events(&token, id, from, to, read_only).await?,
            Source::CalDav(login) => caldav::events(&login, id, from, to, read_only).await?,
        };
        self.store.replace_window(row.calendar.id, from, to, &events).await
    }

    /// Syncs everything now and waits (the refresh button).
    pub async fn sync_now(&self) -> Result<()> {
        // Clear errors: try again even calendars that failed.
        for row in self.store.calendar_rows().await? {
            self.store.set_error(row.calendar.id, None).await?;
        }
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

    /// Forgets cached tokens (after a reconnect, the old ones are stale).
    pub async fn forget_account(&self, account: i64) {
        self.sessions.lock().await.remove(&account);
        self.wake();
    }

    async fn source(&self, calendar: &Calendar) -> Result<Source> {
        Ok(match (calendar.source.as_str(), calendar.account, calendar.caldav) {
            ("local", _, _) => Source::Local,
            ("microsoft", Some(account), _) => {
                Source::Microsoft(self.token(account, Provider::Microsoft).await?)
            }
            ("google", Some(account), _) => Source::Google(self.token(account, Provider::Google).await?),
            ("caldav", _, Some(server)) => Source::CalDav(self.store.caldav_login(server).await?),
            _ => return Err(Error::Config(format!("calendar {} has no source", calendar.id))),
        })
    }

    // ---- Writing ---------------------------------------------------------------------------

    pub async fn create(&self, input: &EventInput) -> Result<Event> {
        validate(input)?;
        let row = self.store.calendar_row(input.calendar).await?;
        if row.calendar.read_only {
            return Err(Error::Config(format!("“{}” is read-only.", row.calendar.name)));
        }
        let remote = match self.source(&row.calendar).await? {
            Source::Local => local_event(&format!("local-{}", random_id()), input),
            Source::Microsoft(token) => graph::create(&token, &row.remote_id, input).await?,
            Source::Google(token) => google::create(&token, &row.remote_id, input).await?,
            Source::CalDav(login) => caldav::create(&login, &row.remote_id, input).await?,
        };
        let event = self.store.save_event(row.calendar.id, &remote).await?;
        self.announce();
        Ok(event)
    }

    /// Changes an event. Moving it to another calendar creates it there and
    /// deletes the original.
    pub async fn update(&self, id: i64, input: &EventInput) -> Result<Event> {
        validate(input)?;
        let current = self.store.event_row(id).await?;
        if current.event.read_only {
            return Err(Error::Config("This event can't be changed from Havre.".into()));
        }
        if input.calendar != current.event.calendar {
            let moved = self.create(input).await?;
            self.delete(id).await?;
            return Ok(moved);
        }
        let row = self.store.calendar_row(current.event.calendar).await?;
        let remote = match self.source(&row.calendar).await? {
            Source::Local => local_event(&current.remote_id, input),
            Source::Microsoft(token) => graph::update(&token, &current.remote_id, input).await?,
            Source::Google(token) => {
                google::update(&token, &row.remote_id, &current.remote_id, input).await?
            }
            Source::CalDav(login) => {
                let href =
                    current.href.as_deref().ok_or_else(|| Error::Config("missing CalDAV address".into()))?;
                let raw =
                    current.raw.as_deref().ok_or_else(|| Error::Config("missing CalDAV event".into()))?;
                caldav::update(&login, href, current.etag.as_deref(), raw, input).await?
            }
        };
        if remote.remote_id != current.remote_id {
            self.store.delete_event(id).await?;
        }
        let event = self.store.save_event(row.calendar.id, &remote).await?;
        self.announce();
        Ok(event)
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        let current = self.store.event_row(id).await?;
        if current.event.read_only {
            return Err(Error::Config("This event can't be deleted from Havre.".into()));
        }
        let row = self.store.calendar_row(current.event.calendar).await?;
        match self.source(&row.calendar).await? {
            Source::Local => {}
            Source::Microsoft(token) => graph::delete(&token, &current.remote_id).await?,
            Source::Google(token) => google::delete(&token, &row.remote_id, &current.remote_id).await?,
            Source::CalDav(login) => {
                if let Some(href) = &current.href {
                    caldav::delete(&login, href, current.etag.as_deref()).await?;
                }
            }
        }
        self.store.delete_event(id).await?;
        self.announce();
        Ok(())
    }

    /// Answers an invitation that's in a Microsoft or Google calendar (they
    /// tell the organizer). `response`: accepted, tentative, declined.
    pub async fn respond(&self, id: i64, response: &str) -> Result<Event> {
        let current = self.store.event_row(id).await?;
        if current.event.my_response.is_none() {
            return Err(Error::Config("You organize this event: there's nothing to answer.".into()));
        }
        let row = self.store.calendar_row(current.event.calendar).await?;
        match self.source(&row.calendar).await? {
            Source::Microsoft(token) => graph::respond(&token, &current.remote_id, response).await?,
            Source::Google(token) => {
                google::respond(&token, &row.remote_id, &current.remote_id, response).await?
            }
            _ => return Err(Error::Config("Answer this invitation from its email.".into())),
        }
        self.store.set_my_response(id, response).await?;
        if let Some(uid) = &current.event.uid {
            self.store.remember_answer(uid, response).await?;
        }
        self.announce();
        self.store.event(id).await
    }

    /// Adds the events of an invitation (`.ics` from an email) to a
    /// calendar (the default one when `None`).
    pub async fn add_from_ics(&self, text: &str, calendar: Option<i64>) -> Result<Vec<Event>> {
        let calendar = match calendar {
            Some(id) => id,
            None => self
                .store
                .calendars()
                .await?
                .into_iter()
                .find(|c| c.is_default)
                .map(|c| c.id)
                .ok_or_else(|| Error::Config("no calendar to add it to".into()))?,
        };
        let found = ics::invitation(text)?;
        if found.is_empty() {
            return Err(Error::Config("This invitation has no event.".into()));
        }
        let mut added = Vec::new();
        for event in found {
            let uid = event.uid.clone();
            let organizer_email = event.organizer.as_deref().and_then(ics::email_in);
            // Your copy has you as organizer: say whose meeting it is.
            let description = match &event.organizer {
                Some(who) if event.description.trim().is_empty() => format!("Organized by {who}."),
                Some(who) => format!("Organized by {who}.\n\n{}", event.description),
                None => event.description,
            };
            let input = EventInput {
                calendar,
                title: event.title,
                location: event.location,
                description,
                start: event.start,
                end: event.end.max(event.start + 60),
                all_day: event.all_day,
                attendees: Vec::new(),
            };
            let created = self.create(&input).await?;
            // Remember which meeting it is a copy of (the invitation card
            // in Mail finds it this way).
            if let Some(uid) = uid {
                self.store.remember_copy(created.id, &uid, organizer_email.as_deref()).await?;
            }
            added.push(created);
        }
        Ok(added)
    }

    // ---- CalDAV servers ---------------------------------------------------------------------

    /// Checks the address and login (by listing the calendars), then saves.
    pub async fn add_caldav(
        &self,
        name: &str,
        url: &str,
        username: &str,
        password: &str,
    ) -> Result<CalDavAccount> {
        let url = url.trim().trim_end_matches('/').to_owned();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(Error::Config("The address starts with https://".into()));
        }
        let login = CalDavLogin {
            url: url.clone(),
            username: username.trim().to_owned(),
            password: password.to_owned(),
        };
        let calendars = caldav::calendars(&login).await?;
        let name = if name.trim().is_empty() { super::caldav_name(&url) } else { name.trim().to_owned() };
        let id = self.store.insert_caldav(&name, &url, &login.username, password).await?;
        self.store.replace_calendars("caldav", None, Some(id), &calendars).await?;
        self.announce();
        self.wake();
        Ok(CalDavAccount { id, name, url, username: login.username })
    }

    pub async fn remove_caldav(&self, id: i64) -> Result<()> {
        self.store.delete_caldav(id).await?;
        self.announce();
        Ok(())
    }

    pub async fn update_calendar(
        &self,
        id: i64,
        enabled: Option<bool>,
        color: Option<&str>,
        is_default: Option<bool>,
    ) -> Result<Calendar> {
        let calendar = self.store.update_calendar(id, enabled, color, is_default).await?;
        self.announce();
        Ok(calendar)
    }
}

fn local_event(remote_id: &str, input: &EventInput) -> RemoteEvent {
    RemoteEvent {
        remote_id: remote_id.to_owned(),
        title: input.title.trim().to_owned(),
        location: input.location.trim().to_owned(),
        description: input.description.clone(),
        start: input.start,
        end: input.end,
        all_day: input.all_day,
        attendees: input
            .attendees
            .iter()
            .map(|email| super::Attendee {
                name: None,
                email: email.trim().to_owned(),
                status: "needs_action".into(),
            })
            .collect(),
        ..RemoteEvent::default()
    }
}
