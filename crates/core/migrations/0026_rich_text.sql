-- Rich text in Mail (step 25).

-- The formatted text (clean HTML). NULL: a plain-text draft. When set,
-- `body` is the plain text Havre makes from it (the message's text part,
-- search, the AI).
ALTER TABLE drafts ADD COLUMN body_html TEXT;

-- The signature, formatted. `signature` stays: its plain text.
ALTER TABLE accounts ADD COLUMN signature_html TEXT NOT NULL DEFAULT '';

-- How new messages are written: 'rich' or 'plain'.
ALTER TABLE mail_settings ADD COLUMN compose_format TEXT NOT NULL DEFAULT 'rich';
