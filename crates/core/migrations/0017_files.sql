-- Files: every attachment in your mail, listed from what Havre knows about
-- each message (`messages.files`, filled when a message is downloaded).
-- With the Files app in the dock, Havre also looks through older mail to
-- find them: how far back is the only setting.
CREATE TABLE files_settings (
    id    INTEGER PRIMARY KEY CHECK (id = 1),
    -- How many of the newest emails of each Inbox and Sent to look through
    -- (0: all of them).
    depth INTEGER NOT NULL DEFAULT 2000
);
INSERT INTO files_settings (id) VALUES (1);

-- The Files list reads only messages with attachments: a partial index
-- (only those rows are in it) keeps it quick on a big mailbox.
CREATE INDEX messages_with_files ON messages (account_id, folder) WHERE files IS NOT NULL AND files != '[]';
