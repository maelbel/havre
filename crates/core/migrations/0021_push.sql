-- Push notifications (step 20): the phones and browsers that get them, and
-- what to send.
CREATE TABLE push_subscriptions (
    id INTEGER PRIMARY KEY,
    -- The push service's address for this browser (Google, Apple, Mozilla…).
    endpoint TEXT NOT NULL UNIQUE,
    -- The browser's keys (base64url): its P-256 public key and a shared secret.
    p256dh TEXT NOT NULL,
    auth TEXT NOT NULL,
    -- "Chrome on Android", from the browser.
    label TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    last_ok_at INTEGER,
    last_error TEXT
);

CREATE TABLE push_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    -- Havre's VAPID private key (32 bytes, hex), encrypted with `Secrets`; made on first use.
    vapid_key BLOB,
    new_mail INTEGER NOT NULL DEFAULT 1,
    -- Show the sender and subject (otherwise just "New email").
    mail_details INTEGER NOT NULL DEFAULT 1,
    reminders INTEGER NOT NULL DEFAULT 1
);
INSERT INTO push_settings (id) VALUES (1);
