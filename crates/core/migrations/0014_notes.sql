-- Notes: Markdown notes kept by Havre, each attached (or not) to a meeting,
-- to people and to an email.

CREATE TABLE notes (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    title           TEXT NOT NULL,
    body            TEXT NOT NULL DEFAULT '',   -- Markdown
    pinned          INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    -- The meeting (like a task's): the iCalendar UID finds it in every
    -- calendar; the start tells the occurrences of a repeating one apart.
    event_uid       TEXT,
    event_title     TEXT,
    event_start     INTEGER,
    -- The email, found again by Message-ID after a move (like a task's).
    mail_account    INTEGER,
    mail_message_id TEXT,
    mail_folder     TEXT,
    mail_uid        INTEGER,
    mail_subject    TEXT,
    mail_from       TEXT
);
CREATE INDEX notes_by_event ON notes (event_uid);
CREATE INDEX notes_by_message ON notes (mail_message_id);

-- A note can be about several people (a meeting's attendees).
CREATE TABLE note_people (
    note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    email   TEXT NOT NULL,                     -- lowercase address
    name    TEXT,
    PRIMARY KEY (note_id, email)
);
CREATE INDEX note_people_by_email ON note_people (email);

-- A `- [ ]` line turned into a task: which task, so the note can say
-- "In Tasks" and tick it with the line. Kept by the line's text (lines
-- move when you edit; their text mostly doesn't).
CREATE TABLE note_tasks (
    note_id INTEGER NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    text    TEXT NOT NULL,
    task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    PRIMARY KEY (note_id, text)
);
