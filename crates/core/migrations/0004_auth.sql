-- Login for Havre itself: one owner (single user for now) and their sessions.

-- The owner's password, hashed with Argon2 (never stored in clear).
CREATE TABLE owner (
    id            INTEGER PRIMARY KEY CHECK (id = 1), -- at most one row
    password_hash TEXT    NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

-- One row per signed-in browser. The cookie holds a random token; only its
-- SHA-256 hash is stored, so a copy of the database can't be used to sign in.
CREATE TABLE sessions (
    token_hash BLOB    PRIMARY KEY,
    created_at INTEGER NOT NULL,
    last_seen  INTEGER NOT NULL,
    user_agent TEXT    NOT NULL DEFAULT ''
);
