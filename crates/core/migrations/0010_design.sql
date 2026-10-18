-- Matching the design: message previews, AI permissions and schedule,
-- the MCP switch, and the AI activity log.

-- What the message list shows under the subject, known once the body is
-- downloaded (the newest inbox messages are downloaded in the background).
ALTER TABLE messages ADD COLUMN preview TEXT;
-- Attachments (JSON list of {name, mime, size}); NULL = body not seen yet.
ALTER TABLE messages ADD COLUMN files TEXT;
-- The message carries a meeting invitation (an .ics part).
ALTER TABLE messages ADD COLUMN has_invite INTEGER NOT NULL DEFAULT 0;
-- When the invited meeting starts (unix seconds), for the list's chip.
ALTER TABLE messages ADD COLUMN invite_at INTEGER;

-- What the AI may read, and when the morning brief is written.
ALTER TABLE ai_settings ADD COLUMN read_mail INTEGER NOT NULL DEFAULT 1;
ALTER TABLE ai_settings ADD COLUMN read_agenda INTEGER NOT NULL DEFAULT 1;
ALTER TABLE ai_settings ADD COLUMN read_people INTEGER NOT NULL DEFAULT 1;
ALTER TABLE ai_settings ADD COLUMN brief_enabled INTEGER NOT NULL DEFAULT 1;
ALTER TABLE ai_settings ADD COLUMN brief_time TEXT NOT NULL DEFAULT '07:30';
-- Other AI apps may use Havre over MCP (on if keys were already made).
ALTER TABLE ai_settings ADD COLUMN mcp_enabled INTEGER NOT NULL DEFAULT 0;
UPDATE ai_settings SET mcp_enabled = 1 WHERE EXISTS (SELECT 1 FROM mcp_tokens);

-- Everything the assistant (and MCP apps) read or did: Settings › AI › Activity.
CREATE TABLE ai_activity (
    id     INTEGER PRIMARY KEY AUTOINCREMENT,
    at     INTEGER NOT NULL,
    source TEXT NOT NULL,   -- assistant | brief | summary | draft | mcp:<app name> | you
    action TEXT NOT NULL,   -- e.g. search_mail, created event
    detail TEXT NOT NULL DEFAULT ''
);
CREATE INDEX ai_activity_by_time ON ai_activity (at);
