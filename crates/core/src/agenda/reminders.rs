//! Ringing event reminders (step 21).
//!
//! Every 30 seconds, [`Agenda`](super::Agenda) asks which reminders are due
//! and announces them on the shared channel (`SyncEvent::EventReminder`): the
//! page shows a toast, Havre Desktop a notification, push reaches the phone.
//!
//! **Exactly once** is the tricky part. Occurrences are rewritten by each
//! sync (their row may be deleted and inserted again), so "already rung" is
//! kept apart, in `event_reminders_rung`, by what identifies an occurrence
//! for good: its calendar, the provider's id and its start. Moving an event
//! changes its start, so the new time rings again — as it should.
//!
//! What it teaches: a time-driven loop, bookkeeping that survives the data
//! being rewritten, and "9:00 your time" with time zones.

use chrono::{NaiveDate, TimeZone};
use chrono_tz::Tz;
use sqlx::Row;

use super::{Event, store::AgendaStore};
use crate::Result;

/// How often the loop looks.
pub const EVERY: std::time::Duration = std::time::Duration::from_secs(30);
/// A reminder found later than this (Havre was off) is noted, not rung: a
/// toast for a meeting that ended long ago helps nobody.
const TOO_LATE: i64 = 15 * 60;
/// "Already rung" marks are kept this long after the event's start.
const KEEP_MARKS: i64 = 2 * 86_400;

/// When a reminder rings: `minutes` before the start. An all-day event
/// starts at midnight *your time* on its day (it's stored as midnight UTC,
/// a date without a clock), so "the day before at 9:00" is right wherever
/// you are.
pub fn ring_at(start: i64, all_day: bool, minutes: i64, tz: Tz) -> i64 {
    let begins = if all_day {
        let day =
            chrono::DateTime::from_timestamp(start, 0).map(|d| d.date_naive()).unwrap_or(NaiveDate::MIN);
        // `earliest`: on a day when clocks jump at midnight, the first moment that exists.
        tz.from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap_or_default())
            .earliest()
            .map_or(start, |t| t.timestamp())
    } else {
        start
    };
    begins - minutes * 60
}

impl AgendaStore {
    /// The events whose reminder is due at `now` and hasn't rung yet, each
    /// with `true` if it should ring (`false`: too late, just note it).
    /// Declined events and hidden calendars don't ring.
    pub(crate) async fn due_reminders(&self, now: i64, tz: Tz) -> Result<Vec<(Event, bool)>> {
        // A generous first cut in SQL (time zones move all-day events by up to
        // 14 hours), the exact time in Rust.
        let rows = sqlx::query(
            "SELECT e.id FROM events e JOIN calendars c ON c.id = e.calendar_id
             WHERE c.user_id = ?2 AND c.enabled = 1 AND e.reminder IS NOT NULL
               AND e.start_ts - e.reminder * 60 - 86400 <= ?1 AND e.start_ts + 86400 >= ?1
               AND COALESCE(e.my_response, '') != 'declined'
               AND NOT EXISTS (SELECT 1 FROM event_reminders_rung r WHERE r.calendar_id = e.calendar_id
                               AND r.remote_id = e.remote_id AND r.start_ts = e.start_ts)",
        )
        .bind(now)
        .bind(self.user.0)
        .fetch_all(&self.pool)
        .await?;
        let mut due = Vec::new();
        for row in rows {
            let event = self.event(row.get("id")).await?;
            let Some(minutes) = event.reminder else { continue };
            let at = ring_at(event.start, event.all_day, minutes, tz);
            if at <= now {
                due.push((event, now - at <= TOO_LATE));
            }
        }
        Ok(due)
    }

    /// Notes that an occurrence's reminder rang (or was skipped).
    pub(crate) async fn mark_rung(&self, event: i64, now: i64) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO event_reminders_rung (calendar_id, remote_id, start_ts, rung_at)
             SELECT calendar_id, remote_id, start_ts, ? FROM events WHERE id = ?",
        )
        .bind(now)
        .bind(event)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Forgets the marks of events long past.
    pub(crate) async fn forget_old_marks(&self, now: i64) -> Result<()> {
        sqlx::query("DELETE FROM event_reminders_rung WHERE start_ts < ?")
            .bind(now - KEEP_MARKS)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_day_reminders_ring_at_your_time() {
        // Tue 6 Oct 2026, an all-day event (midnight UTC).
        let day = 1_791_244_800;
        let paris: Tz = "Europe/Paris".parse().unwrap();
        // The day before at 9:00 in Paris (summer time, UTC+2) is 07:00 UTC.
        assert_eq!(ring_at(day, true, 15 * 60, paris), day - 86_400 + 7 * 3600);
        let new_york: Tz = "America/New_York".parse().unwrap();
        assert_eq!(ring_at(day, true, 15 * 60, new_york), day - 86_400 + 13 * 3600);
        // A timed event: just minutes before.
        assert_eq!(ring_at(day + 3600, false, 10, paris), day + 3600 - 600);
    }
}
