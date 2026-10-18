-- The assistant can read notes and tasks (and propose new ones), like mail,
-- the agenda and people. On by default; it only applies while the Notes or
-- Tasks app is in the dock (Settings › Your apps › AI › What it can read).
ALTER TABLE ai_settings ADD COLUMN read_notes INTEGER NOT NULL DEFAULT 1;
ALTER TABLE ai_settings ADD COLUMN read_tasks INTEGER NOT NULL DEFAULT 1;
