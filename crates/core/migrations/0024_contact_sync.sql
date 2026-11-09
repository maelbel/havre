-- Contacts sync (step 23): your address books in Google, Microsoft and on
-- CardDAV servers, kept in step with People.

-- Which accounts and servers sync contacts ('account:3', 'caldav:2'). Off
-- until you switch it on: syncing writes into your address books.
CREATE TABLE contact_sources (
    key TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 0
);

-- One address book on one source.
CREATE TABLE contact_books (
    id INTEGER PRIMARY KEY,
    -- 'google', 'microsoft' or 'carddav'.
    source TEXT NOT NULL,
    account_id INTEGER REFERENCES accounts (id) ON DELETE CASCADE,
    caldav_id INTEGER REFERENCES caldav_accounts (id) ON DELETE CASCADE,
    -- The provider's id (Microsoft: a contact folder; CardDAV: the book's URL).
    remote_id TEXT NOT NULL,
    name TEXT NOT NULL,
    read_only INTEGER NOT NULL DEFAULT 0,
    -- Where to start next time (Google's syncToken, CardDAV's ctag).
    sync_token TEXT,
    synced_at INTEGER,
    error TEXT
);

-- A Havre contact <-> a contact in an address book. One Havre contact can
-- be in several books (the same person in Google and in iCloud).
CREATE TABLE contact_links (
    id INTEGER PRIMARY KEY,
    contact_id INTEGER NOT NULL REFERENCES contacts (id) ON DELETE CASCADE,
    book_id INTEGER NOT NULL REFERENCES contact_books (id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL,
    -- CardDAV: the vCard's URL.
    href TEXT,
    etag TEXT,
    -- CardDAV: the vCard as last read, so properties Havre doesn't know are
    -- kept when writing back.
    raw TEXT,
    -- The details as last synced (JSON): what both sides agreed on, to tell
    -- which side changed a field since.
    synced TEXT NOT NULL DEFAULT '{}',
    -- Where to fetch the photo (Google: a URL; Microsoft: 'graph').
    photo_url TEXT,
    UNIQUE (book_id, remote_id)
);
CREATE INDEX contact_links_by_contact ON contact_links (contact_id);

-- A small copy of a linked contact's photo. Empty `data`: it has none.
CREATE TABLE contact_photos (
    link_id INTEGER PRIMARY KEY REFERENCES contact_links (id) ON DELETE CASCADE,
    mime TEXT NOT NULL,
    data BLOB NOT NULL,
    fetched_at INTEGER NOT NULL
);

-- Created by a sync (not by you): removed when it leaves its last address
-- book, unless you made it a favourite or said what it is.
ALTER TABLE contacts ADD COLUMN from_sync INTEGER NOT NULL DEFAULT 0;
-- The address book that last changed the details, and when.
ALTER TABLE contacts ADD COLUMN updated_from TEXT;
ALTER TABLE contacts ADD COLUMN updated_from_at INTEGER;

-- Where the details of someone in no address book go when you edit them
-- (NULL: Havre only).
CREATE TABLE contact_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    default_book INTEGER REFERENCES contact_books (id) ON DELETE SET NULL
);
INSERT INTO contact_settings (id) VALUES (1);
