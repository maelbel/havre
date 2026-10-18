-- Step 8, organizing mail: stars, labels, snooze.

-- Starred = the IMAP \Flagged flag (Gmail's star, Outlook's flag): it lives
-- on the server, so a star from your phone shows here and the other way.
ALTER TABLE messages ADD COLUMN flagged INTEGER NOT NULL DEFAULT 0;
-- The Message-ID header: the same in every folder (a UID is not), so labels
-- survive moving a message, and "Undo" can find it in its new folder.
-- NULL = not read yet (filled in by the next sync); '' = the message has none.
ALTER TABLE messages ADD COLUMN message_id TEXT;
CREATE INDEX messages_by_message_id ON messages (account_id, message_id);
CREATE INDEX messages_flagged ON messages (flagged) WHERE flagged = 1;

-- Labels are Havre's own (servers disagree on them: Gmail labels are
-- folders, Microsoft 365 has none over IMAP). They work on every account.
CREATE TABLE labels (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    name     TEXT NOT NULL UNIQUE COLLATE NOCASE,
    color    TEXT NOT NULL DEFAULT 'lyon',   -- a design color: work, personal, lyon, hol
    position INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE message_labels (
    label_id   INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    PRIMARY KEY (label_id, account_id, message_id)
) WITHOUT ROWID;
CREATE INDEX message_labels_by_message ON message_labels (account_id, message_id);

-- Snoozed messages: hidden from their folder until `until` (unix seconds).
-- Then Havre marks them unread and `back` = 1 puts them on top of All inboxes.
CREATE TABLE snoozes (
    account_id INTEGER NOT NULL,
    folder     TEXT NOT NULL,
    uid        INTEGER NOT NULL,
    until      INTEGER NOT NULL,
    back       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, folder, uid),
    FOREIGN KEY (account_id, folder, uid) REFERENCES messages(account_id, folder, uid) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX snoozes_due ON snoozes (until) WHERE back = 0;
