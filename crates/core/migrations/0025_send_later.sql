-- Signatures, undo send and send later (step 24).

-- Each account's signature (plain text, added after a "-- " line), and
-- whether replies and forwards get it too.
ALTER TABLE accounts ADD COLUMN signature TEXT NOT NULL DEFAULT '';
ALTER TABLE accounts ADD COLUMN signature_in_replies INTEGER NOT NULL DEFAULT 1;

-- The signature as it was put in this draft: to swap it when "From" changes.
ALTER TABLE drafts ADD COLUMN signature TEXT NOT NULL DEFAULT '';
-- When to send it (unix seconds). NULL: a draft like any other. Undo send
-- is a send a few seconds from now; send later, at the time you chose.
ALTER TABLE drafts ADD COLUMN send_at INTEGER;
-- 'waiting' (until send_at), 'sending' (claimed by the outbox: never twice),
-- 'failed' (with send_error).
ALTER TABLE drafts ADD COLUMN send_state TEXT;
ALTER TABLE drafts ADD COLUMN send_error TEXT;
CREATE INDEX drafts_to_send ON drafts (send_at) WHERE send_at IS NOT NULL;

-- Mail's own settings.
CREATE TABLE mail_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    -- How long "Undo" stays possible after "Send" (0: off).
    undo_seconds INTEGER NOT NULL DEFAULT 10
);
INSERT INTO mail_settings (id) VALUES (1);
