-- AI becomes an app: it's "on" when it's in the dock (app_settings, id
-- 'ai'), like Tasks and Notes. The old switch (ai_settings.enabled) is no
-- longer read; whoever had AI on keeps it, now as an installed app.
INSERT INTO app_settings (id, enabled, position)
SELECT 'ai', 1, 5 FROM ai_settings WHERE id = 1 AND enabled = 1
ON CONFLICT (id) DO NOTHING;

-- The AI app's conversations (the ⌘K chats are kept here too).
CREATE TABLE ai_conversations (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    title      TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- One message of a conversation. An assistant turn keeps what the answer
-- carried (steps, proposals, the emails and events it cites) as JSON, and
-- which proposals you already confirmed.
CREATE TABLE ai_turns (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id INTEGER NOT NULL REFERENCES ai_conversations(id) ON DELETE CASCADE,
    role            TEXT NOT NULL,              -- user | assistant
    text            TEXT NOT NULL,
    answer          TEXT,                       -- JSON (assistant turns)
    done            TEXT NOT NULL DEFAULT '[]', -- JSON list of confirmed proposal numbers
    created_at      INTEGER NOT NULL
);
CREATE INDEX ai_turns_by_conversation ON ai_turns (conversation_id, id);
