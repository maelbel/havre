-- Tasks: to-dos from Microsoft To Do, Google Tasks, CalDAV task lists, or
-- Havre itself, with links to the email, the person and the meeting they
-- belong to.

CREATE TABLE task_lists (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    source      TEXT NOT NULL,         -- local | microsoft | google | caldav
    account_id  INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    caldav_id   INTEGER REFERENCES caldav_accounts(id) ON DELETE CASCADE,
    remote_id   TEXT NOT NULL,         -- the provider's id, or the CalDAV URL
    name        TEXT NOT NULL,
    color       TEXT NOT NULL,         -- #rrggbb
    read_only   INTEGER NOT NULL DEFAULT 0,
    is_default  INTEGER NOT NULL DEFAULT 0,   -- where new tasks go
    synced_at   INTEGER,
    error       TEXT
);

CREATE TABLE tasks (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    list_id         INTEGER NOT NULL REFERENCES task_lists(id) ON DELETE CASCADE,
    remote_id       TEXT NOT NULL,     -- unique per list
    href            TEXT,              -- CalDAV: the resource URL
    etag            TEXT,
    raw             TEXT,              -- CalDAV: the iCalendar text, kept to edit it
    title           TEXT NOT NULL,
    notes           TEXT NOT NULL DEFAULT '',
    due             INTEGER,           -- the day it's due: midnight UTC (like all-day events)
    due_at          INTEGER,           -- with a time: that moment (unix seconds)
    remind_at       INTEGER,           -- when to show a reminder
    reminded        INTEGER NOT NULL DEFAULT 0,
    done_at         INTEGER,           -- NULL: still to do
    updated_at      INTEGER NOT NULL,
    -- Links, kept by Havre only (the providers don't know about them):
    mail_account    INTEGER,
    mail_message_id TEXT,              -- finds the email again, even after it moved
    mail_folder     TEXT,              -- where it was, when it has no Message-ID
    mail_uid        INTEGER,
    mail_subject    TEXT,
    mail_from       TEXT,
    person          TEXT,              -- lowercase address
    person_name     TEXT,
    event_uid       TEXT,
    event_title     TEXT,
    event_start     INTEGER,
    UNIQUE (list_id, remote_id)
);
CREATE INDEX tasks_by_due ON tasks (due);
CREATE INDEX tasks_by_person ON tasks (person);
CREATE INDEX tasks_by_message ON tasks (mail_message_id);

-- Which accounts sync their tasks: key `account:<id>` or `caldav:<id>`.
-- No row: mail accounts sync (when signed in with permission), CalDAV
-- servers don't (many have no task lists).
CREATE TABLE task_sync (
    key     TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL
);

-- Havre's own list: always there, works without any account.
INSERT INTO task_lists (source, remote_id, name, color, is_default)
VALUES ('local', 'inbox', 'Inbox', '#c99ab0', 1);
