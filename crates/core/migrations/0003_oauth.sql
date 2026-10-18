-- Accounts can sign in with OAuth ("Sign in with Microsoft") instead of a
-- password. For those, `imap_password` is empty and the long-lived refresh
-- token is stored, encrypted, in `oauth_refresh`.
ALTER TABLE accounts ADD COLUMN auth TEXT NOT NULL DEFAULT 'password'; -- 'password' | 'microsoft'
ALTER TABLE accounts ADD COLUMN oauth_refresh BLOB;
