-- Repeating events (step 16): which series an occurrence belongs to, and
-- each series' rule, so it can be shown and changed.

-- The provider's id for the whole series (Microsoft's series master, Google's
-- recurring event, the iCalendar UID for CalDAV and Havre's calendar).
ALTER TABLE events ADD COLUMN series TEXT;
-- Which occurrence of the rule this is (unix seconds): stays the same when
-- one occurrence is moved.
ALTER TABLE events ADD COLUMN original_start INTEGER;
CREATE INDEX events_by_series ON events (calendar_id, series);

CREATE TABLE event_series (
    calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    series      TEXT NOT NULL,
    rrule       TEXT,                  -- the rule as the provider wrote it
    repeat      TEXT,                  -- Havre's description (JSON); NULL: a rule Havre can't edit
    time_zone   TEXT,                  -- the IANA zone it repeats in ("Europe/Paris")
    first_start INTEGER NOT NULL,      -- the first occurrence (unix seconds)
    first_end   INTEGER NOT NULL,
    all_day     INTEGER NOT NULL DEFAULT 0,
    ics         TEXT,                  -- Havre's own calendar: the whole series as iCalendar
    fetched_at  INTEGER NOT NULL,
    PRIMARY KEY (calendar_id, series)
);
