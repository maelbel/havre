-- Step 26: notifications that act.

-- New invitations get a notification of their own (Accept / Decline).
ALTER TABLE push_settings ADD COLUMN invitations INTEGER NOT NULL DEFAULT 1;

-- A reminder pushed back from a notification ("In 10 min"). When `due_at`
-- comes, the reminder is announced again (push, the open page, Desktop), then
-- the row is deleted: once, for every device.
CREATE TABLE push_snoozed (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    kind    TEXT NOT NULL CHECK (kind IN ('task', 'event')),
    ref_id  INTEGER NOT NULL,      -- the task's or the event's id
    start   INTEGER,               -- an event: which occurrence (repeating events)
    due_at  INTEGER NOT NULL
);
CREATE INDEX push_snoozed_due ON push_snoozed (due_at);
