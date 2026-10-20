//! Keeps the Agenda in sync, and writes changes back to the providers.
//!
//! - Every 5 minutes (and on demand), each calendar source is asked for its
//!   list of calendars, then each shown calendar for its events from two
//!   months ago to a year ahead.
//! - Looking further back or ahead fetches that period on the spot.
//! - Creating, changing or deleting an event goes to the provider first
//!   (it decides, e.g. the event id); the local copy follows.
//! - Repeating events: a change goes to the series as the provider keeps it
//!   ([`Scope`]: only this occurrence, this and the following ones, or all),
//!   then the calendar's occurrences are read again, so each one is right.
//!   Havre's own calendar keeps each series as iCalendar text and works out
//!   the occurrences itself.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
use chrono_tz::Tz;
use tokio::sync::{Mutex, Notify, broadcast};

use super::{
    AgendaStore, Calendar, Event, EventInput, RemoteCalendar, RemoteEvent, Scope, SeriesRule, caldav,
    day_start, google, graph, ics,
    repeat::{self, Repeat},
    store::{CalDavAccount, CalDavLogin, CalendarRow, EventRow, SeriesRow},
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
/// A series' rule is read again from Microsoft or Google after this long.
const RULE_MAX_AGE: i64 = 6 * 3600;
/// At most this many series rules read per calendar and sync.
const RULES_PER_SYNC: u32 = 50;

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
            if row.calendar.enabled && !covered && row.calendar.error.is_none() {
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
            if row.calendar.enabled {
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
        let (calendar, id) = (row.calendar.id, &row.remote_id);
        match self.source(&row.calendar).await? {
            Source::Local => {
                // Havre's calendar: its repeating events' occurrences, worked
                // out from each series' text (its other events are just rows).
                let mut occurrences = Vec::new();
                for series in self.store.local_series(calendar).await? {
                    let text = series.ics.unwrap_or_default();
                    match ics::events_between(&text, None, None, from, to, false) {
                        Ok(found) => {
                            occurrences.extend(found.into_iter().map(|e| RemoteEvent { raw: None, ..e }))
                        }
                        Err(err) => tracing::warn!(series = series.rule.series, "unreadable series: {err}"),
                    }
                }
                self.store.replace_window(calendar, from, to, &occurrences, true).await
            }
            Source::Microsoft(token) => {
                let events = graph::events(&token, id, from, to, read_only).await?;
                self.store.replace_window(calendar, from, to, &events, false).await?;
                for series in self.store.series_to_read(calendar, RULE_MAX_AGE, RULES_PER_SYNC).await? {
                    match graph::series(&token, &series).await {
                        Ok(rule) => self.store.save_series(calendar, &rule, None).await?,
                        Err(err) => {
                            tracing::warn!(calendar = row.calendar.name, "can't read a series: {err}")
                        }
                    }
                }
                Ok(())
            }
            Source::Google(token) => {
                let events = google::events(&token, id, from, to, read_only).await?;
                self.store.replace_window(calendar, from, to, &events, false).await?;
                for series in self.store.series_to_read(calendar, RULE_MAX_AGE, RULES_PER_SYNC).await? {
                    match google::series(&token, id, &series).await {
                        Ok(rule) => self.store.save_series(calendar, &rule, None).await?,
                        Err(err) => {
                            tracing::warn!(calendar = row.calendar.name, "can't read a series: {err}")
                        }
                    }
                }
                Ok(())
            }
            // iCalendar gives each series' rule with its occurrences.
            Source::CalDav(login) => {
                let events = caldav::events(&login, id, from, to, read_only).await?;
                self.store.replace_window(calendar, from, to, &events, false).await
            }
        }
    }

    /// After changing a repeating event: the calendar's occurrences are read
    /// again over the whole period kept in sync (nothing is guessed locally).
    /// The change itself is done, so a failure here is only logged: the next
    /// sync catches up.
    async fn refresh(&self, calendar: i64) {
        let refreshed = async {
            let row = self.store.calendar_row(calendar).await?;
            let (from, to) = default_window();
            let from = row.sync_from.map_or(from, |f| f.min(from));
            let to = row.sync_to.map_or(to, |t| t.max(to));
            self.sync_calendar(&row, from, to).await
        };
        if let Err(err) = refreshed.await {
            tracing::warn!(calendar, "reading the calendar again: {err}");
        }
    }

    /// After a series changed: its rule is read again, its occurrences too;
    /// returns the occurrence nearest `near` (the one that was edited).
    async fn after_series_change(&self, calendar: i64, series: &str, near: i64) -> Result<Event> {
        self.store.forget_rule(calendar, series).await?;
        self.refresh(calendar).await;
        self.announce();
        self.store.occurrence_near(calendar, series, near).await?.ok_or_else(|| {
            Error::Config(
                "Saved. Its dates are outside the period the Agenda keeps: refresh to see them.".into(),
            )
        })
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
        let tz = repeat::zone(input.time_zone.as_deref())?;
        if let Some(rule) = &input.repeat {
            return self.create_series(&row, input, rule, tz).await;
        }
        let remote = match self.source(&row.calendar).await? {
            Source::Local => local_event(&format!("local-{}", random_id()), input),
            Source::Microsoft(token) => graph::create(&token, &row.remote_id, input, None).await?,
            Source::Google(token) => google::create(&token, &row.remote_id, input, None).await?,
            Source::CalDav(login) => caldav::create(&login, &row.remote_id, input, None).await?,
        };
        let event = self.store.save_event(row.calendar.id, &remote).await?;
        self.announce();
        Ok(event)
    }

    /// A new repeating event: the provider makes the series, then its
    /// occurrences are read.
    async fn create_series(
        &self,
        row: &CalendarRow,
        input: &EventInput,
        rule: &Repeat,
        tz: Tz,
    ) -> Result<Event> {
        let series = match self.source(&row.calendar).await? {
            Source::Local => {
                let uid = format!("local-{}", random_id());
                let text = ics::new_series_ics(&uid, input, rule, tz);
                self.store
                    .save_series(row.calendar.id, &local_rule(&uid, input, rule, tz), Some(&text))
                    .await?;
                uid
            }
            Source::Microsoft(token) => {
                graph::create(&token, &row.remote_id, input, Some(tz)).await?.remote_id
            }
            Source::Google(token) => google::create(&token, &row.remote_id, input, Some(tz)).await?.remote_id,
            Source::CalDav(login) => {
                let created = caldav::create(&login, &row.remote_id, input, Some(tz)).await?;
                created.series.unwrap_or(created.remote_id)
            }
        };
        self.after_series_change(row.calendar.id, &series, input.start).await
    }

    /// Changes an event. Moving it to another calendar creates it there and
    /// deletes the original. A repeating event changes for `scope`.
    pub async fn update(&self, id: i64, input: &EventInput, scope: Scope) -> Result<Event> {
        validate(input)?;
        let current = self.store.event_row(id).await?;
        if current.event.read_only {
            return Err(Error::Config("This event can't be changed from Havre.".into()));
        }
        let tz = repeat::zone(input.time_zone.as_deref())?;
        let Some(series) = current.series.clone() else {
            return self.update_single(&current, input, tz).await;
        };
        let row = self.store.calendar_row(current.event.calendar).await?;
        let rule = self.store.series(row.calendar.id, &series).await?;
        let editable = rule.as_ref().is_some_and(|r| r.rule.repeat.is_some());
        let original = current.original_start.unwrap_or(current.event.start);
        let scope = effective_scope(scope, &current, rule.as_ref(), original);

        if input.calendar != current.event.calendar {
            if scope != Scope::All {
                return Err(Error::Config(
                    "A repeating event moves to another calendar as a whole: choose “All events”.".into(),
                ));
            }
            if !editable {
                return Err(Error::Config(
                    "This event repeats in a way set in another app: move it there.".into(),
                ));
            }
            let moved = self.create(input).await?;
            self.delete(id, Scope::All).await?;
            return Ok(moved);
        }

        match scope {
            Scope::This => self.update_occurrence(&row, &current, &series, rule, original, input).await,
            Scope::Following => {
                let Some(SeriesRow { rule: old, .. }) = rule.filter(|_| editable) else {
                    return Err(Error::Config(
                        "This event repeats in a way set in another app, so Havre can't split it: change only \
                         this event, or all of them."
                            .into(),
                    ));
                };
                // The following ones become a series of their own; one that
                // ends "after 10 times" keeps counting from the start.
                let mut input = input.clone();
                if let (Some(asked), Some(was)) = (input.repeat.as_mut(), old.repeat.as_ref())
                    && asked.clone().normalized() == *was
                    && let Some(count) = was.count
                {
                    let zone = repeat::zone(old.time_zone.as_deref()).unwrap_or(tz);
                    let before = was.occurrences_before(old.first_start, original, old.all_day, zone);
                    asked.count = Some(count.saturating_sub(before).max(1));
                }
                self.end_series_before(&row, &current, &series, original).await?;
                self.forget_after_write(row.calendar.id, &series).await?;
                self.create(&input).await
            }
            Scope::All => {
                if input.repeat.is_none() && editable {
                    // It stops repeating: one event in place of the series.
                    let single = self.create(input).await?;
                    self.delete(id, Scope::All).await?;
                    return Ok(single);
                }
                // A rule made elsewhere is kept as it is.
                let new_rule = input.repeat.as_ref().filter(|_| editable);
                self.update_series(&row, &current, &series, rule, original, input, new_rule, tz).await
            }
        }
    }

    /// An event that doesn't repeat (yet: `input.repeat` makes it repeat).
    async fn update_single(&self, current: &EventRow, input: &EventInput, tz: Tz) -> Result<Event> {
        let id = current.event.id;
        if input.calendar != current.event.calendar {
            let moved = self.create(input).await?;
            self.delete(id, Scope::This).await?;
            return Ok(moved);
        }
        let row = self.store.calendar_row(current.event.calendar).await?;
        if let Some(rule) = &input.repeat {
            // It becomes a series: the provider keeps the same event, now
            // with a rule (Havre's calendar makes a new series in its place).
            let series = match self.source(&row.calendar).await? {
                Source::Local => {
                    let created = self.create_series(&row, input, rule, tz).await?;
                    self.store.delete_event(id).await?;
                    return Ok(created);
                }
                Source::Microsoft(token) => {
                    graph::update_series(&token, &current.remote_id, input, Some(rule), tz).await?;
                    current.remote_id.clone()
                }
                Source::Google(token) => {
                    google::update_series(
                        &token,
                        &row.remote_id,
                        &current.remote_id,
                        input,
                        Some(rule),
                        tz,
                        true,
                    )
                    .await?;
                    current.remote_id.clone()
                }
                Source::CalDav(login) => {
                    let (href, raw) = caldav_resource(current)?;
                    let text = ics::update_master(raw, input, current.event.start, Some(rule), tz)?;
                    caldav::put(&login, href, current.etag.as_deref(), &text).await?;
                    current.event.uid.clone().unwrap_or_else(|| current.remote_id.clone())
                }
            };
            return self.after_series_change(row.calendar.id, &series, input.start).await;
        }
        let remote = match self.source(&row.calendar).await? {
            Source::Local => local_event(&current.remote_id, input),
            Source::Microsoft(token) => graph::update(&token, &current.remote_id, input).await?,
            Source::Google(token) => {
                google::update(&token, &row.remote_id, &current.remote_id, input).await?
            }
            Source::CalDav(login) => {
                let (href, raw) = caldav_resource(current)?;
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

    /// *Only this event*: Microsoft and Google change that occurrence;
    /// iCalendar gets a modified occurrence in the series' text.
    async fn update_occurrence(
        &self,
        row: &CalendarRow,
        current: &EventRow,
        series: &str,
        rule: Option<SeriesRow>,
        original: i64,
        input: &EventInput,
    ) -> Result<Event> {
        match self.source(&row.calendar).await? {
            Source::Local => {
                let stored = rule.ok_or_else(missing_series)?;
                let text =
                    ics::override_occurrence(stored.ics.as_deref().unwrap_or_default(), original, input)?;
                self.store.save_series(row.calendar.id, &stored.rule, Some(&text)).await?;
            }
            Source::Microsoft(token) => {
                graph::update(&token, &current.remote_id, input).await?;
            }
            Source::Google(token) => {
                google::update(&token, &row.remote_id, &current.remote_id, input).await?;
            }
            Source::CalDav(login) => {
                let (href, raw) = caldav_resource(current)?;
                let text = ics::override_occurrence(raw, original, input)?;
                caldav::put(&login, href, current.etag.as_deref(), &text).await?;
            }
        }
        self.after_series_change(row.calendar.id, series, input.start).await
    }

    /// *All events*: the series takes the edited occurrence's fields and
    /// moves as it did (see [`repeat::shift`]).
    #[allow(clippy::too_many_arguments)]
    async fn update_series(
        &self,
        row: &CalendarRow,
        current: &EventRow,
        series: &str,
        rule: Option<SeriesRow>,
        original: i64,
        input: &EventInput,
        new_rule: Option<&Repeat>,
        tz: Tz,
    ) -> Result<Event> {
        let first_start = rule.as_ref().map_or(original, |r| r.rule.first_start);
        // The series' own time zone, else the browser's.
        let zone = rule
            .as_ref()
            .and_then(|r| r.rule.time_zone.as_deref())
            .and_then(|name| repeat::zone(Some(name)).ok())
            .unwrap_or(tz);
        let start = repeat::shift(first_start, original, input.start, input.all_day, zone);
        let moved = EventInput { start, end: start + (input.end - input.start), ..input.clone() };
        match self.source(&row.calendar).await? {
            Source::Local => {
                let stored = rule.ok_or_else(missing_series)?;
                let text = ics::update_master(
                    stored.ics.as_deref().unwrap_or_default(),
                    input,
                    original,
                    new_rule,
                    tz,
                )?;
                self.store.save_series(row.calendar.id, &stored.rule, Some(&text)).await?;
            }
            Source::Microsoft(token) => graph::update_series(&token, series, &moved, new_rule, zone).await?,
            Source::Google(token) => {
                let shifted = start != first_start;
                google::update_series(&token, &row.remote_id, series, &moved, new_rule, zone, shifted).await?
            }
            Source::CalDav(login) => {
                let (href, raw) = caldav_resource(current)?;
                let text = ics::update_master(raw, input, original, new_rule, tz)?;
                caldav::put(&login, href, current.etag.as_deref(), &text).await?;
            }
        }
        self.after_series_change(row.calendar.id, series, input.start).await
    }

    /// Ends a series just before an occurrence (or deletes it, when that
    /// is its first one).
    async fn end_series_before(
        &self,
        row: &CalendarRow,
        current: &EventRow,
        series: &str,
        original: i64,
    ) -> Result<()> {
        let calendar = row.calendar.id;
        match self.source(&row.calendar).await? {
            Source::Local => {
                let stored = self.store.series(calendar, series).await?.ok_or_else(missing_series)?;
                match ics::cut(stored.ics.as_deref().unwrap_or_default(), original)? {
                    Some(text) => self.store.save_series(calendar, &stored.rule, Some(&text)).await?,
                    None => self.store.delete_series(calendar, series).await?,
                }
            }
            Source::Microsoft(token) => {
                if !graph::cut_series(&token, series, original, current.event.all_day).await? {
                    graph::delete(&token, series).await?;
                }
            }
            Source::Google(token) => {
                if !google::cut_series(&token, &row.remote_id, series, original).await? {
                    google::delete(&token, &row.remote_id, series).await?;
                }
            }
            Source::CalDav(login) => {
                let (href, raw) = caldav_resource(current)?;
                match ics::cut(raw, original)? {
                    Some(text) => {
                        caldav::put(&login, href, current.etag.as_deref(), &text).await?;
                    }
                    None => caldav::delete(&login, href, current.etag.as_deref()).await?,
                }
            }
        }
        Ok(())
    }

    /// The series changed: its rule and occurrences are read again.
    async fn forget_after_write(&self, calendar: i64, series: &str) -> Result<()> {
        self.store.forget_rule(calendar, series).await?;
        self.refresh(calendar).await;
        Ok(())
    }

    /// Deletes an event; a repeating one for `scope`.
    pub async fn delete(&self, id: i64, scope: Scope) -> Result<()> {
        let current = self.store.event_row(id).await?;
        if current.event.read_only {
            return Err(Error::Config("This event can't be deleted from Havre.".into()));
        }
        let row = self.store.calendar_row(current.event.calendar).await?;
        let calendar = row.calendar.id;
        let Some(series) = current.series.clone() else {
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
            return Ok(());
        };
        let rule = self.store.series(calendar, &series).await?;
        let original = current.original_start.unwrap_or(current.event.start);
        match effective_scope(scope, &current, rule.as_ref(), original) {
            Scope::This => match self.source(&row.calendar).await? {
                Source::Local => {
                    let stored = rule.ok_or_else(missing_series)?;
                    let text = ics::exclude(stored.ics.as_deref().unwrap_or_default(), original)?;
                    self.store.save_series(calendar, &stored.rule, Some(&text)).await?;
                }
                Source::Microsoft(token) => graph::delete(&token, &current.remote_id).await?,
                Source::Google(token) => google::delete(&token, &row.remote_id, &current.remote_id).await?,
                Source::CalDav(login) => {
                    let (href, raw) = caldav_resource(&current)?;
                    let text = ics::exclude(raw, original)?;
                    caldav::put(&login, href, current.etag.as_deref(), &text).await?;
                }
            },
            Scope::Following => self.end_series_before(&row, &current, &series, original).await?,
            Scope::All => {
                match self.source(&row.calendar).await? {
                    Source::Local => {}
                    Source::Microsoft(token) => graph::delete(&token, &series).await?,
                    Source::Google(token) => google::delete(&token, &row.remote_id, &series).await?,
                    Source::CalDav(login) => {
                        let (href, _) = caldav_resource(&current)?;
                        caldav::delete(&login, href, current.etag.as_deref()).await?;
                    }
                }
                self.store.delete_series(calendar, &series).await?;
            }
        }
        self.store.delete_event(id).await?;
        self.forget_after_write(calendar, &series).await?;
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
                repeat: None,
                time_zone: None,
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

/// On the first occurrence, "this and the following ones" is all of them.
fn effective_scope(scope: Scope, current: &EventRow, rule: Option<&SeriesRow>, original: i64) -> Scope {
    let first = current.event.first_occurrence || rule.is_some_and(|r| original <= r.rule.first_start);
    if scope == Scope::Following && first { Scope::All } else { scope }
}

/// A CalDAV event's file: its address and text.
fn caldav_resource(current: &EventRow) -> Result<(&str, &str)> {
    let href = current.href.as_deref().ok_or_else(|| Error::Config("missing CalDAV address".into()))?;
    let raw = current.raw.as_deref().ok_or_else(|| Error::Config("missing CalDAV event".into()))?;
    Ok((href, raw))
}

fn missing_series() -> Error {
    Error::Config("This repeating event is no longer in Havre: refresh the Agenda.".into())
}

/// A series of Havre's calendar, before its text is read back.
fn local_rule(uid: &str, input: &EventInput, rule: &Repeat, tz: Tz) -> SeriesRule {
    SeriesRule {
        series: uid.to_owned(),
        rrule: rule.rrule(input.all_day, tz),
        repeat: Some(rule.clone().normalized()),
        time_zone: (!input.all_day).then(|| tz.name().to_owned()),
        first_start: input.start,
        first_end: input.end,
        all_day: input.all_day,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{accounts::AccountManager, agenda::repeat::Freq};

    async fn agenda() -> (Arc<Agenda>, i64) {
        let db = Database::in_memory().await.unwrap();
        let accounts = AccountManager::start(db.clone()).await.unwrap();
        let agenda = Agenda::start(db, accounts.sender());
        let local =
            agenda.store().calendars().await.unwrap().into_iter().find(|c| c.source == "local").unwrap();
        (agenda, local.id)
    }

    fn weekly(count: u32) -> Repeat {
        Repeat { freq: Freq::Weekly, interval: 1, days: vec![], on: None, until: None, count: Some(count) }
    }

    fn input(calendar: i64, title: &str, start: i64, repeat: Option<Repeat>) -> EventInput {
        EventInput {
            calendar,
            title: title.into(),
            location: String::new(),
            description: String::new(),
            start,
            end: start + 1800,
            all_day: false,
            attendees: vec![],
            repeat,
            time_zone: Some("Europe/Paris".into()),
        }
    }

    /// The series' occurrences in the next two months: (title, start).
    async fn shown(agenda: &Agenda, from: i64) -> Vec<(String, i64)> {
        let events = agenda.events(from - DAY, from + 70 * DAY).await.unwrap();
        events.into_iter().filter(|e| e.recurring).map(|e| (e.title, e.start)).collect()
    }

    #[tokio::test]
    async fn repeating_events_in_havres_calendar() {
        let (agenda, calendar) = agenda().await;
        // Next week, 08:00 UTC, every week, 5 times.
        let start = day_start(now()) + 7 * DAY + 8 * 3600;
        let week = 7 * DAY;
        let first = agenda.create(&input(calendar, "Check-in", start, Some(weekly(5)))).await.unwrap();
        assert!(first.recurring && first.first_occurrence);
        assert_eq!(first.repeat.as_ref().and_then(|r| r.count), Some(5));
        assert_eq!(shown(&agenda, start).await.len(), 5);
        let id_of = |start: i64| {
            let agenda = Arc::clone(&agenda);
            async move {
                let events = agenda.events(start - DAY, start + DAY).await.unwrap();
                events.into_iter().find(|e| e.recurring && (e.start - start).abs() < 4 * 3600).unwrap()
            }
        };

        // Only the second one, an hour later.
        let second = id_of(start + week).await;
        assert!(!second.first_occurrence);
        let moved = agenda
            .update(second.id, &input(calendar, "Moved", second.start + 3600, None), Scope::This)
            .await
            .unwrap();
        assert_eq!(moved.title, "Moved");
        let titles: Vec<String> = shown(&agenda, start).await.into_iter().map(|(t, _)| t).collect();
        assert_eq!(titles, ["Check-in", "Moved", "Check-in", "Check-in", "Check-in"]);

        // The third and the following ones: a new series, still 5 in all.
        let third = id_of(start + 2 * week).await;
        let later = input(calendar, "Later", third.start, Some(weekly(5)));
        let new = agenda.update(third.id, &later, Scope::Following).await.unwrap();
        assert_eq!(new.repeat.as_ref().and_then(|r| r.count), Some(3), "counts on from the old series");
        let titles: Vec<String> = shown(&agenda, start).await.into_iter().map(|(t, _)| t).collect();
        assert_eq!(titles, ["Check-in", "Moved", "Later", "Later", "Later"]);

        // All of the first series renamed (from its first occurrence).
        let first = id_of(start).await;
        // (The editor sends the rule it shows: since the split, it ends
        // the day before the third one.)
        assert_eq!(first.repeat.as_ref().map(|r| (r.count, r.until.is_some())), Some((None, true)));
        agenda
            .update(first.id, &input(calendar, "Weekly", first.start, first.repeat.clone()), Scope::All)
            .await
            .unwrap();
        let titles: Vec<String> = shown(&agenda, start).await.into_iter().map(|(t, _)| t).collect();
        assert_eq!(titles, ["Weekly", "Weekly", "Later", "Later", "Later"]);

        // Deleting one of the new series, then all of it.
        let fourth = id_of(start + 3 * week).await;
        agenda.delete(fourth.id, Scope::This).await.unwrap();
        assert_eq!(shown(&agenda, start).await.len(), 4);
        let fifth = id_of(start + 4 * week).await;
        agenda.delete(fifth.id, Scope::All).await.unwrap();
        let titles: Vec<String> = shown(&agenda, start).await.into_iter().map(|(t, _)| t).collect();
        assert_eq!(titles, ["Weekly", "Weekly"]);

        // "Following" from the first one is the whole series.
        let first = id_of(start).await;
        agenda.delete(first.id, Scope::Following).await.unwrap();
        assert!(shown(&agenda, start).await.is_empty());

        // A one-off event that becomes a series, then stops repeating.
        let single = agenda.create(&input(calendar, "Lunch", start, None)).await.unwrap();
        assert!(!single.recurring);
        let series = agenda
            .update(single.id, &input(calendar, "Lunch", start, Some(weekly(3))), Scope::This)
            .await
            .unwrap();
        assert!(series.recurring);
        assert_eq!(shown(&agenda, start).await.len(), 3);
        let one = agenda.update(series.id, &input(calendar, "Lunch", start, None), Scope::All).await.unwrap();
        assert!(!one.recurring);
        assert!(shown(&agenda, start).await.is_empty());

        // Rules are checked.
        let bad = Repeat { interval: 0, ..weekly(3) };
        assert!(agenda.create(&input(calendar, "Bad", start, Some(bad))).await.is_err());
        let mut zoned = input(calendar, "Bad", start, Some(weekly(3)));
        zoned.time_zone = Some("Nowhere/Land".into());
        assert!(agenda.create(&zoned).await.is_err());
    }
}
