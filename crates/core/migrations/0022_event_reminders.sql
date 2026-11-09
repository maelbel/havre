-- Event reminders (step 21).

-- Minutes before the start (all-day events: before midnight, your time).
-- NULL: no reminder. Read from and written to the provider like the title.
ALTER TABLE events ADD COLUMN reminder INTEGER;

CREATE TABLE agenda_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    -- What new events start with (NULL: none).
    default_reminder INTEGER DEFAULT 10,
    default_all_day_reminder INTEGER,
    -- The IANA time zone your browser last reported ("Europe/Paris"), to ring
    -- all-day reminders at your 9:00 rather than the server's.
    time_zone TEXT
);
INSERT INTO agenda_settings (id) VALUES (1);

-- Reminders already rung: each occurrence rings once, even when a sync
-- rewrites its row. Marks older than two days are removed.
CREATE TABLE event_reminders_rung (
    calendar_id INTEGER NOT NULL,
    remote_id TEXT NOT NULL,
    start_ts INTEGER NOT NULL,
    rung_at INTEGER NOT NULL,
    PRIMARY KEY (calendar_id, remote_id, start_ts)
);

ALTER TABLE push_settings ADD COLUMN event_reminders INTEGER NOT NULL DEFAULT 1;
