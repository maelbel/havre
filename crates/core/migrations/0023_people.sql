-- People (step 22): your own details about people, and who you write to.

-- The To and Cc addresses (lowercase, a JSON list). NULL = not read yet: the
-- sync reads them for mail stored before this column, a batch at a time.
ALTER TABLE messages ADD COLUMN recipients TEXT;

-- What you know about someone, beyond what mail and meetings tell.
CREATE TABLE contacts (
    id INTEGER PRIMARY KEY,
    -- Your name for them (empty: the one they use).
    name TEXT NOT NULL DEFAULT '',
    company TEXT NOT NULL DEFAULT '',
    role TEXT NOT NULL DEFAULT '',
    -- JSON list of {label, number}.
    phones TEXT NOT NULL DEFAULT '[]',
    -- 'YYYY-MM-DD', or '--MM-DD' without the year.
    birthday TEXT,
    note TEXT NOT NULL DEFAULT '',
    favorite INTEGER NOT NULL DEFAULT 0,
    -- NULL: guessed from the address; 'person' or 'automated' when you said.
    kind TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- One person, several addresses: each address belongs to one contact.
CREATE TABLE contact_emails (
    email TEXT PRIMARY KEY,
    contact_id INTEGER NOT NULL REFERENCES contacts (id) ON DELETE CASCADE,
    -- 0 = the main address (shown first).
    position INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX contact_emails_by_contact ON contact_emails (contact_id);
