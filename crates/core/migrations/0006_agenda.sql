-- The Agenda: calendars (from mail accounts, CalDAV servers, or Havre
-- itself) and a local copy of their events.

-- CalDAV servers (Nextcloud, iCloud, Fastmail…), not tied to a mail account.
CREATE TABLE caldav_accounts (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL,
    url        TEXT NOT NULL,
    username   TEXT NOT NULL,
    password   BLOB NOT NULL,          -- encrypted, like mail passwords
    created_at INTEGER NOT NULL
);

CREATE TABLE calendars (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    source      TEXT NOT NULL,         -- local | microsoft | google | caldav
    account_id  INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    caldav_id   INTEGER REFERENCES caldav_accounts(id) ON DELETE CASCADE,
    remote_id   TEXT NOT NULL,         -- the provider's id, or the CalDAV URL
    name        TEXT NOT NULL,
    color       TEXT NOT NULL,         -- #rrggbb
    read_only   INTEGER NOT NULL DEFAULT 0,
    enabled     INTEGER NOT NULL DEFAULT 1,   -- shown and synced
    is_default  INTEGER NOT NULL DEFAULT 0,   -- where new events go
    sync_from   INTEGER,               -- the period kept in sync (unix seconds)
    sync_to     INTEGER,
    synced_at   INTEGER,
    error       TEXT,
    UNIQUE (account_id, caldav_id, remote_id)
);

CREATE TABLE events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    remote_id   TEXT NOT NULL,         -- unique per calendar (one per occurrence)
    href        TEXT,                  -- CalDAV: the resource URL
    etag        TEXT,
    raw         TEXT,                  -- CalDAV: the iCalendar text, kept to edit it
    title       TEXT NOT NULL,
    location    TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    start_ts    INTEGER NOT NULL,      -- unix seconds (all-day: midnight UTC)
    end_ts      INTEGER NOT NULL,      -- exclusive
    all_day     INTEGER NOT NULL DEFAULT 0,
    recurring   INTEGER NOT NULL DEFAULT 0,
    read_only   INTEGER NOT NULL DEFAULT 0,
    organizer   TEXT,
    attendees   TEXT NOT NULL DEFAULT '[]',   -- JSON
    join_url    TEXT,
    UNIQUE (calendar_id, remote_id)
);
CREATE INDEX events_by_time ON events (start_ts, end_ts);

-- Havre's own calendar: always there, works without any account.
INSERT INTO calendars (source, remote_id, name, color, is_default)
VALUES ('local', 'havre', 'Havre', '#e3bc6a', 1);
