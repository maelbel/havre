-- Step 9: nightly backups (Settings › Backups). One row.
CREATE TABLE backup_settings (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    enabled    INTEGER NOT NULL DEFAULT 1,
    time       TEXT NOT NULL DEFAULT '03:00',   -- the server's local time
    keep       INTEGER NOT NULL DEFAULT 7,      -- how many backups to keep
    last_at    INTEGER,                         -- last success (unix seconds)
    last_error TEXT                             -- why the last attempt failed
);
INSERT INTO backup_settings (id) VALUES (1);
