-- Step 29a: several users. Every piece of data gets an owner.
--
-- Today's owner becomes user 1, the admin, and everything that exists goes to
-- them: nothing changes for an existing Havre.

CREATE TABLE users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    name          TEXT    NOT NULL DEFAULT '',
    -- What they sign in with (29b); NULL for user 1 until they choose one.
    login         TEXT    UNIQUE COLLATE NOCASE,
    -- Argon2, as before. NULL: signs in only through SSO (step 30).
    password_hash TEXT,
    admin         INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
INSERT INTO users (id, password_hash, admin, created_at, updated_at)
    SELECT 1, password_hash, 1, created_at, updated_at FROM owner;
-- A Havre not set up yet has no owner: user 1 exists anyway (no password:
-- the first-run screen still shows), so everything below has its owner.
INSERT OR IGNORE INTO users (id, admin, created_at, updated_at) VALUES (1, 1, unixepoch(), unixepoch());
DROP TABLE owner;

-- Sessions: whose. Old sessions (all the owner's) stay signed in.
ALTER TABLE sessions ADD COLUMN user_id INTEGER REFERENCES users (id) ON DELETE CASCADE;
UPDATE sessions SET user_id = 1;
DELETE FROM sessions WHERE user_id IS NULL;

-- The roots: everything else belongs to one of them (a message to its
-- account, an event to its calendar, a to-do to its list…). The column may be
-- NULL only so that SQLite can add it: a row without an owner is seen by
-- nobody (every query asks for `user_id = ?`).
ALTER TABLE accounts           ADD COLUMN user_id INTEGER REFERENCES users (id) ON DELETE CASCADE;
ALTER TABLE caldav_accounts    ADD COLUMN user_id INTEGER REFERENCES users (id) ON DELETE CASCADE;
ALTER TABLE calendars          ADD COLUMN user_id INTEGER REFERENCES users (id) ON DELETE CASCADE;
ALTER TABLE task_lists         ADD COLUMN user_id INTEGER REFERENCES users (id) ON DELETE CASCADE;
ALTER TABLE notes              ADD COLUMN user_id INTEGER REFERENCES users (id) ON DELETE CASCADE;
ALTER TABLE contacts           ADD COLUMN user_id INTEGER REFERENCES users (id) ON DELETE CASCADE;
ALTER TABLE contact_books      ADD COLUMN user_id INTEGER REFERENCES users (id) ON DELETE CASCADE;
ALTER TABLE push_subscriptions ADD COLUMN user_id INTEGER REFERENCES users (id) ON DELETE CASCADE;
ALTER TABLE push_snoozed       ADD COLUMN user_id INTEGER REFERENCES users (id) ON DELETE CASCADE;
UPDATE accounts           SET user_id = 1;
UPDATE caldav_accounts    SET user_id = 1;
UPDATE calendars          SET user_id = 1;
UPDATE task_lists         SET user_id = 1;
UPDATE notes              SET user_id = 1;
UPDATE contacts           SET user_id = 1;
UPDATE contact_books      SET user_id = 1;
UPDATE push_subscriptions SET user_id = 1;
UPDATE push_snoozed       SET user_id = 1;
CREATE INDEX accounts_user        ON accounts (user_id);
CREATE INDEX caldav_accounts_user ON caldav_accounts (user_id);
CREATE INDEX calendars_user       ON calendars (user_id);
CREATE INDEX task_lists_user      ON task_lists (user_id);
CREATE INDEX notes_user           ON notes (user_id);
CREATE INDEX contacts_user        ON contacts (user_id);
CREATE INDEX contact_books_user   ON contact_books (user_id);
CREATE INDEX push_user            ON push_subscriptions (user_id);

-- Settings that were one row for the whole Havre become one row per user.
-- (SQLite can't change a primary key: new table, copy, drop, rename.)

CREATE TABLE user_apps (
    user_id  INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    id       TEXT    NOT NULL,
    enabled  INTEGER NOT NULL DEFAULT 1,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, id)
);
INSERT INTO user_apps (user_id, id, enabled, position) SELECT 1, id, enabled, position FROM app_settings;
DROP TABLE app_settings;

-- What to push: per user. The VAPID key stays the server's (push_settings).
CREATE TABLE push_prefs (
    user_id         INTEGER PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    new_mail        INTEGER NOT NULL DEFAULT 1,
    mail_details    INTEGER NOT NULL DEFAULT 1,
    reminders       INTEGER NOT NULL DEFAULT 1,
    event_reminders INTEGER NOT NULL DEFAULT 1,
    invitations     INTEGER NOT NULL DEFAULT 1
);
INSERT INTO push_prefs (user_id, new_mail, mail_details, reminders, event_reminders, invitations)
    SELECT 1, new_mail, mail_details, reminders, event_reminders, invitations FROM push_settings WHERE id = 1;

CREATE TABLE agenda_settings_new (
    user_id                  INTEGER PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    default_reminder         INTEGER DEFAULT 10,
    default_all_day_reminder INTEGER,
    time_zone                TEXT
);
INSERT INTO agenda_settings_new (user_id, default_reminder, default_all_day_reminder, time_zone)
    SELECT 1, default_reminder, default_all_day_reminder, time_zone FROM agenda_settings WHERE id = 1;
DROP TABLE agenda_settings;
ALTER TABLE agenda_settings_new RENAME TO agenda_settings;

CREATE TABLE mail_settings_new (
    user_id        INTEGER PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    undo_seconds   INTEGER NOT NULL DEFAULT 10,
    compose_format TEXT    NOT NULL DEFAULT 'rich'
);
INSERT INTO mail_settings_new (user_id, undo_seconds, compose_format)
    SELECT 1, undo_seconds, compose_format FROM mail_settings WHERE id = 1;
DROP TABLE mail_settings;
ALTER TABLE mail_settings_new RENAME TO mail_settings;

CREATE TABLE contact_settings_new (
    user_id      INTEGER PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    default_book INTEGER REFERENCES contact_books (id) ON DELETE SET NULL
);
INSERT INTO contact_settings_new (user_id, default_book) SELECT 1, default_book FROM contact_settings WHERE id = 1;
DROP TABLE contact_settings;
ALTER TABLE contact_settings_new RENAME TO contact_settings;

-- Your answer to an invitation: yours (two users may get the same meeting).
CREATE TABLE invitation_answers_new (
    user_id     INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    uid         TEXT    NOT NULL,
    response    TEXT    NOT NULL,
    answered_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, uid)
);
INSERT INTO invitation_answers_new (user_id, uid, response, answered_at)
    SELECT 1, uid, response, answered_at FROM invitation_answers;
DROP TABLE invitation_answers;
ALTER TABLE invitation_answers_new RENAME TO invitation_answers;

-- Labels: a name is unique per user (two users may both have "Family").
-- Rebuilding `labels` would empty `message_labels` (its rows go with their
-- label: ON DELETE CASCADE), so they are kept aside and put back.
CREATE TABLE message_labels_kept AS SELECT * FROM message_labels;
CREATE TABLE labels_new (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id  INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name     TEXT    NOT NULL COLLATE NOCASE,
    color    TEXT    NOT NULL DEFAULT 'lyon',
    position INTEGER NOT NULL DEFAULT 0,
    UNIQUE (user_id, name)
);
INSERT INTO labels_new (id, user_id, name, color, position) SELECT id, 1, name, color, position FROM labels;
DROP TABLE labels;
ALTER TABLE labels_new RENAME TO labels;
INSERT INTO message_labels SELECT * FROM message_labels_kept;
DROP TABLE message_labels_kept;

-- A contact's addresses: an address is unique per user (your sister may be
-- a contact of yours and of your mother's).
CREATE TABLE contact_emails_new (
    user_id    INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    email      TEXT    NOT NULL,
    contact_id INTEGER NOT NULL REFERENCES contacts (id) ON DELETE CASCADE,
    position   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, email)
);
INSERT INTO contact_emails_new (user_id, email, contact_id, position) SELECT 1, email, contact_id, position FROM contact_emails;
DROP TABLE contact_emails;
ALTER TABLE contact_emails_new RENAME TO contact_emails;
CREATE INDEX contact_emails_by_contact ON contact_emails (contact_id);

-- How far back Files looks: per user (their mail, their choice).
CREATE TABLE files_settings_new (
    user_id INTEGER PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    depth   INTEGER NOT NULL DEFAULT 2000
);
INSERT INTO files_settings_new (user_id, depth) SELECT 1, depth FROM files_settings WHERE id = 1;
DROP TABLE files_settings;
ALTER TABLE files_settings_new RENAME TO files_settings;
