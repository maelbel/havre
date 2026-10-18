-- AI (optional, off by default).

-- One row: which provider, which model. The API key is encrypted like the
-- mail passwords.
CREATE TABLE ai_settings (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    enabled     INTEGER NOT NULL DEFAULT 0,
    provider    TEXT NOT NULL DEFAULT 'ollama',   -- ollama | openai | anthropic
    url         TEXT NOT NULL DEFAULT '',         -- empty: the provider's usual address
    model       TEXT NOT NULL DEFAULT '',
    api_key     BLOB,                             -- encrypted
    embed_url   TEXT NOT NULL DEFAULT '',         -- search by meaning: empty = same server
    embed_model TEXT NOT NULL DEFAULT '',         -- empty: search by meaning off
    index_mail  INTEGER NOT NULL DEFAULT 0
);
INSERT INTO ai_settings (id) VALUES (1);

-- Texts already written (summaries, the morning brief), so reopening is free.
CREATE TABLE ai_cache (
    key        TEXT PRIMARY KEY,
    text       TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

-- Search by meaning: one vector ("embedding") per message. Deleted with
-- the message.
CREATE TABLE mail_vectors (
    account_id INTEGER NOT NULL,
    folder     TEXT NOT NULL,
    uid        INTEGER NOT NULL,
    model      TEXT NOT NULL,
    with_body  INTEGER NOT NULL,     -- 0: only sender and subject were known
    vector     BLOB NOT NULL,        -- little-endian f32, length 1
    PRIMARY KEY (account_id, folder, uid),
    FOREIGN KEY (account_id, folder, uid) REFERENCES messages(account_id, folder, uid) ON DELETE CASCADE
);
