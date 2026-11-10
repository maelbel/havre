//! Step 29a's security test: two users on one Havre.
//!
//! The admin's data is full of the word `SECRET` (a subject, a note, a task,
//! a meeting, a person, a label, a draft, a device…). Then **every API route**
//! is asked by the other user, with the admin's ids where a route takes one.
//! Whatever the answer (an empty list, "not found", "bad request"), it must
//! never contain `SECRET`, never be a server error, and afterwards the
//! admin's data must be as it was: nothing read, nothing changed.
//!
//! A route this test doesn't know makes it fail (`every_route_is_tested`):
//! a new route comes with its line here, so it's checked too.
//!
//! What it teaches: testing a security property across a whole API, and
//! driving an Axum router without a network (`oneshot`, from `tower`).

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::connect_info::MockConnectInfo,
    http::{Request, StatusCode},
};
use futures::StreamExt;
use havre_core::{
    accounts::AccountManager,
    agenda::Agenda,
    ai::Ai,
    mail::{Folder, FolderRole, MessageSummary, Person},
    people::sync::ContactSync,
    push::{NewSubscription, Push, SubscriptionKeys},
    store::Database,
    sync::{AccountEvent, SyncEvent},
    tasks::Tasks,
    users::UserId,
};
use serde_json::{Value, json};
use tokio::sync::watch;
use tower::ServiceExt;

use crate::{AppState, Shutdown, api, auth, mail_source::MailSource, mcp};

const ADMIN: UserId = UserId::ADMIN;
/// In everything the admin has; never in anything the other user gets.
const MARK: &str = "SECRET";

/// A valid browser key (RFC 8291's example), to register a push device.
const UA_PUBLIC: &str =
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
const UA_AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";

/// The admin's things, by id.
struct Ids {
    account: i64,
    calendar: i64,
    event: i64,
    note: i64,
    task: i64,
    list: i64,
    label: i64,
    draft: i64,
    device: i64,
}

struct Fixture {
    app: Router,
    state: AppState,
    db: Database,
    member: UserId,
    admin_cookie: String,
    member_cookie: String,
    ids: Ids,
    /// Kept alive: dropping it would tell long requests the server stops.
    _stop: watch::Sender<bool>,
}

fn from<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("test input")
}

/// A Havre with two users: the admin (user 1), full of `SECRET`s, and Léa.
async fn havre() -> Fixture {
    let db = Database::in_memory().await.unwrap();
    let login = db.auth();
    // The admin's browser says SECRET too: Léa must not see it in her sessions.
    let admin_token = login.set_up("correct horse battery", "SECRET browser").await.unwrap();
    let member = db.users().add("Léa").await.unwrap();
    let member_token = login.create_session(member, "Léa's phone").await.unwrap();

    let manager = AccountManager::start(db.clone()).await.unwrap();
    let agenda = Agenda::start(db.clone(), manager.sender());
    let tasks = Tasks::start(db.clone(), manager.sender());
    let contacts = ContactSync::start(db.clone(), manager.sender());
    let ai = Ai::start(Arc::clone(&manager), Arc::clone(&agenda));
    let push = Push::start(db.clone(), &manager.sender());

    // ---- The admin's things --------------------------------------------------
    let account = db
        .insert_account(
            ADMIN,
            &from(json!({
                "name": "SECRET work", "email": "admin-SECRET@corp.test", "imap_host": "127.0.0.1",
                "imap_port": 1, "password": "x", "color": "work"
            })),
        )
        .await
        .unwrap()
        .id;
    let mailbox = db.mailbox(ADMIN, account).await.unwrap();
    let inbox = Folder {
        path: "INBOX".into(),
        name: "INBOX".into(),
        depth: 0,
        role: FolderRole::Inbox,
        declared_role: true,
    };
    mailbox.replace_folders(&[inbox]).await.unwrap();
    let message = MessageSummary {
        uid: 1,
        from: Some(Person { name: Some("SECRET sender".into()), address: Some("boss@corp.test".into()) }),
        subject: "SECRET subject".into(),
        date: Some("2026-11-01T09:00:00+00:00".into()),
        unread: true,
        flagged: true,
        message_id: Some("m1@corp.test".into()),
        recipients: Some(vec!["admin-secret@corp.test".into()]),
        ..MessageSummary::default()
    };
    mailbox.upsert_messages("INBOX", &[message]).await.unwrap();
    let label = db.create_label(ADMIN, "SECRET label", "work").await.unwrap().id;
    mailbox.set_label("INBOX", &[1], label, true).await.unwrap();

    let note = db
        .notes(ADMIN)
        .create(&from(json!({ "title": "SECRET note", "body": "SECRET body" })))
        .await
        .unwrap()
        .note
        .id;
    let task = tasks.create(ADMIN, &from(json!({ "title": "SECRET task" }))).await.unwrap();
    let mine = agenda.for_user(ADMIN);
    let calendar = mine.store().calendars().await.unwrap()[0].id;
    let start = 1_793_000_000; // November 2026
    let event = mine
        .create(&from(json!({
            "calendar": calendar, "title": "SECRET meeting", "start": start, "end": start + 3600,
            "attendees": ["friend@x.test"]
        })))
        .await
        .unwrap()
        .id;
    mine.store().set_time_zone("Europe/Paris").await.unwrap();
    db.people(ADMIN).create("SECRET person", Some("friend@x.test")).await.unwrap();
    let draft = manager
        .new_draft(ADMIN, from(json!({ "account": account, "subject": "SECRET draft" })))
        .await
        .unwrap()
        .id;
    push.subscribe(
        ADMIN,
        &NewSubscription {
            endpoint: "https://push.example/SECRET".into(),
            keys: SubscriptionKeys { p256dh: UA_PUBLIC.into(), auth: UA_AUTH.into() },
            label: "SECRET phone".into(),
        },
    )
    .await
    .unwrap();
    let device = push.devices(ADMIN, None).await.unwrap()[0].id;
    let list = tasks.store(ADMIN).lists().await.unwrap()[0].id;
    for app in ["ai", "notes", "tasks", "files"] {
        db.apps(ADMIN).set_enabled(app, true).await.unwrap();
    }

    // ---- Léa's own --------------------------------------------------------------
    db.notes(member).create(&from(json!({ "title": "Léa's note" }))).await.unwrap();

    let (stop, stopping) = watch::channel(false);
    let state = AppState {
        mail: MailSource::Accounts(manager),
        auth: Some(login),
        agenda: Some(agenda),
        tasks: Some(tasks),
        contacts: Some(contacts),
        ai: Some(ai),
        backups: None,
        push: Some(push),
        throttle: auth::Throttle::default(),
        shutdown: Shutdown(stopping),
    };
    let app = Router::new()
        .nest("/api", api::router(state.clone()))
        .merge(mcp::endpoint())
        .with_state(state.clone())
        // The sign-in route reads the caller's address (to slow down guessing).
        .layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 9))));
    Fixture {
        app,
        state,
        db,
        member,
        admin_cookie: format!("havre_session={admin_token}"),
        member_cookie: format!("havre_session={member_token}"),
        ids: Ids { account, calendar, event, note, task: task.id, list, label, draft, device },
        _stop: stop,
    }
}

/// Calls the API: `(status, body)`.
async fn call(app: &Router, cookie: &str, method: &str, uri: &str, body: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 10 * 1024 * 1024).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Every route, as `(method, path pattern, the request to send)`. The
/// requests use the admin's ids: what another user might try.
fn requests(ids: &Ids) -> Vec<(&'static str, &'static str, String, String)> {
    let Ids { account: a, calendar: c, event: e, note: n, task: t, list: l, label: lb, draft: d, device: dv } =
        *ids;
    let sel = json!({ "account": a, "folder": "INBOX", "uids": [1] }).to_string();
    let r = |m: &'static str, p: &'static str, uri: String, body: &str| (m, p, uri, body.to_owned());
    vec![
        // Public: the same for everyone.
        r("GET", "/health", "/api/health".into(), ""),
        r("GET", "/auth/status", "/api/auth/status".into(), ""),
        r("POST", "/auth/setup", "/api/auth/setup".into(), r#"{"password":"another password"}"#),
        r("POST", "/auth/login", "/api/auth/login".into(), r#"{"password":"wrong password!"}"#),
        // Léa's own session (not the admin's: their cookie is still valid at the end).
        r("POST", "/auth/password", "/api/auth/password".into(), r#"{"current":"x","new":"new password!"}"#),
        r("GET", "/auth/sessions", "/api/auth/sessions".into(), ""),
        r("DELETE", "/auth/sessions/others", "/api/auth/sessions/others".into(), ""),
        // Accounts and mail.
        r("GET", "/accounts", "/api/accounts".into(), ""),
        r("POST", "/accounts", "/api/accounts".into(), "{}"),
        r("PATCH", "/accounts/{id}", format!("/api/accounts/{a}"), r#"{"name":"mine now"}"#),
        r("DELETE", "/accounts/{id}", format!("/api/accounts/{a}"), ""),
        r("GET", "/accounts/detect", "/api/accounts/detect?email=nobody".into(), ""),
        r("POST", "/accounts/oauth", "/api/accounts/oauth".into(), &json!({ "reconnect": a }).to_string()),
        r("GET", "/accounts/oauth/{flow}", "/api/accounts/oauth/flow".into(), ""),
        r("GET", "/oauth/{provider}/callback", "/api/oauth/google/callback?state=x".into(), ""),
        r("GET", "/mail/folders", format!("/api/mail/folders?account={a}"), ""),
        r("GET", "/mail/messages", format!("/api/mail/messages?account={a}&folder=INBOX"), ""),
        r("GET", "/mail/messages/{uid}", format!("/api/mail/messages/1?account={a}&folder=INBOX"), ""),
        r("PUT", "/mail/messages/{uid}/seen", format!("/api/mail/messages/1/seen?account={a}"), r#"{"seen":true}"#),
        r("GET", "/mail/messages/{uid}/attachments/{index}", format!("/api/mail/messages/1/attachments/0?account={a}"), ""),
        r("POST", "/mail/actions", "/api/mail/actions".into(), &json!({ "account": a, "action": "trash", "folder": "INBOX", "uids": [1] }).to_string()),
        r("POST", "/mail/sync", format!("/api/mail/sync?account={a}"), ""),
        r("GET", "/mail/status", format!("/api/mail/status?account={a}"), ""),
        r("GET", "/contacts", "/api/contacts?q=fri".into(), ""),
        r("GET", "/mail/organize", "/api/mail/organize".into(), ""),
        r("GET", "/mail/view", "/api/mail/view?view=starred".into(), ""),
        r("GET", "/mail/view", format!("/api/mail/view?view=label&label={lb}"), ""),
        r("GET", "/mail/view", "/api/mail/view?view=inboxes".into(), ""),
        r("POST", "/mail/flags", "/api/mail/flags".into(), &json!({ "account": a, "folder": "INBOX", "uids": [1], "flagged": false }).to_string()),
        r("POST", "/mail/labels", "/api/mail/labels".into(), &json!({ "account": a, "folder": "INBOX", "uids": [1], "label": lb, "on": false }).to_string()),
        r("POST", "/mail/snooze", "/api/mail/snooze".into(), &json!({ "account": a, "folder": "INBOX", "uids": [1], "until": 4_000_000_000_i64 }).to_string()),
        r("POST", "/mail/undo", "/api/mail/undo".into(), &json!({ "account": a, "from": "INBOX", "to": "Trash", "message_ids": ["m1@corp.test"] }).to_string()),
        r("POST", "/mail/present", "/api/mail/present".into(), &sel),
        // The admin's label name (another case): free for Léa, no "already exists".
        r("POST", "/labels", "/api/labels".into(), r#"{"name":"secret label"}"#),
        r("PATCH", "/labels/{id}", format!("/api/labels/{lb}"), r#"{"name":"taken"}"#),
        r("DELETE", "/labels/{id}", format!("/api/labels/{lb}"), ""),
        r("GET", "/mail/unread", "/api/mail/unread".into(), ""),
        r("GET", "/mail/search", "/api/mail/search?q=SECRET".into(), ""),
        r("GET", "/mail/all", "/api/mail/all".into(), ""),
        r("GET", "/mail/all", "/api/mail/all?q=SECRET".into(), ""),
        // Writing.
        r("GET", "/drafts", "/api/drafts".into(), ""),
        r("POST", "/drafts", "/api/drafts".into(), &json!({ "account": a }).to_string()),
        r("POST", "/drafts/from-message", "/api/drafts/from-message".into(), &json!({ "account": a, "folder": "INBOX", "uid": 1 }).to_string()),
        r("GET", "/drafts/{id}", format!("/api/drafts/{d}"), ""),
        r("PATCH", "/drafts/{id}", format!("/api/drafts/{d}"), r#"{"subject":"changed"}"#),
        r("DELETE", "/drafts/{id}", format!("/api/drafts/{d}"), ""),
        r("POST", "/drafts/{id}/close", format!("/api/drafts/{d}/close"), ""),
        r("POST", "/drafts/{id}/attachments", format!("/api/drafts/{d}/attachments?filename=x.txt"), "hello"),
        r("DELETE", "/drafts/{id}/attachments/{attachment}", format!("/api/drafts/{d}/attachments/1"), ""),
        r("POST", "/drafts/{id}/send", format!("/api/drafts/{d}/send"), "{}"),
        r("POST", "/drafts/{id}/unsend", format!("/api/drafts/{d}/unsend"), ""),
        r("GET", "/mail/scheduled", "/api/mail/scheduled".into(), ""),
        r("GET", "/mail/settings", "/api/mail/settings".into(), ""),
        r("PUT", "/mail/settings", "/api/mail/settings".into(), r#"{"undo_seconds":5,"compose_format":"plain"}"#),
        // Agenda.
        r("GET", "/agenda/calendars", "/api/agenda/calendars".into(), ""),
        r("PATCH", "/agenda/calendars/{id}", format!("/api/agenda/calendars/{c}"), r##"{"enabled":false,"color":"#000000"}"##),
        r("GET", "/agenda/calendars/{id}/export.ics", format!("/api/agenda/calendars/{c}/export.ics"), ""),
        r("GET", "/agenda/events", "/api/agenda/events?from=1792000000&to=1794000000".into(), ""),
        r("POST", "/agenda/events", "/api/agenda/events".into(), &json!({ "calendar": c, "title": "intruder", "start": 1_793_000_000, "end": 1_793_003_600 }).to_string()),
        r("PATCH", "/agenda/events/{id}", format!("/api/agenda/events/{e}"), &json!({ "calendar": c, "title": "changed", "start": 1_793_000_000, "end": 1_793_003_600 }).to_string()),
        r("DELETE", "/agenda/events/{id}", format!("/api/agenda/events/{e}"), ""),
        r("POST", "/agenda/events/{id}/respond", format!("/api/agenda/events/{e}/respond"), r#"{"response":"declined"}"#),
        r("GET", "/agenda/events/{id}/related", format!("/api/agenda/events/{e}/related"), ""),
        r("POST", "/agenda/sync", "/api/agenda/sync".into(), ""),
        r("GET", "/agenda/caldav", "/api/agenda/caldav".into(), ""),
        r("POST", "/agenda/caldav", "/api/agenda/caldav".into(), "{}"),
        r("DELETE", "/agenda/caldav/{id}", "/api/agenda/caldav/1".into(), ""),
        r("POST", "/agenda/invitations", "/api/agenda/invitations".into(), &json!({ "account": a, "folder": "INBOX", "uid": 1, "index": 0 }).to_string()),
        r("GET", "/agenda/invitation", format!("/api/agenda/invitation?account={a}&folder=INBOX&uid=1&index=0"), ""),
        r("POST", "/agenda/invitation/respond", "/api/agenda/invitation/respond".into(), &json!({ "account": a, "folder": "INBOX", "uid": 1, "index": 0, "response": "accepted" }).to_string()),
        r("GET", "/agenda/invitations/pending", "/api/agenda/invitations/pending".into(), ""),
        r("GET", "/agenda/found-in-mail", "/api/agenda/found-in-mail".into(), ""),
        r("GET", "/agenda/settings", "/api/agenda/settings".into(), ""),
        r("PATCH", "/agenda/settings", "/api/agenda/settings".into(), r#"{"default_reminder":null}"#),
        r("PUT", "/agenda/time-zone", "/api/agenda/time-zone".into(), r#"{"zone":"Asia/Tokyo"}"#),
        // Tasks.
        r("GET", "/tasks", "/api/tasks".into(), ""),
        r("POST", "/tasks", "/api/tasks".into(), &json!({ "title": "intruder", "list": l }).to_string()),
        r("POST", "/tasks/quick", "/api/tasks/quick".into(), r#"{"text":"intruder"}"#),
        r("POST", "/tasks/parse", "/api/tasks/parse".into(), r#"{"text":"call the boss #Inbox"}"#),
        r("POST", "/tasks/from-mail", "/api/tasks/from-mail".into(), &json!({ "account": a, "folder": "INBOX", "uid": 1, "text": "x" }).to_string()),
        r("GET", "/tasks/lists", "/api/tasks/lists".into(), ""),
        r("POST", "/tasks/lists", "/api/tasks/lists".into(), r#"{"name":"Léa's"}"#),
        r("PATCH", "/tasks/lists/{id}", format!("/api/tasks/lists/{l}"), r#"{"name":"changed"}"#),
        r("DELETE", "/tasks/lists/{id}", format!("/api/tasks/lists/{l}"), ""),
        r("POST", "/tasks/sync", "/api/tasks/sync".into(), ""),
        r("GET", "/tasks/sources", "/api/tasks/sources".into(), ""),
        r("PUT", "/tasks/sources/{key}", format!("/api/tasks/sources/account:{a}"), r#"{"enabled":true}"#),
        r("GET", "/tasks/{id}", format!("/api/tasks/{t}"), ""),
        r("PUT", "/tasks/{id}", format!("/api/tasks/{t}"), r#"{"title":"changed"}"#),
        r("DELETE", "/tasks/{id}", format!("/api/tasks/{t}"), ""),
        r("PUT", "/tasks/{id}/done", format!("/api/tasks/{t}/done"), r#"{"done":true}"#),
        r("GET", "/tasks/{id}/mail", format!("/api/tasks/{t}/mail"), ""),
        // Notes.
        r("GET", "/notes", "/api/notes".into(), ""),
        r("GET", "/notes", "/api/notes?q=SECRET".into(), ""),
        r("POST", "/notes", "/api/notes".into(), r#"{"title":"Léa again"}"#),
        r("POST", "/notes/from-event", "/api/notes/from-event".into(), &json!({ "event": e }).to_string()),
        r("POST", "/notes/from-mail", "/api/notes/from-mail".into(), &json!({ "account": a, "folder": "INBOX", "uid": 1 }).to_string()),
        r("GET", "/notes/people", "/api/notes/people".into(), ""),
        r("GET", "/notes/{id}", format!("/api/notes/{n}"), ""),
        r("PUT", "/notes/{id}", format!("/api/notes/{n}"), r#"{"title":"changed","body":""}"#),
        r("DELETE", "/notes/{id}", format!("/api/notes/{n}"), ""),
        r("PUT", "/notes/{id}/pin", format!("/api/notes/{n}/pin"), r#"{"pinned":true}"#),
        r("PUT", "/notes/{id}/todos/{line}", format!("/api/notes/{n}/todos/0"), r#"{"done":true}"#),
        r("POST", "/notes/{id}/todos/{line}/task", format!("/api/notes/{n}/todos/0/task"), "{}"),
        r("GET", "/notes/{id}/mail", format!("/api/notes/{n}/mail"), ""),
        // Files.
        r("GET", "/files", "/api/files".into(), ""),
        r("GET", "/files/in-message", format!("/api/files/in-message?account={a}&folder=INBOX&uid=1"), ""),
        r("GET", "/files/progress", "/api/files/progress".into(), ""),
        r("PUT", "/files/settings", "/api/files/settings".into(), r#"{"depth":10}"#),
        // People and contacts.
        r("GET", "/people", "/api/people".into(), ""),
        r("GET", "/people", "/api/people?filter=all".into(), ""),
        r("POST", "/people", "/api/people".into(), r#"{"name":"Léa's friend"}"#),
        r("GET", "/people/companies", "/api/people/companies".into(), ""),
        r("GET", "/people/today", "/api/people/today?date=2026-11-10".into(), ""),
        r("GET", "/people/{email}", "/api/people/friend@x.test".into(), ""),
        r("GET", "/people/{email}", "/api/people/boss@corp.test".into(), ""),
        r("GET", "/people/{email}", "/api/people/~1".into(), ""),
        r("PATCH", "/people/{email}", "/api/people/~1".into(), r#"{"name":"changed"}"#),
        r("POST", "/people/{email}/merge", "/api/people/~1/merge".into(), r#"{"email":"lea@x.test"}"#),
        r("DELETE", "/people/{email}/addresses/{other}", "/api/people/~1/addresses/friend@x.test".into(), ""),
        r("GET", "/people/{email}/photo", "/api/people/friend@x.test/photo".into(), ""),
        r("POST", "/people/import", "/api/people/import".into(), "BEGIN:VCARD\nVERSION:3.0\nFN:Léa's aunt\nEND:VCARD\n"),
        r("GET", "/people/export.vcf", "/api/people/export.vcf".into(), ""),
        r("GET", "/contacts/settings", "/api/contacts/settings".into(), ""),
        r("PUT", "/contacts/sources/{key}", format!("/api/contacts/sources/account:{a}"), r#"{"enabled":true}"#),
        r("PUT", "/contacts/default", "/api/contacts/default".into(), r#"{"book":1}"#),
        r("POST", "/contacts/sync", "/api/contacts/sync".into(), ""),
        // The apps, the dock, the status card.
        r("GET", "/apps", "/api/apps".into(), ""),
        r("PATCH", "/apps/{id}", "/api/apps/ai".into(), r#"{"enabled":true}"#),
        r("PUT", "/apps/order", "/api/apps/order".into(), r#"{"ids":["mail"]}"#),
        r("GET", "/system", "/api/system".into(), ""),
        // Notifications.
        r("GET", "/push/key", "/api/push/key".into(), ""),
        r("POST", "/push/subscribe", "/api/push/subscribe".into(), "{}"),
        r("DELETE", "/push/subscribe", "/api/push/subscribe".into(), r#"{"endpoint":"https://push.example/SECRET"}"#),
        r("GET", "/push/devices", "/api/push/devices".into(), ""),
        r("DELETE", "/push/devices/{id}", format!("/api/push/devices/{dv}"), ""),
        r("GET", "/push/settings", "/api/push/settings".into(), ""),
        r("PATCH", "/push/settings", "/api/push/settings".into(), r#"{"new_mail":false}"#),
        r("POST", "/push/test", "/api/push/test".into(), r#"{"endpoint":"https://push.example/SECRET"}"#),
        r("PUT", "/push/seen", "/api/push/seen".into(), r#"{"endpoint":"https://push.example/SECRET"}"#),
        r("POST", "/push/action", "/api/push/action".into(), &json!({ "action": "read", "target": { "kind": "mail", "account": a, "folder": "INBOX", "uid": 1 } }).to_string()),
        r("POST", "/push/action", "/api/push/action".into(), &json!({ "action": "done", "target": { "kind": "task", "task": t } }).to_string()),
        r("POST", "/push/action", "/api/push/action".into(), &json!({ "action": "snooze-10", "target": { "kind": "event", "event": e, "start": 1_793_000_000 } }).to_string()),
        // The admin's only: AI, MCP tokens, backups (404 for anyone else).
        r("GET", "/ai/status", "/api/ai/status".into(), ""),
        r("GET", "/ai/settings", "/api/ai/settings".into(), ""),
        r("PATCH", "/ai/settings", "/api/ai/settings".into(), "{}"),
        r("GET", "/ai/models", "/api/ai/models".into(), ""),
        r("POST", "/ai/test", "/api/ai/test".into(), ""),
        r("POST", "/ai/summarize", "/api/ai/summarize".into(), "{}"),
        r("POST", "/ai/draft", "/api/ai/draft".into(), "{}"),
        r("POST", "/ai/brief", "/api/ai/brief".into(), "{}"),
        r("GET", "/ai/search", "/api/ai/search?q=boss".into(), ""),
        r("POST", "/ai/ask", "/api/ai/ask".into(), "{}"),
        r("GET", "/ai/conversations", "/api/ai/conversations".into(), ""),
        r("POST", "/ai/conversations/ask", "/api/ai/conversations/ask".into(), "{}"),
        r("GET", "/ai/conversations/{id}", "/api/ai/conversations/1".into(), ""),
        r("PATCH", "/ai/conversations/{id}", "/api/ai/conversations/1".into(), "{}"),
        r("DELETE", "/ai/conversations/{id}", "/api/ai/conversations/1".into(), ""),
        r("PUT", "/ai/conversations/{id}/turns/{turn}/done", "/api/ai/conversations/1/turns/0/done".into(), "{}"),
        r("GET", "/ai/claude-code", "/api/ai/claude-code".into(), ""),
        r("POST", "/ai/claude-code/login", "/api/ai/claude-code/login".into(), ""),
        r("DELETE", "/ai/claude-code/login", "/api/ai/claude-code/login".into(), ""),
        r("POST", "/ai/claude-code/login/complete", "/api/ai/claude-code/login/complete".into(), "{}"),
        r("POST", "/ai/claude-code/logout", "/api/ai/claude-code/logout".into(), ""),
        r("GET", "/ai/activity", "/api/ai/activity".into(), ""),
        r("POST", "/ai/activity", "/api/ai/activity".into(), "{}"),
        r("GET", "/ai/health", "/api/ai/health".into(), ""),
        r("GET", "/mcp/tokens", "/api/mcp/tokens".into(), ""),
        r("POST", "/mcp/tokens", "/api/mcp/tokens".into(), r#"{"name":"mine"}"#),
        r("DELETE", "/mcp/tokens/{id}", "/api/mcp/tokens/1".into(), ""),
        r("GET", "/backups", "/api/backups".into(), ""),
        r("POST", "/backups", "/api/backups".into(), ""),
        r("PATCH", "/backups/settings", "/api/backups/settings".into(), "{}"),
        r("PUT", "/backups/remote", "/api/backups/remote".into(), "{}"),
        r("DELETE", "/backups/remote", "/api/backups/remote".into(), ""),
        r("POST", "/backups/remote/test", "/api/backups/remote/test".into(), "{}"),
        r("GET", "/backups/{name}", "/api/backups/havre-2026-11-10.db".into(), ""),
        r("DELETE", "/backups/{name}", "/api/backups/havre-2026-11-10.db".into(), ""),
        r("POST", "/backups/{name}/offsite", "/api/backups/havre-2026-11-10.db/offsite".into(), ""),
        // `/mcp` takes its own tokens, never a browser session.
        r("POST", "/mcp", "/mcp".into(), r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#),
        r("GET", "/mcp", "/mcp".into(), ""),
        r("DELETE", "/mcp", "/mcp".into(), ""),
        // Last: signs Léa out (her own session only).
        r("POST", "/auth/logout", "/api/auth/logout".into(), ""),
    ]
}

/// Routes that are the admin's only: anyone else gets 404.
fn admin_only(pattern: &str) -> bool {
    (pattern.starts_with("/ai/") && pattern != "/ai/status")
        || pattern.starts_with("/backups")
        || pattern.starts_with("/mcp/tokens")
}

#[tokio::test]
async fn another_user_reaches_nothing_of_the_admins() {
    let f = havre().await;

    // The admin does see their things (or the test would prove nothing).
    for uri in
        ["/api/notes", "/api/tasks", "/api/mail/all", "/api/drafts", "/api/people", "/api/push/devices"]
    {
        let (status, body) = call(&f.app, &f.admin_cookie, "GET", uri, "").await;
        assert!(
            status.is_success() && body.contains(MARK),
            "the admin sees theirs at {uri}: {status} {body}"
        );
    }

    for (method, pattern, uri, body) in requests(&f.ids) {
        let (status, answer) = call(&f.app, &f.member_cookie, method, &uri, &body).await;
        let what = format!("{method} {uri} → {status} {answer}");
        assert!(!status.is_server_error(), "a server error: {what}");
        assert!(!answer.contains(MARK), "the admin's data leaked: {what}");
        if admin_only(pattern) {
            assert_eq!(status, StatusCode::NOT_FOUND, "the admin's only: {what}");
        }
        if (method, pattern) == ("POST", "/labels") {
            assert_eq!(status, StatusCode::CREATED, "label names are each user's: {what}");
        }
    }

    // Léa's own things work: she's a user, not a stranger.
    let (status, body) = call(&f.app, &f.member_cookie, "GET", "/api/notes", "").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "signed out by the last request: {body}");

    // And nothing of the admin's changed.
    let db = &f.db;
    let note = db.notes(ADMIN).view(f.ids.note).await.unwrap().note;
    assert_eq!((note.title.as_str(), note.pinned), ("SECRET note", false));
    let tasks = f.state.tasks.as_ref().unwrap();
    let task = tasks.store(ADMIN).task(f.ids.task).await.unwrap();
    assert_eq!((task.title.as_str(), task.done_at), ("SECRET task", None));
    assert_eq!(tasks.store(ADMIN).lists().await.unwrap()[0].name, "Inbox");
    let agenda = f.state.agenda.as_ref().unwrap().for_user(ADMIN);
    assert_eq!(agenda.store().event(f.ids.event).await.unwrap().title, "SECRET meeting");
    let calendar = agenda.store().calendars().await.unwrap().into_iter().find(|c| c.id == f.ids.calendar);
    assert!(calendar.is_some_and(|c| c.enabled && c.color != "#000000"));
    assert_eq!(agenda.store().settings().await.unwrap().time_zone.as_deref(), Some("Europe/Paris"));
    assert_eq!(db.labels(ADMIN).await.unwrap()[0].name, "SECRET label");
    let draft = db.drafts(ADMIN).get(f.ids.draft).await.unwrap();
    assert_eq!((draft.subject.as_str(), draft.send_at), ("SECRET draft", None));
    assert_eq!(db.drafts(ADMIN).mail_settings().await.unwrap().compose_format, "rich");
    assert_eq!(db.accounts(ADMIN).await.unwrap()[0].name, "SECRET work");
    let push = f.state.push.as_ref().unwrap();
    assert_eq!(push.devices(ADMIN, None).await.unwrap().len(), 1);
    assert!(push.settings(ADMIN).await.unwrap().new_mail);
    let mailbox = db.mailbox(ADMIN, f.ids.account).await.unwrap();
    let query = havre_core::mail::ListQuery { limit: 10, before_uid: None, search: None };
    let page = mailbox.page("INBOX", &query).await.unwrap();
    let message = &page.messages[0];
    assert!(message.unread && message.flagged && message.snoozed_until.is_none());
    assert_eq!(message.labels, [f.ids.label]);
    assert_eq!(db.people(ADMIN).person("friend@x.test").await.unwrap().person.name, "SECRET person");
    assert!(
        f.state
            .auth
            .as_ref()
            .unwrap()
            .check_session(f.admin_cookie.split('=').nth(1).unwrap())
            .await
            .unwrap()
            .is_some()
    );
    // Léa's attempts made her own things, never the admin's.
    assert!(
        db.notes(f.member).notes(&Default::default()).await.unwrap().iter().all(|n| !n.title.contains(MARK))
    );
}

/// Live changes (`/api/events`) carry each user's own only: a reminder of
/// the admin's never reaches Léa's page.
#[tokio::test]
async fn live_changes_go_to_their_user_only() {
    let f = havre().await;
    let request =
        Request::builder().uri("/api/events").header("cookie", &f.member_cookie).body(Body::empty()).unwrap();
    let response = f.app.clone().oneshot(request).await.unwrap();
    let mut stream = response.into_body().into_data_stream();
    let MailSource::Accounts(manager) = &f.state.mail else { unreachable!() };
    let send = |user: UserId, title: &str| {
        let event = SyncEvent::Reminder { task: 1, title: title.into() };
        manager.sender().send(AccountEvent { account: 0, user, event }).unwrap();
    };
    send(ADMIN, "SECRET reminder");
    send(f.member, "Léa's reminder");
    let mut seen = String::new();
    while !seen.contains("Léa's reminder") {
        let next = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next()).await;
        let chunk = next.expect("Léa's event arrives").expect("the stream goes on").unwrap();
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(!seen.contains(MARK), "{seen}");
}

/// Every route of the API is in [`requests`]: a new route must come with
/// its line there, so it's checked too.
#[test]
fn every_route_is_tested() {
    let sources = [
        include_str!("api.rs"),
        include_str!("agenda.rs"),
        include_str!("ai.rs"),
        include_str!("auth.rs"),
        include_str!("backups.rs"),
        include_str!("contacts.rs"),
        include_str!("files.rs"),
        include_str!("mcp.rs"),
        include_str!("notes.rs"),
        include_str!("organize.rs"),
        include_str!("platform.rs"),
        include_str!("push.rs"),
        include_str!("tasks.rs"),
    ];
    let ids =
        Ids { account: 1, calendar: 1, event: 1, note: 1, task: 1, list: 1, label: 1, draft: 1, device: 1 };
    let tested: Vec<(&str, &str)> = requests(&ids).iter().map(|(m, p, _, _)| (*m, *p)).collect();
    let mut missing = Vec::new();
    for source in sources {
        // Each `.route("/path", get(a).post(b))`: its path, then its methods.
        for part in source.split(".route(").skip(1) {
            let Some(path) = part.split('"').nth(1) else { continue };
            // What follows, up to the end of this call (the next line starting a route, or the function's end).
            let methods = part.split("\n}").next().unwrap_or(part);
            for (word, method) in [
                ("get(", "GET"),
                ("post(", "POST"),
                ("put(", "PUT"),
                ("patch(", "PATCH"),
                ("delete(", "DELETE"),
            ] {
                if methods.contains(word) && !tested.contains(&(method, path)) {
                    missing.push(format!("{method} {path}"));
                }
            }
        }
    }
    // `GET /events` is tested on its own (it never ends).
    missing.retain(|m| m != "GET /events");
    assert!(missing.is_empty(), "routes the isolation test doesn't try: {missing:?}");
}
