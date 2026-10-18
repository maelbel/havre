-- Several mail accounts, configured from the web UI.
--
-- The mail cache (folders, messages, bodies) gains an account column. It is
-- only a copy of what's on the mail servers, so instead of converting it we
-- drop it: the next sync downloads everything again.

DROP TABLE bodies;
DROP TABLE messages;
DROP TABLE folders;

CREATE TABLE accounts (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    name          TEXT NOT NULL,              -- "Work", "Personal"…
    email         TEXT NOT NULL,
    color         TEXT NOT NULL DEFAULT 'work',
    imap_host     TEXT NOT NULL,
    imap_port     INTEGER NOT NULL DEFAULT 993,
    imap_user     TEXT NOT NULL,
    imap_password BLOB NOT NULL,              -- encrypted: nonce + ciphertext
    position      INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL
);

CREATE TABLE folders (
    account_id        INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    path              TEXT NOT NULL,
    name              TEXT NOT NULL,
    depth             INTEGER NOT NULL,
    role              TEXT NOT NULL,
    declared_role     INTEGER NOT NULL DEFAULT 0,
    position          INTEGER NOT NULL DEFAULT 0,
    total             INTEGER NOT NULL DEFAULT 0,
    unread            INTEGER NOT NULL DEFAULT 0,
    uid_validity      INTEGER,
    oldest_synced_uid INTEGER,
    complete          INTEGER NOT NULL DEFAULT 0,
    fingerprint       TEXT,
    synced_at         INTEGER,
    PRIMARY KEY (account_id, path)
);

CREATE TABLE messages (
    account_id   INTEGER NOT NULL,
    folder       TEXT NOT NULL,
    uid          INTEGER NOT NULL,
    subject      TEXT NOT NULL,
    from_name    TEXT,
    from_address TEXT,
    date         TEXT,
    unread       INTEGER NOT NULL,
    search       TEXT NOT NULL,
    PRIMARY KEY (account_id, folder, uid),
    FOREIGN KEY (account_id, folder) REFERENCES folders(account_id, path) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE TABLE bodies (
    account_id INTEGER NOT NULL,
    folder     TEXT NOT NULL,
    uid        INTEGER NOT NULL,
    raw        BLOB NOT NULL,
    size       INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL,              -- unix time; oldest are evicted first
    PRIMARY KEY (account_id, folder, uid),
    FOREIGN KEY (account_id, folder, uid) REFERENCES messages(account_id, folder, uid) ON DELETE CASCADE
);
CREATE INDEX bodies_by_age ON bodies (fetched_at);
