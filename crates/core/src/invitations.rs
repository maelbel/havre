//! Invitations: where Mail and the Agenda meet.
//!
//! A meeting invitation arrives as an email with an `.ics` part. Opening it
//! in Mail shows a card (title, time, what else you have then) with
//! **Accept / Maybe / Decline**:
//!
//! - if the meeting is already in a Microsoft or Google calendar (their
//!   servers add invitations themselves), the answer goes through them;
//! - otherwise Havre emails the organizer the standard answer (an iTIP
//!   `REPLY`) and, if you accept, adds the meeting to your default calendar.

use std::sync::Arc;

use serde::Serialize;

use crate::{
    Error, Result,
    accounts::AccountManager,
    agenda::{Agenda, Event, Scope, ics},
};

/// What the invitation card shows.
#[derive(Debug, Clone, Serialize)]
pub struct InvitationView {
    /// `REQUEST`, `CANCEL`…
    pub method: String,
    pub title: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub location: String,
    pub organizer: Option<String>,
    /// More than one occurrence (a repeating meeting).
    pub occurrences: usize,
    /// The meeting in your calendars, if it's there.
    pub event: Option<Event>,
    /// Your answer, if you gave one.
    pub my_response: Option<String>,
    /// What else you have at that time.
    pub conflicts: Vec<Event>,
    /// You're the organizer, or it's a cancellation: no buttons.
    pub can_respond: bool,
}

async fn ics_text(
    accounts: &AccountManager,
    account: i64,
    folder: &str,
    uid: u32,
    index: usize,
) -> Result<String> {
    let file = accounts.engine(account).await?.attachment(folder, uid, index).await?;
    Ok(String::from_utf8_lossy(&file.bytes).into_owned())
}

/// Reads the invitation attached to a message.
pub async fn view(
    accounts: &AccountManager,
    agenda: &Agenda,
    account: i64,
    folder: &str,
    uid: u32,
    index: usize,
) -> Result<InvitationView> {
    let text = ics_text(accounts, account, folder, uid, index).await?;
    build_view(accounts, agenda, account, &text).await
}

async fn build_view(
    accounts: &AccountManager,
    agenda: &Agenda,
    account: i64,
    text: &str,
) -> Result<InvitationView> {
    let invitation = ics::read_invitation(text)?;
    let first =
        invitation.events.first().ok_or_else(|| Error::Config("This invitation has no event.".into()))?;
    let me = accounts.database().account(account).await?;
    let store = agenda.store();
    let existing = match &invitation.uid {
        Some(uid) => store.events_by_uid(uid).await?,
        None => Vec::new(),
    };
    let event = existing.iter().find(|e| e.start == first.start).or(existing.first()).cloned();
    let answered = match &invitation.uid {
        Some(uid) => store.answer(uid).await?,
        None => None,
    };
    // A Microsoft / Google copy knows your answer (it may have changed
    // elsewhere); otherwise, what you answered from Havre.
    let my_response = match event.as_ref().and_then(|e| e.my_response.clone()) {
        Some(r) => Some(r).filter(|r| r != "needs_action"),
        None => answered,
    };
    let conflicts = store
        .events(first.start, first.end.max(first.start + 60))
        .await?
        .into_iter()
        .filter(|e| !e.all_day && e.my_response.as_deref() != Some("declined"))
        // Not the meeting itself (or your copy of it).
        .filter(|e| !existing.iter().any(|x| x.id == e.id))
        .take(5)
        .collect();
    let i_organize = invitation.organizer_email.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&me.email));
    Ok(InvitationView {
        can_respond: invitation.method == "REQUEST" && !i_organize && invitation.organizer_email.is_some(),
        method: invitation.method,
        title: first.title.clone(),
        start: first.start,
        end: first.end,
        all_day: first.all_day,
        location: first.location.clone(),
        organizer: invitation.organizer.clone(),
        occurrences: invitation.events.len(),
        event,
        my_response,
        conflicts,
    })
}

/// An invitation found in your mail that still needs you: not answered,
/// or not in your Agenda yet (the Agenda's "Found in Mail").
#[derive(Debug, Clone, Serialize)]
pub struct FoundInvitation {
    pub account: i64,
    pub folder: String,
    pub uid: u32,
    /// The `.ics` part (to answer or add it).
    pub index: usize,
    pub subject: String,
    pub from: Option<String>,
    pub view: InvitationView,
}

/// Recent invitations in your mail for meetings still to come, that you
/// haven't answered (or, for one you can't answer, haven't added).
pub async fn found_in_mail(
    accounts: &AccountManager,
    agenda: &Agenda,
    limit: usize,
) -> Result<Vec<FoundInvitation>> {
    use sqlx::Row;
    let now = chrono::Utc::now().timestamp();
    // `invite_at` is filled in when a message's body is first read (see
    // `mail::Digest`), so this only looks at what Havre has downloaded.
    let rows = sqlx::query(
        "SELECT account_id, folder, uid, subject, from_name, from_address, files FROM messages
         WHERE has_invite = 1 AND invite_at >= ? ORDER BY invite_at LIMIT 40",
    )
    .bind(now - 3600)
    .fetch_all(&accounts.database().pool())
    .await?;
    let mut found: Vec<FoundInvitation> = Vec::new();
    for r in rows {
        let files: Vec<crate::mail::FileInfo> =
            serde_json::from_str(&r.get::<Option<String>, _>("files").unwrap_or_default())
                .unwrap_or_default();
        let Some(ics) =
            files.iter().find(|f| f.mime == "text/calendar" || f.name.to_lowercase().ends_with(".ics"))
        else {
            continue;
        };
        let (account, folder, uid) =
            (r.get::<i64, _>("account_id"), r.get::<String, _>("folder"), r.get::<i64, _>("uid") as u32);
        // One unreadable invitation shouldn't hide the others.
        let Ok(view) = view(accounts, agenda, account, &folder, uid, ics.index).await else { continue };
        let open = view.method != "CANCEL"
            && view.my_response.is_none()
            && (view.can_respond || view.event.is_none());
        // The same meeting in two messages (an update, a forward): once.
        let seen = found.iter().any(|f| f.view.title == view.title && f.view.start == view.start);
        if open && !seen {
            let name: Option<String> = r.get("from_name");
            let address: Option<String> = r.get("from_address");
            found.push(FoundInvitation {
                account,
                folder,
                uid,
                index: ics.index,
                subject: r.get("subject"),
                from: name.filter(|n| !n.is_empty()).or(address),
                view,
            });
        }
        if found.len() >= limit {
            break;
        }
    }
    Ok(found)
}

/// Accept, maybe or decline (`accepted`, `tentative`, `declined`).
pub async fn respond(
    accounts: &Arc<AccountManager>,
    agenda: &Agenda,
    account: i64,
    folder: &str,
    uid: u32,
    index: usize,
    response: &str,
) -> Result<InvitationView> {
    let partstat = match response {
        "accepted" => "ACCEPTED",
        "tentative" => "TENTATIVE",
        "declined" => "DECLINED",
        other => return Err(Error::Config(format!("unknown answer “{other}”"))),
    };
    let text = ics_text(accounts, account, folder, uid, index).await?;
    let invitation = ics::read_invitation(&text)?;
    let store = agenda.store();
    let existing = match &invitation.uid {
        Some(uid) => store.events_by_uid(uid).await?,
        None => Vec::new(),
    };

    // 1. Already in a Microsoft / Google calendar as an invitation: answer there.
    if let Some(event) = existing.iter().find(|e| e.my_response.is_some()) {
        agenda.respond(event.id, response).await?;
    } else {
        // 2. Otherwise: email the organizer…
        let me = accounts.database().account(account).await?;
        let organizer = invitation
            .organizer_email
            .clone()
            .ok_or_else(|| Error::Config("This invitation doesn't say who to answer.".into()))?;
        let title = invitation.events.first().map(|e| e.title.clone()).unwrap_or_default();
        let verb = match response {
            "accepted" => "Accepted",
            "tentative" => "Tentative",
            _ => "Declined",
        };
        let name = if me.sender_name.is_empty() { me.email.clone() } else { me.sender_name.clone() };
        let reply = ics::reply_ics(&text, &me.email, &me.sender_name, partstat)?;
        let body = match response {
            "accepted" => format!("{name} has accepted the invitation “{title}”."),
            "tentative" => format!("{name} might attend “{title}”."),
            _ => format!("{name} has declined the invitation “{title}”."),
        };
        accounts.send_calendar_reply(account, &organizer, &format!("{verb}: {title}"), &body, &reply).await?;

        // …and keep your calendar in step.
        let mine: Vec<&Event> = existing.iter().filter(|e| !e.read_only).collect();
        if response == "declined" {
            for event in mine {
                agenda.delete(event.id, Scope::This).await?;
            }
        } else if existing.is_empty() {
            agenda.add_from_ics(&text, None).await?;
        }
        if let Some(uid) = &invitation.uid {
            store.remember_answer(uid, response).await?;
        }
    }
    build_view(accounts, agenda, account, &text).await
}
