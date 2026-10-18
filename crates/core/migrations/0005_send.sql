-- Writing mail.

-- The outgoing (SMTP) server of each account. NULL = guessed from the IMAP
-- server (imap.example.com → smtp.example.com). The login is the IMAP one.
ALTER TABLE accounts ADD COLUMN smtp_host TEXT;
ALTER TABLE accounts ADD COLUMN smtp_port INTEGER;
ALTER TABLE accounts ADD COLUMN smtp_security TEXT; -- 'tls' (port 465) | 'starttls' (port 587)

-- Your name as recipients see it ("Maël Belliard <mael@…>").
ALTER TABLE accounts ADD COLUMN sender_name TEXT NOT NULL DEFAULT '';

-- What an OAuth2 sign-in allows. NULL: signed in before sending existed
-- (reading only, for Microsoft: reconnect to send).
ALTER TABLE accounts ADD COLUMN oauth_scope TEXT;

-- Messages being written. Kept until sent or discarded, so nothing is lost
-- if the page closes.
CREATE TABLE drafts (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id    INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    kind          TEXT NOT NULL DEFAULT 'new',   -- new | reply | reply_all | forward
    to_addrs      TEXT NOT NULL DEFAULT '',
    cc_addrs      TEXT NOT NULL DEFAULT '',
    bcc_addrs     TEXT NOT NULL DEFAULT '',
    subject       TEXT NOT NULL DEFAULT '',
    body          TEXT NOT NULL DEFAULT '',
    in_reply_to   TEXT,                          -- threading headers of replies
    refs          TEXT,
    source_folder TEXT,                          -- the message replied to / forwarded
    source_uid    INTEGER,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE draft_attachments (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    draft_id INTEGER NOT NULL REFERENCES drafts(id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    mime     TEXT NOT NULL,
    data     BLOB NOT NULL
);
CREATE INDEX draft_attachments_by_draft ON draft_attachments (draft_id);
