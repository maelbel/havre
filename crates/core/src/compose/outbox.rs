//! The outbox: undo send and send later.
//!
//! Both are the same thing: a draft with a **time to send** (`send_at`).
//! *Send* sets it a few seconds from now (the undo delay); *Undo* clears it
//! before that; *Send later* sets it to the time you chose. A loop
//! ([`outbox_loop`]) sends the drafts whose time has come, with the usual
//! [`AccountManager::send_draft`]. It all happens on the server: closing the
//! page doesn't stop (or speed up) anything.
//!
//! ## Never twice
//!
//! Before sending, a draft is **claimed**: `UPDATE … SET send_state =
//! 'sending' WHERE send_state = 'waiting'`. SQLite runs one write at a time,
//! so if two loops (or a loop and *Send now*) try at once, only one sees
//! "1 row changed": the other doesn't send.
//!
//! And if Havre stops in the middle of sending (a crash, a restart), the
//! message may or may not have left: SMTP can't tell us afterwards. Havre
//! doesn't guess. At the next start the draft is marked *failed*, with
//! "it may have been sent": you look in Sent, then send again or delete it.

use std::{sync::Weak, time::Duration};

use serde::{Deserialize, Serialize};

use super::{Draft, Drafts, SendOutcome, now, recipients};
use crate::{
    Error, Result,
    accounts::AccountManager,
    sync::{AccountEvent, SyncEvent},
};

/// The undo delays offered (seconds; 0 = off).
pub const UNDO_CHOICES: [i64; 5] = [0, 5, 10, 20, 30];
/// Sent more than this after its time (Havre was off): said so.
const LATE: i64 = 5 * 60;
/// Later than this, a scheduled message leaves the server's Drafts folder
/// (it could otherwise be sent from another mail app too).
const SCHEDULED: i64 = 60;
/// How often the outbox looks for messages to send.
const EVERY: Duration = Duration::from_secs(2);

const INTERRUPTED: &str = "Havre stopped while sending this message: it may have gone. Look in Sent, then send \
                           it again or delete it.";

/// Mail's settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MailSettings {
    /// How long "Undo" stays possible after "Send" (0: off).
    pub undo_seconds: i64,
}

/// What "Send" answers.
#[derive(Debug, Clone, Serialize)]
pub struct SendReply {
    /// When it goes (or went).
    pub send_at: i64,
    /// Already sent (no undo delay): what happened.
    pub outcome: Option<SendOutcome>,
}

impl Drafts {
    pub async fn mail_settings(&self) -> Result<MailSettings> {
        let undo_seconds = sqlx::query_scalar("SELECT undo_seconds FROM mail_settings WHERE id = 1")
            .fetch_one(&self.pool)
            .await?;
        Ok(MailSettings { undo_seconds })
    }

    pub async fn set_mail_settings(&self, settings: &MailSettings) -> Result<MailSettings> {
        if !UNDO_CHOICES.contains(&settings.undo_seconds) {
            return Err(Error::Config("Undo lasts 0, 5, 10, 20 or 30 seconds.".into()));
        }
        sqlx::query("UPDATE mail_settings SET undo_seconds = ? WHERE id = 1")
            .bind(settings.undo_seconds)
            .execute(&self.pool)
            .await?;
        self.mail_settings().await
    }

    /// Sets when to send a draft. Not one being sent right now.
    pub(crate) async fn schedule(&self, id: i64, at: i64) -> Result<()> {
        let changed = sqlx::query(
            "UPDATE drafts SET send_at = ?, send_state = 'waiting', send_error = NULL
             WHERE id = ? AND (send_state IS NULL OR send_state IN ('waiting', 'failed'))",
        )
        .bind(at)
        .bind(id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if changed == 0 {
            self.get(id).await?; // NotFound if it's gone (sent).
            return Err(Error::Config("This message is being sent right now.".into()));
        }
        Ok(())
    }

    /// Undo / Don't send: back to a draft (and back to the server's Drafts
    /// folder, at the next save). Too late once it's being sent.
    pub async fn unschedule(&self, id: i64) -> Result<Draft> {
        let changed = sqlx::query(
            "UPDATE drafts SET send_at = NULL, send_state = NULL, send_error = NULL, server_version = NULL
             WHERE id = ? AND send_state IN ('waiting', 'failed')",
        )
        .bind(id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if changed == 0 {
            return Err(match self.get(id).await {
                Ok(_) => Error::Config("Too late: it's being sent.".into()),
                Err(_) => Error::Config("Too late: it's already sent.".into()),
            });
        }
        self.get(id).await
    }

    /// Claims a draft to send it: `true` for exactly one caller (see the
    /// module's "Never twice").
    async fn claim(&self, id: i64, now: i64) -> Result<bool> {
        let changed = sqlx::query(
            "UPDATE drafts SET send_state = 'sending'
             WHERE id = ? AND send_state = 'waiting' AND send_at <= ?",
        )
        .bind(id)
        .bind(now)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(changed == 1)
    }

    /// The drafts whose time has come, oldest first.
    async fn due(&self, now: i64) -> Result<Vec<(i64, i64)>> {
        Ok(sqlx::query_as(
            "SELECT id, send_at FROM drafts WHERE send_state = 'waiting' AND send_at <= ? ORDER BY send_at LIMIT 20",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn send_failed(&self, id: i64, error: &str) -> Result<()> {
        sqlx::query("UPDATE drafts SET send_state = 'failed', send_error = ? WHERE id = ?")
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// At start: drafts left `sending` (Havre stopped in the middle) become
    /// failed, never sent again on their own. Returns how many.
    async fn interrupted(&self) -> Result<u64> {
        Ok(sqlx::query(
            "UPDATE drafts SET send_state = 'failed', send_error = ? WHERE send_state = 'sending'",
        )
        .bind(INTERRUPTED)
        .execute(&self.pool)
        .await?
        .rows_affected())
    }

    /// Its copy on the server was removed (scheduled for later).
    async fn forget_server_copy(&self, id: i64) -> Result<()> {
        sqlx::query(
            "UPDATE drafts SET server_version = NULL, server_account = NULL, server_folder = NULL,
                               server_uid = NULL, server_saved_at = NULL
             WHERE id = ?",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Messages waiting to be sent (and those that failed), soonest first.
    pub async fn scheduled(&self) -> Result<Vec<Draft>> {
        let ids: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM drafts WHERE send_at IS NOT NULL ORDER BY send_at, id")
                .fetch_all(&self.pool)
                .await?;
        let mut out = Vec::new();
        for id in ids {
            out.push(self.get(id).await?);
        }
        Ok(out)
    }
}

/// The recipients are checked when you press Send, not when it goes.
fn check_recipients(draft: &Draft) -> Result<()> {
    let (to, cc, bcc) =
        (recipients(&draft.to, "To")?, recipients(&draft.cc, "Cc")?, recipients(&draft.bcc, "Bcc")?);
    if to.iter().chain(cc.iter()).chain(bcc.iter()).next().is_none() {
        return Err(Error::Config("Add at least one recipient.".into()));
    }
    Ok(())
}

impl AccountManager {
    /// "Send" (`at`: `None`) or "Send later" (`at`: a time). Without an undo
    /// delay, "Send" sends right away and answers what happened.
    pub async fn send_draft_at(&self, id: i64, at: Option<i64>) -> Result<SendReply> {
        let drafts = self.drafts();
        let draft = drafts.get(id).await?;
        check_recipients(&draft)?;
        let now = now();
        let at = match at {
            Some(at) if at < now - 60 => return Err(Error::Config("Pick a time that hasn't passed.".into())),
            Some(at) => at.max(now),
            None => now + drafts.mail_settings().await?.undo_seconds,
        };
        drafts.schedule(id, at).await?;
        if at <= now {
            let outcome = self.send_due(id, at).await?;
            return Ok(SendReply { send_at: at, outcome: Some(outcome) });
        }
        if at > now + SCHEDULED {
            // Out of the server's Drafts folder, so another mail app can't send it too.
            let _one_at_a_time = self.drafts_lock.lock().await;
            let draft = drafts.get(id).await?;
            if self.remove_server_copy(&draft).await {
                drafts.forget_server_copy(id).await?;
            }
        }
        announce(self, id, &draft.subject, Outcome::Scheduled);
        Ok(SendReply { send_at: at, outcome: None })
    }

    /// Undo / Don't send.
    pub async fn unsend(&self, id: i64) -> Result<Draft> {
        let draft = self.drafts().unschedule(id).await?;
        announce(self, id, &draft.subject, Outcome::Scheduled);
        Ok(draft)
    }

    /// Sends a draft whose time has come, if this call wins the claim.
    async fn send_due(&self, id: i64, send_at: i64) -> Result<SendOutcome> {
        let drafts = self.drafts();
        let subject = drafts.get(id).await?.subject;
        if !drafts.claim(id, now()).await? {
            return Err(Error::Config("This message is already being sent.".into()));
        }
        match self.send_draft(id).await {
            Ok(outcome) => {
                let late = now() - send_at > LATE;
                announce(self, id, &subject, Outcome::Sent { late, warning: outcome.warning.clone() });
                Ok(outcome)
            }
            Err(err) => {
                let message = shown(&err);
                drafts.send_failed(id, &message).await?;
                announce(self, id, &subject, Outcome::Failed(message));
                Err(err)
            }
        }
    }
}

/// Why it didn't go, as a sentence for you (the details are in the log).
fn shown(err: &Error) -> String {
    match err {
        Error::Config(message) | Error::SignIn(message) | Error::NotFound(message) => message.clone(),
        other => {
            tracing::warn!("sending: {other}");
            "The outgoing server didn't take it. Havre didn't send it: try again.".into()
        }
    }
}

enum Outcome {
    /// Scheduled, or back to a draft: lists to refresh.
    Scheduled,
    Sent {
        late: bool,
        warning: Option<String>,
    },
    Failed(String),
}

/// Tells the page (and push) what the outbox did.
fn announce(manager: &AccountManager, draft: i64, subject: &str, outcome: Outcome) {
    let (sent, late, error) = match outcome {
        Outcome::Scheduled => (false, false, None),
        Outcome::Sent { late, warning } => (true, late, warning),
        Outcome::Failed(error) => (false, false, Some(error)),
    };
    let event = SyncEvent::Outbox { draft, subject: subject.to_owned(), sent, late, error };
    let _ = manager.sender().send(AccountEvent { account: 0, event });
}

/// Every 2 s: sends the messages whose time has come. A `Weak` pointer,
/// like the drafts loop: it mustn't keep the manager alive.
pub(crate) async fn outbox_loop(manager: Weak<AccountManager>) {
    if let Some(manager) = manager.upgrade() {
        match manager.drafts().interrupted().await {
            Ok(0) => {}
            Ok(n) => tracing::warn!("{n} message(s) were being sent when Havre stopped: marked to check"),
            Err(err) => tracing::warn!("outbox: {err}"),
        }
    }
    loop {
        tokio::time::sleep(EVERY).await;
        let Some(manager) = manager.upgrade() else { return };
        let due = match manager.drafts().due(now()).await {
            Ok(due) => due,
            Err(err) => {
                tracing::warn!(%err, "outbox");
                continue;
            }
        };
        for (id, send_at) in due {
            // A failure is kept on the draft and announced: nothing else to do here.
            if let Err(err) = manager.send_due(id, send_at).await {
                tracing::warn!(draft = id, "not sent: {err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{DraftKind, DraftUpdate, Prefill};

    #[tokio::test]
    async fn scheduling_claiming_and_undoing() {
        let db = crate::store::Database::in_memory().await.unwrap();
        let account = db
            .insert_account(&crate::store::NewAccount {
                name: "Me".into(),
                email: "me@example.org".into(),
                imap_host: "mail.example.org".into(),
                imap_port: 993,
                imap_user: "me".into(),
                color: "work".into(),
                password: "x".into(),
                sender_name: String::new(),
                smtp_host: None,
                smtp_port: None,
                smtp_security: None,
            })
            .await
            .unwrap();
        let drafts = db.drafts();
        let id = drafts.insert(account.id, DraftKind::New, &Prefill::default(), None).await.unwrap();
        let update = DraftUpdate { to: Some("anna@example.com".into()), ..Default::default() };
        let draft = drafts.update(id, &update).await.unwrap();
        assert!(check_recipients(&draft).is_ok());

        // Scheduled for t=1000: not due before, due after; out of the server-copy loop meanwhile.
        drafts.schedule(id, 1000).await.unwrap();
        assert!(drafts.due(999).await.unwrap().is_empty());
        assert_eq!(drafts.due(1000).await.unwrap(), [(id, 1000)]);
        assert!(drafts.due_on_server(i64::MAX / 2).await.unwrap().is_empty());
        assert_eq!(drafts.scheduled().await.unwrap()[0].send_state.as_deref(), Some("waiting"));

        // Two claims at once: one wins.
        assert!(!drafts.claim(id, 999).await.unwrap(), "not yet");
        assert!(drafts.claim(id, 1000).await.unwrap());
        assert!(!drafts.claim(id, 1000).await.unwrap(), "never twice");
        // Being sent: no undo, no rescheduling.
        assert!(drafts.unschedule(id).await.is_err());
        assert!(drafts.schedule(id, 2000).await.is_err());

        // Havre stopped meanwhile: at the next start it's failed, not sent again.
        assert_eq!(drafts.interrupted().await.unwrap(), 1);
        let failed = drafts.get(id).await.unwrap();
        assert_eq!(failed.send_state.as_deref(), Some("failed"));
        assert!(failed.send_error.unwrap().contains("may have gone"));
        assert!(drafts.due(i64::MAX / 2).await.unwrap().is_empty());

        // You send it again, then undo: a draft again, back to the server's Drafts later.
        drafts.schedule(id, 3000).await.unwrap();
        let back = drafts.unschedule(id).await.unwrap();
        assert_eq!((back.send_at, back.send_state), (None, None));
        assert!(back.server_pending);
        assert_eq!(drafts.due_on_server(i64::MAX / 2).await.unwrap(), [id]);

        // Settings: only the offered delays.
        assert_eq!(drafts.mail_settings().await.unwrap().undo_seconds, 10);
        assert!(drafts.set_mail_settings(&MailSettings { undo_seconds: 7 }).await.is_err());
        assert_eq!(
            drafts.set_mail_settings(&MailSettings { undo_seconds: 30 }).await.unwrap().undo_seconds,
            30
        );

        // A new message holding only its signature isn't worth a copy on the server.
        let update = crate::store::AccountUpdate { signature: Some("Maël".into()), ..Default::default() };
        db.update_account(account.id, &update).await.unwrap();
        let signed = drafts.insert(account.id, DraftKind::New, &Prefill::default(), None).await.unwrap();
        assert!(drafts.get(signed).await.unwrap().is_empty());
        assert!(!drafts.due_on_server(i64::MAX / 2).await.unwrap().contains(&signed));
        drafts.delete(signed).await.unwrap();

        // No recipient: refused when you press Send, not later.
        let empty = drafts.insert(account.id, DraftKind::New, &Prefill::default(), None).await.unwrap();
        assert!(check_recipients(&drafts.get(empty).await.unwrap()).is_err());
    }
}
