-- Drafts on the mail server (step 17): each draft also has a copy in its
-- account's Drafts folder, replaced as it changes.

-- One Message-ID for every version of the draft: how its copy is found again.
ALTER TABLE drafts ADD COLUMN message_id TEXT;
-- Counts the changes; `server_version` is the one the server copy holds.
ALTER TABLE drafts ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE drafts ADD COLUMN server_version INTEGER;
-- Where the copy is: account, folder, UID (the account can change while writing).
ALTER TABLE drafts ADD COLUMN server_account INTEGER;
ALTER TABLE drafts ADD COLUMN server_folder TEXT;
ALTER TABLE drafts ADD COLUMN server_uid INTEGER;
ALTER TABLE drafts ADD COLUMN server_saved_at INTEGER;
-- Why the last try failed (shown in the compose window), and when it was.
ALTER TABLE drafts ADD COLUMN server_error TEXT;
ALTER TABLE drafts ADD COLUMN server_tried_at INTEGER;
