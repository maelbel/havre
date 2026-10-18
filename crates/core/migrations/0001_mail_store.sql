-- Havre's local copy of the mailbox.
-- The mail server stays the source of truth: this is a cache that makes
-- the UI instant and lets us search and count without asking the server.

CREATE TABLE folders (
    path              TEXT PRIMARY KEY,          -- exact server name
    name              TEXT NOT NULL,             -- decoded display name
    depth             INTEGER NOT NULL,
    role              TEXT NOT NULL,             -- inbox, sent, trash, …
    declared_role     INTEGER NOT NULL DEFAULT 0,
    position          INTEGER NOT NULL DEFAULT 0, -- order in the sidebar
    total             INTEGER NOT NULL DEFAULT 0, -- from IMAP STATUS
    unread            INTEGER NOT NULL DEFAULT 0, -- from IMAP STATUS
    uid_validity      INTEGER,                   -- if it changes, UIDs changed: start over
    oldest_synced_uid INTEGER,                   -- NULL = never synced
    complete          INTEGER NOT NULL DEFAULT 0, -- every older header is stored
    fingerprint       TEXT,                      -- STATUS values last time: unchanged = skip
    synced_at         INTEGER                    -- unix time of the last sync
);

CREATE TABLE messages (
    folder       TEXT NOT NULL REFERENCES folders(path) ON DELETE CASCADE,
    uid          INTEGER NOT NULL,
    subject      TEXT NOT NULL,
    from_name    TEXT,
    from_address TEXT,
    date         TEXT,
    unread       INTEGER NOT NULL,
    search       TEXT NOT NULL,                  -- lowercase, no accents: for LIKE search
    PRIMARY KEY (folder, uid)
) WITHOUT ROWID;

-- Full raw emails, cached the first time they are opened.
CREATE TABLE bodies (
    folder     TEXT NOT NULL,
    uid        INTEGER NOT NULL,
    raw        BLOB NOT NULL,
    fetched_at INTEGER NOT NULL,                 -- unix time
    PRIMARY KEY (folder, uid),
    FOREIGN KEY (folder, uid) REFERENCES messages(folder, uid) ON DELETE CASCADE
);
