-- The app platform: apps talk to each other.

-- Events know their iCalendar UID (shared by every copy of a meeting: the
-- invitation in your mail, the event in your calendar), and your answer to
-- them when you're a guest.
ALTER TABLE events ADD COLUMN uid TEXT;
ALTER TABLE events ADD COLUMN my_response TEXT;  -- accepted | tentative | declined | needs_action; NULL: yours
CREATE INDEX events_by_uid ON events (uid);

-- Which apps are in the dock, and in what order.
CREATE TABLE app_settings (
    id       TEXT PRIMARY KEY,
    enabled  INTEGER NOT NULL DEFAULT 1,
    position INTEGER NOT NULL DEFAULT 0
);

-- People search looks up senders by address.
CREATE INDEX messages_by_sender ON messages (from_address);

-- Your answers to invitations received by email, by UID (the organizer's
-- calendar knows them; this is so Havre remembers too).
CREATE TABLE invitation_answers (
    uid         TEXT PRIMARY KEY,
    response    TEXT NOT NULL,
    answered_at INTEGER NOT NULL
);

-- Invitations you added to one of your calendars (Microsoft, Google, CalDAV
-- or Havre's): the provider gives your copy its own UID, so we remember
-- which meeting it is a copy of.
CREATE TABLE invitation_copies (
    uid         TEXT NOT NULL,
    calendar_id INTEGER NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    remote_id   TEXT NOT NULL,
    organizer   TEXT,              -- lowercase address (People shows the meeting)
    PRIMARY KEY (calendar_id, remote_id)
);
CREATE INDEX invitation_copies_by_uid ON invitation_copies (uid);
