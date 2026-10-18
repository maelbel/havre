//! Keeps Tasks in sync, writes changes back to the providers, and rings
//! reminders.
//!
//! - Every 5 minutes (and on demand), each source is asked for its lists,
//!   then each list for its tasks (still to do, and done in the last 30
//!   days).
//! - Creating, changing or deleting a task goes to the provider first; the
//!   local copy follows (same as the Agenda).
//! - Every 30 seconds, tasks whose reminder time has come are announced on
//!   the events channel: the web app and Havre Desktop show a notification.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::sync::{Mutex, Notify, broadcast};

use super::{
    Links, RemoteList, RemoteTask, Task, TaskInput, TaskList, caldav, gtasks, parse,
    store::{Keeps, ListRow, TaskStore},
    todo, validate,
};
use crate::{
    Error, Result,
    agenda::store::CalDavLogin,
    oauth::{OAuthSession, Provider, Resource},
    store::Database,
    sync::{AccountEvent, SyncEvent},
};

const EVERY: Duration = Duration::from_secs(5 * 60);
const REMINDERS_EVERY: Duration = Duration::from_secs(30);
/// Reminders missed by more than this (Havre was off) aren't shown late.
const TOO_LATE: i64 = 6 * 3600;

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or_default()
}

/// Where a list's tasks live.
enum Source {
    Local,
    Microsoft(String),
    Google(String),
    CalDav(CalDavLogin),
}

impl Source {
    /// What Havre keeps that this provider can't.
    fn keeps(&self) -> Keeps {
        match self {
            Source::Local => Keeps { time: false, reminder: false },
            Source::Microsoft(_) => Keeps { time: true, reminder: false },
            Source::Google(_) => Keeps { time: true, reminder: true },
            Source::CalDav(_) => Keeps { time: false, reminder: true },
        }
    }
}

/// One account or CalDAV server, for Settings › Accounts (task sync).
#[derive(Debug, Clone, Serialize)]
pub struct TaskSource {
    /// `account:3` or `caldav:2`.
    pub key: String,
    /// `microsoft`, `google` or `caldav`.
    pub kind: String,
    pub name: String,
    pub enabled: bool,
    /// The sign-in allows tasks (`false`: reconnect the account).
    pub allowed: bool,
    pub lists: i64,
    pub error: Option<String>,
}

pub struct Tasks {
    db: Database,
    store: TaskStore,
    events: broadcast::Sender<AccountEvent>,
    /// Graph / Google tokens, per mail account.
    sessions: Mutex<HashMap<i64, Arc<OAuthSession>>>,
    wake: Notify,
    /// One full sync at a time.
    syncing: Mutex<()>,
}

impl Tasks {
    /// Starts the background sync and the reminders. `events` is the
    /// channel the web UI listens to (shared with Mail and the Agenda).
    pub fn start(db: Database, events: broadcast::Sender<AccountEvent>) -> Arc<Self> {
        let tasks = Arc::new(Self {
            store: db.tasks(),
            db,
            events,
            sessions: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            syncing: Mutex::new(()),
        });

        // An account was added, reconnected or removed: sync soon, with
        // fresh tokens. The loops hold a `Weak` reference, so they stop when
        // Havre drops `Tasks`.
        let mut changes = tasks.events.subscribe();
        let listener = Arc::downgrade(&tasks);
        tokio::spawn(async move {
            loop {
                let change = match changes.recv().await {
                    Ok(change) => change,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let Some(tasks) = listener.upgrade() else { break };
                if change.event == SyncEvent::Accounts {
                    tasks.sessions.lock().await.remove(&change.account);
                    tasks.wake.notify_one();
                }
            }
        });

        let background = Arc::downgrade(&tasks);
        tokio::spawn(async move {
            loop {
                let Some(tasks) = background.upgrade() else { break };
                if let Err(err) = tasks.sync_all().await {
                    tracing::warn!("tasks sync: {err}");
                }
                let wake = async { tasks.wake.notified().await };
                tokio::select! {
                    _ = tokio::time::sleep(EVERY) => {}
                    _ = wake => {}
                }
            }
        });

        let ringer = Arc::downgrade(&tasks);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(REMINDERS_EVERY).await;
                let Some(tasks) = ringer.upgrade() else { break };
                if let Err(err) = tasks.ring(now()).await {
                    tracing::warn!("reminders: {err}");
                }
            }
        });
        tasks
    }

    pub fn store(&self) -> &TaskStore {
        &self.store
    }

    fn announce(&self) {
        let _ = self.events.send(AccountEvent { account: 0, event: SyncEvent::Tasks });
    }

    /// Syncs soon, in the background.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Announces the reminders whose time has come (once each).
    async fn ring(&self, now: i64) -> Result<()> {
        for task in self.store.due_reminders(now).await? {
            self.store.mark_reminded(task.id).await?;
            if task.remind_at.is_some_and(|at| now - at <= TOO_LATE) {
                let event = SyncEvent::Reminder { task: task.id, title: task.title.clone() };
                let _ = self.events.send(AccountEvent { account: 0, event });
            }
        }
        Ok(())
    }

    // ---- Syncing ---------------------------------------------------------------------------

    /// Refreshes every source: its lists, then their tasks.
    pub async fn sync_all(&self) -> Result<()> {
        let _guard = self.syncing.lock().await;
        let now = now();

        for account in self.db.accounts().await? {
            let Some(provider) = Provider::parse(&account.auth) else { continue };
            let source = provider.as_str();
            let key = format!("account:{}", account.id);
            if !account.can_tasks || !self.store.sync_enabled(&key).await? {
                // Off, or signed in before Tasks existed (reconnect to allow).
                self.store.remove_lists(source, Some(account.id), None).await?;
                continue;
            }
            let listed = match self.token(account.id, provider).await {
                Ok(token) => match provider {
                    Provider::Microsoft => todo::lists(&token).await,
                    Provider::Google => gtasks::lists(&token).await,
                },
                Err(err) => Err(err),
            };
            self.apply_listing(source, Some(account.id), None, listed).await?;
        }
        for server in self.db.agenda().caldav_accounts().await? {
            let key = format!("caldav:{}", server.id);
            if !self.store.sync_enabled(&key).await? {
                self.store.remove_lists("caldav", None, Some(server.id)).await?;
                continue;
            }
            let listed = match self.db.agenda().caldav_login(server.id).await {
                Ok(login) => caldav::lists(&login).await,
                Err(err) => Err(err),
            };
            self.apply_listing("caldav", None, Some(server.id), listed).await?;
        }

        for row in self.store.list_rows().await? {
            if row.list.source == "local" {
                continue;
            }
            let result = self.sync_list(&row, now).await;
            if let Err(err) = &result {
                tracing::warn!(list = row.list.name, "{err}");
            }
            self.store.set_list_status(row.list.id, result.err().map(|e| e.to_string()).as_deref()).await?;
        }
        self.announce();
        Ok(())
    }

    /// Saves a source's lists, or its error on its lists.
    async fn apply_listing(
        &self,
        source: &str,
        account: Option<i64>,
        caldav: Option<i64>,
        listed: Result<Vec<RemoteList>>,
    ) -> Result<()> {
        match listed {
            Ok(lists) => self.store.replace_lists(source, account, caldav, &lists).await,
            Err(err) => {
                tracing::warn!(source, "can't list task lists: {err}");
                self.store.set_source_error(source, account, caldav, &err.to_string()).await
            }
        }
    }

    async fn sync_list(&self, row: &ListRow, now: i64) -> Result<()> {
        let source = self.source(&row.list).await?;
        let id = &row.remote_id;
        let tasks: Vec<RemoteTask> = match &source {
            Source::Local => return Ok(()),
            Source::Microsoft(token) => todo::tasks(token, id, now).await?,
            Source::Google(token) => gtasks::tasks(token, id, now).await?,
            Source::CalDav(login) => caldav::tasks(login, id).await?,
        };
        self.store.replace_tasks(row.list.id, &tasks, source.keeps()).await
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

    async fn source(&self, list: &TaskList) -> Result<Source> {
        Ok(match (list.source.as_str(), list.account, list.caldav) {
            ("local", _, _) => Source::Local,
            ("microsoft", Some(account), _) => {
                Source::Microsoft(self.token(account, Provider::Microsoft).await?)
            }
            ("google", Some(account), _) => Source::Google(self.token(account, Provider::Google).await?),
            ("caldav", _, Some(server)) => Source::CalDav(self.db.agenda().caldav_login(server).await?),
            _ => return Err(Error::Config(format!("task list {} has no source", list.id))),
        })
    }

    // ---- Settings ----------------------------------------------------------------------

    /// Every account and CalDAV server that could sync tasks.
    pub async fn sources(&self) -> Result<Vec<TaskSource>> {
        let lists = self.store.lists().await?;
        let summary = |account: Option<i64>, caldav: Option<i64>| {
            let mine: Vec<&TaskList> = lists
                .iter()
                .filter(|l| l.source != "local" && l.account == account && l.caldav == caldav)
                .collect();
            (mine.len() as i64, mine.iter().find_map(|l| l.error.clone()))
        };
        let mut out = Vec::new();
        for account in self.db.accounts().await? {
            let Some(provider) = Provider::parse(&account.auth) else { continue };
            let key = format!("account:{}", account.id);
            let (count, error) = summary(Some(account.id), None);
            out.push(TaskSource {
                enabled: self.store.sync_enabled(&key).await?,
                key,
                kind: provider.as_str().to_owned(),
                name: account.name,
                allowed: account.can_tasks,
                lists: count,
                error,
            });
        }
        for server in self.db.agenda().caldav_accounts().await? {
            let key = format!("caldav:{}", server.id);
            let (count, error) = summary(None, Some(server.id));
            out.push(TaskSource {
                enabled: self.store.sync_enabled(&key).await?,
                key,
                kind: "caldav".into(),
                name: server.name,
                allowed: true,
                lists: count,
                error,
            });
        }
        Ok(out)
    }

    /// Turns task sync on or off for one account or server, and syncs.
    pub async fn set_sync(&self, key: &str, enabled: bool) -> Result<Vec<TaskSource>> {
        if !self.sources().await?.iter().any(|s| s.key == key) {
            return Err(Error::NotFound(format!("task source {key}")));
        }
        self.store.set_sync_enabled(key, enabled).await?;
        self.sync_all().await?;
        self.sources().await
    }

    // ---- Writing ---------------------------------------------------------------------------

    /// A new task (in the default list when `input.list` is `None`).
    pub async fn create(&self, input: &TaskInput) -> Result<Task> {
        validate(input)?;
        let list = match input.list {
            Some(id) => id,
            None => self.store.default_list().await?,
        };
        let row = self.store.list_row(list).await?;
        if row.list.read_only {
            return Err(Error::Config(format!("“{}” is read-only.", row.list.name)));
        }
        let source = self.source(&row.list).await?;
        let now = now();
        let mut remote = match &source {
            Source::Local => local_task(&format!("local-{}", crate::agenda::random_id()), input, now),
            Source::Microsoft(token) => todo::create(token, &row.remote_id, input, now).await?,
            Source::Google(token) => gtasks::create(token, &row.remote_id, input, now).await?,
            Source::CalDav(login) => caldav::create(login, &row.remote_id, input, now).await?,
        };
        keep_what_we_know(&mut remote, input);
        let task = self.store.save_task(list, &remote, input.links.as_ref(), source.keeps()).await?;
        self.announce();
        Ok(task)
    }

    /// Changes a task. Moving it to another list creates it there (with its
    /// links) and deletes the original.
    pub async fn update(&self, id: i64, input: &TaskInput) -> Result<Task> {
        validate(input)?;
        let current = self.store.task_row(id).await?;
        if current.task.read_only {
            return Err(Error::Config("This task is in a read-only list.".into()));
        }
        if let Some(list) = input.list
            && list != current.task.list
        {
            let links = input.links.clone().unwrap_or_else(|| current.task.links.clone());
            let moved = self.create(&TaskInput { links: Some(links), ..input.clone() }).await?;
            self.delete(id).await?;
            return Ok(moved);
        }
        let row = self.store.list_row(current.task.list).await?;
        let source = self.source(&row.list).await?;
        let now = now();
        // Done stays done at the time it was done.
        let input = &TaskInput { list: Some(current.task.list), ..input.clone() };
        let mut remote = match &source {
            Source::Local => {
                let mut task = local_task(&current.remote_id, input, now);
                if input.done && current.task.done_at.is_some() {
                    task.done_at = current.task.done_at;
                }
                task
            }
            Source::Microsoft(token) => {
                todo::update(token, &row.remote_id, &current.remote_id, input, now).await?
            }
            Source::Google(token) => {
                gtasks::update(token, &row.remote_id, &current.remote_id, input, now).await?
            }
            Source::CalDav(login) => {
                let href =
                    current.href.as_deref().ok_or_else(|| Error::Config("missing CalDAV address".into()))?;
                let raw =
                    current.raw.as_deref().ok_or_else(|| Error::Config("missing CalDAV task".into()))?;
                caldav::update(login, href, current.etag.as_deref(), raw, input, now).await?
            }
        };
        keep_what_we_know(&mut remote, input);
        let task = self.store.save_task(row.list.id, &remote, input.links.as_ref(), source.keeps()).await?;
        self.announce();
        Ok(task)
    }

    /// Ticks a task (or unticks it).
    pub async fn set_done(&self, id: i64, done: bool) -> Result<Task> {
        let task = self.store.task(id).await?;
        self.update(
            id,
            &TaskInput {
                list: Some(task.list),
                title: task.title,
                notes: task.notes,
                due: task.due,
                due_at: task.due_at,
                remind_at: task.remind_at,
                done,
                links: None,
            },
        )
        .await
    }

    pub async fn delete(&self, id: i64) -> Result<()> {
        let current = self.store.task_row(id).await?;
        if current.task.read_only {
            return Err(Error::Config("This task is in a read-only list.".into()));
        }
        let row = self.store.list_row(current.task.list).await?;
        match self.source(&row.list).await? {
            Source::Local => {}
            Source::Microsoft(token) => todo::delete(&token, &row.remote_id, &current.remote_id).await?,
            Source::Google(token) => gtasks::delete(&token, &row.remote_id, &current.remote_id).await?,
            Source::CalDav(login) => {
                if let Some(href) = &current.href {
                    caldav::delete(&login, href, current.etag.as_deref()).await?;
                }
            }
        }
        self.store.delete_task(id).await?;
        self.announce();
        Ok(())
    }

    /// Quick add: reads `text` ("Call Thomas fri 10:00 #work") and creates
    /// the task, with links when given (made from an email).
    pub async fn quick_add(&self, text: &str, js_offset: i32, links: Option<Links>) -> Result<Task> {
        let parsed = self.parse(text, js_offset).await?;
        self.create(&TaskInput {
            list: parsed.list,
            title: parsed.title,
            due: parsed.due,
            due_at: parsed.due_at,
            links,
            ..TaskInput::default()
        })
        .await
    }

    /// What quick add would make of `text` (the preview while typing).
    pub async fn parse(&self, text: &str, js_offset: i32) -> Result<parse::Parsed> {
        let lists: Vec<(i64, String)> =
            self.store.lists().await?.into_iter().filter(|l| !l.read_only).map(|l| (l.id, l.name)).collect();
        Ok(parse::parse(text, now(), js_offset, &lists))
    }
}

/// A Havre-list task from the editor.
fn local_task(remote_id: &str, input: &TaskInput, now: i64) -> RemoteTask {
    RemoteTask {
        remote_id: remote_id.to_owned(),
        title: input.title.trim().to_owned(),
        notes: input.notes.trim().to_owned(),
        due: input.due,
        due_at: input.due_at,
        remind_at: input.remind_at,
        done_at: if input.done { Some(now) } else { None },
        updated_at: Some(now),
        ..RemoteTask::default()
    }
}

/// What the provider doesn't send back but we just set: the time (To Do,
/// Google) and the reminder (Google, CalDAV).
fn keep_what_we_know(remote: &mut RemoteTask, input: &TaskInput) {
    if remote.due == input.due && remote.due_at.is_none() {
        remote.due_at = input.due_at;
    }
    if remote.remind_at.is_none() {
        remote.remind_at = input.remind_at;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::{DAY, MailLink, TaskQuery};

    async fn tasks() -> (Arc<Tasks>, Database) {
        let db = Database::in_memory().await.unwrap();
        let (events, _) = broadcast::channel(16);
        (Tasks::start(db.clone(), events), db)
    }

    #[tokio::test]
    async fn havre_lists_work_with_no_account() {
        let (tasks, _) = tasks().await;
        let links = Links {
            mail: Some(MailLink {
                account: 1,
                message_id: Some("q4@renard".into()),
                subject: "Devis".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let due = 20_000 * DAY;
        let task = tasks
            .create(&TaskInput {
                title: " Confirm the quote ".into(),
                due: Some(due),
                due_at: Some(due + 17 * 3600),
                remind_at: Some(due + 16 * 3600),
                links: Some(links.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(task.title, "Confirm the quote");
        assert_eq!(task.links, links);

        // Ticked, then the reminder doesn't ring.
        let done = tasks.set_done(task.id, true).await.unwrap();
        assert!(done.done_at.is_some());
        assert_eq!(done.links, links, "ticking keeps the links");
        assert!(tasks.store().due_reminders(due + 16 * 3600).await.unwrap().is_empty());
        let open = tasks.set_done(task.id, false).await.unwrap();
        assert_eq!(open.done_at, None);

        // Moved to another Havre list: links follow.
        let home = tasks.store().create_local_list("Home").await.unwrap();
        let moved = tasks
            .update(
                task.id,
                &TaskInput { list: Some(home.id), title: "Confirm".into(), ..Default::default() },
            )
            .await
            .unwrap();
        assert_eq!((moved.list, moved.links.clone()), (home.id, links));
        assert!(tasks.store().task(task.id).await.is_err());

        // Found by its email.
        let q = TaskQuery { message_id: Some("q4@renard".into()), ..Default::default() };
        assert_eq!(tasks.store().tasks(&q).await.unwrap().len(), 1);
        tasks.delete(moved.id).await.unwrap();
        assert!(tasks.store().tasks(&TaskQuery::default()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn reminders_ring_once() {
        let (tasks, _) = tasks().await;
        let mut events = tasks.events.subscribe();
        let at = now() - 60;
        let task = tasks
            .create(&TaskInput { title: "Call".into(), remind_at: Some(at), ..Default::default() })
            .await
            .unwrap();
        // Skip the "tasks changed" announcement of the create.
        tasks.ring(now()).await.unwrap();
        tasks.ring(now()).await.unwrap();
        let mut rings = 0;
        while let Ok(e) = events.try_recv() {
            if let SyncEvent::Reminder { task: id, .. } = e.event {
                assert_eq!(id, task.id);
                rings += 1;
            }
        }
        assert_eq!(rings, 1);
    }

    #[tokio::test]
    async fn quick_add_picks_the_list() {
        let (tasks, _) = tasks().await;
        let home = tasks.store().create_local_list("Home").await.unwrap();
        let task = tasks.quick_add("Boiler service #home tomorrow", -120, None).await.unwrap();
        assert_eq!((task.title.as_str(), task.list), ("Boiler service", home.id));
        assert!(task.due.is_some());
    }
}
