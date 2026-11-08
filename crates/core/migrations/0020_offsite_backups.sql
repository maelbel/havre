-- Off-site copies of the backups (step 18): one destination, a WebDAV folder
-- or an S3-compatible bucket, and what happened to the last copy sent.
CREATE TABLE backup_remote (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    enabled INTEGER NOT NULL DEFAULT 0,
    kind TEXT NOT NULL DEFAULT 'webdav',
    -- WebDAV: the folder's address. S3: the endpoint.
    url TEXT NOT NULL DEFAULT '',
    bucket TEXT NOT NULL DEFAULT '',
    region TEXT NOT NULL DEFAULT '',
    prefix TEXT NOT NULL DEFAULT '',
    path_style INTEGER NOT NULL DEFAULT 1,
    -- WebDAV user, or S3 access key.
    username TEXT NOT NULL DEFAULT '',
    -- The password or S3 secret key, encrypted with `Secrets` (never sent to the browser).
    secret BLOB,
    -- How many copies to keep at the destination.
    keep INTEGER NOT NULL DEFAULT 14,
    last_at INTEGER,
    last_name TEXT,
    last_size INTEGER,
    last_error TEXT,
    -- When copies started failing (cleared by the next success).
    failing_since INTEGER,
    -- The local backup waiting to be sent (a newer one replaces it).
    pending TEXT,
    next_try_at INTEGER
);
INSERT INTO backup_remote (id) VALUES (1);
