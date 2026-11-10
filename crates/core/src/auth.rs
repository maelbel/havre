//! The login for Havre itself.
//!
//! Your mail is readable by anyone who can open Havre's page, so Havre asks
//! for a password first. Several users (step 29): user 1 is the admin, the
//! one who set Havre up; a session belongs to one user, and every request
//! made with it acts as that user ([`SignedIn`]).
//!
//! - **Password**: hashed with Argon2, a deliberately slow, memory-hungry
//!   hash made for passwords, so a stolen database is very hard to crack.
//! - **Sessions**: signing in creates a random token, kept by the browser in
//!   an `HttpOnly` cookie (JavaScript can't read it). Only the token's
//!   SHA-256 hash is stored. A session lasts 30 days after the last visit.
//!
//! The web server decides *when* to check (see `havre-server`'s `auth.rs`);
//! this module only knows *how*.

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::{Error, Result, users::UserId};

/// A session ends after this long without a visit.
pub const SESSION_DAYS: i64 = 30;
/// The shortest password accepted.
pub const MIN_PASSWORD: usize = 8;

/// Password and sessions, stored in the database.
#[derive(Clone)]
pub struct Auth {
    pool: SqlitePool,
}

/// Who a session belongs to: what every request acts as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedIn {
    pub user: UserId,
    pub admin: bool,
}

/// The admin: user 1, made at first run.
const ADMIN: i64 = 1;

/// A signed-in browser, as listed in Settings.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionInfo {
    pub created_at: i64,
    pub last_seen: i64,
    pub user_agent: String,
    /// The browser asking.
    pub current: bool,
}

impl Auth {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Has the admin chosen a password yet? If not, the web UI shows the
    /// first-run screen.
    pub async fn is_set_up(&self) -> Result<bool> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = ? AND password_hash IS NOT NULL")
                .bind(ADMIN)
                .fetch_one(&self.pool)
                .await?;
        Ok(count > 0)
    }

    /// First run: sets the admin's password and signs in. Refused once a
    /// password exists, so nobody can take over an installed Havre this way.
    pub async fn set_up(&self, password: &str, user_agent: &str) -> Result<String> {
        check_strength(password)?;
        let hash = hash_password(password).await?;
        let now = now();
        // The admin row exists after a reset (their data stays); else it's made.
        sqlx::query(
            "INSERT INTO users (id, admin, created_at, updated_at) VALUES (?, 1, ?, ?)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(ADMIN)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        let set = sqlx::query(
            "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ? AND password_hash IS NULL",
        )
        .bind(&hash)
        .bind(now)
        .bind(ADMIN)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if set == 0 {
            return Err(Error::SignIn("Havre already has a password: sign in instead.".into()));
        }
        self.create_session(UserId(ADMIN), user_agent).await
    }

    /// Checks the password; on success, returns a new session token. (Only
    /// the admin signs in with a password alone; others give a name, 29b.)
    pub async fn sign_in(&self, password: &str, user_agent: &str) -> Result<String> {
        if !self.check_password(UserId(ADMIN), password).await? {
            return Err(Error::SignIn("Wrong password.".into()));
        }
        self.create_session(UserId(ADMIN), user_agent).await
    }

    async fn check_password(&self, user: UserId, password: &str) -> Result<bool> {
        let hash: Option<Option<String>> = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
            .bind(user.0)
            .fetch_optional(&self.pool)
            .await?;
        let Some(Some(hash)) = hash else { return Ok(false) };
        verify_password(password, hash).await
    }

    /// Changes `user`'s password and signs out their other browsers.
    pub async fn change_password(
        &self,
        user: UserId,
        current: &str,
        new: &str,
        keep_token: &str,
    ) -> Result<()> {
        if !self.check_password(user, current).await? {
            return Err(Error::SignIn("The current password is wrong.".into()));
        }
        check_strength(new)?;
        let hash = hash_password(new).await?;
        sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
            .bind(hash)
            .bind(now())
            .bind(user.0)
            .execute(&self.pool)
            .await?;
        self.sign_out_others(user, keep_token).await
    }

    /// Forgets the admin's password and every session: the next visit shows
    /// the first-run screen again (`HAVRE_RESET_PASSWORD=1`, if it's
    /// forgotten). Nobody's data is touched.
    pub async fn reset(&self) -> Result<()> {
        sqlx::query("DELETE FROM sessions").execute(&self.pool).await?;
        sqlx::query("UPDATE users SET password_hash = NULL WHERE id = ?")
            .bind(ADMIN)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---- Sessions ---------------------------------------------------------------------

    /// Signs `user` in on this browser: a new session, its token for the
    /// cookie. Only once the caller knows who it is (their password checked,
    /// or, in step 30, their identity provider's word).
    pub async fn create_session(&self, user: UserId, user_agent: &str) -> Result<String> {
        // 256 random bits: impossible to guess.
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        let now = now();
        sqlx::query(
            "INSERT INTO sessions (token_hash, created_at, last_seen, user_agent, user_id) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(token_hash(&token))
        .bind(now)
        .bind(now)
        .bind(user_agent.chars().take(200).collect::<String>())
        .bind(user.0)
        .execute(&self.pool)
        .await?;
        Ok(token)
    }

    /// Whose live session this cookie token is (`None`: none). Also extends
    /// it (at most once a minute, to avoid a database write on every request).
    pub async fn check_session(&self, token: &str) -> Result<Option<SignedIn>> {
        let now = now();
        let row: Option<(i64, i64, bool)> = sqlx::query_as(
            "SELECT s.last_seen, u.id, u.admin FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ?",
        )
        .bind(token_hash(token))
        .fetch_optional(&self.pool)
        .await?;
        match row {
            None => Ok(None),
            Some((seen, ..)) if now - seen > SESSION_DAYS * 86_400 => {
                self.sign_out(token).await?;
                Ok(None)
            }
            Some((seen, user, admin)) => {
                if now - seen > 60 {
                    sqlx::query("UPDATE sessions SET last_seen = ? WHERE token_hash = ?")
                        .bind(now)
                        .bind(token_hash(token))
                        .execute(&self.pool)
                        .await?;
                }
                Ok(Some(SignedIn { user: UserId(user), admin }))
            }
        }
    }

    pub async fn sign_out(&self, token: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(token_hash(token))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Signs out every browser of `user` except this one.
    pub async fn sign_out_others(&self, user: UserId, keep_token: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash <> ?")
            .bind(user.0)
            .bind(token_hash(keep_token))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// `user`'s signed-in browsers, most recent first.
    pub async fn sessions(&self, user: UserId, current_token: &str) -> Result<Vec<SessionInfo>> {
        let current = token_hash(current_token);
        let rows: Vec<(Vec<u8>, i64, i64, String)> = sqlx::query_as(
            "SELECT token_hash, created_at, last_seen, user_agent FROM sessions WHERE user_id = ?
             ORDER BY last_seen DESC",
        )
        .bind(user.0)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(hash, created_at, last_seen, user_agent)| SessionInfo {
                created_at,
                last_seen,
                user_agent,
                current: hash == current,
            })
            .collect())
    }
}

fn check_strength(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(Error::Config(format!("Choose a password of at least {MIN_PASSWORD} characters.")));
    }
    Ok(())
}

/// Argon2 takes a noticeable moment on purpose: run it on a thread meant
/// for blocking work, so the server keeps answering other requests.
async fn hash_password(password: &str) -> Result<String> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
        // A fresh random salt is made for each hash (argon2 0.6 does it itself).
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(|err| Error::Config(format!("can't hash the password: {err}")))
    })
    .await
    .map_err(|err| Error::Config(format!("can't hash the password: {err}")))?
}

async fn verify_password(password: &str, hash: String) -> Result<bool> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
        let parsed =
            PasswordHash::new(&hash).map_err(|err| Error::Config(format!("bad stored hash: {err}")))?;
        Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
    })
    .await
    .map_err(|err| Error::Config(format!("can't check the password: {err}")))?
}

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use crate::{store::Database, users::UserId};

    /// Passwords set before the move to argon2 0.6 still work: the hash is a
    /// standard PHC string. The first was written by Havre 0.1.0 (argon2 0.5),
    /// the second by another Argon2 library, with the same parameters.
    #[tokio::test]
    async fn hashes_from_before_still_verify() {
        let havre_0_1 = "$argon2id$v=19$m=19456,t=2,p=1$2z8Igrew5Te42TCi9qs+Jw$2gO2S08eOeQLgSLBTCnPzw2es2QRuhykHrnuCzWB6kI";
        assert!(super::verify_password("offsite-test-password-1", havre_0_1.into()).await.unwrap());
        assert!(!super::verify_password("another password", havre_0_1.into()).await.unwrap());
        let other_library = "$argon2id$v=19$m=19456,t=2,p=1$MXNN/ULbZ67M4S5WiPiRMg$shp2tAbjgIyAr2MyZXzX3JD5hpPbsKVtHPT5jWQGzh0";
        assert!(super::verify_password("correct horse battery staple", other_library.into()).await.unwrap());
        // And a new hash keeps the same shape (Argon2id, same cost).
        let fresh = super::hash_password("correct horse").await.unwrap();
        assert!(fresh.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{fresh}");
    }

    #[tokio::test]
    async fn first_run_then_sign_in_and_out() {
        let auth = Database::in_memory().await.unwrap().auth();
        assert!(!auth.is_set_up().await.unwrap());
        assert!(auth.set_up("short", "test").await.is_err());
        let first = auth.set_up("correct horse", "test").await.unwrap();
        assert!(auth.is_set_up().await.unwrap());
        // A second first-run is refused.
        assert!(auth.set_up("another password", "test").await.is_err());

        assert!(auth.check_session(&first).await.unwrap().is_some());
        assert!(!auth.check_session("forged").await.unwrap().is_some());
        assert!(auth.sign_in("wrong password", "test").await.is_err());
        let second = auth.sign_in("correct horse", "phone").await.unwrap();
        assert_eq!(
            auth.sessions(UserId::ADMIN, &second).await.unwrap().iter().filter(|s| s.current).count(),
            1
        );

        // Changing the password signs out the other browser.
        auth.change_password(UserId::ADMIN, "correct horse", "battery staple", &second).await.unwrap();
        assert!(!auth.check_session(&first).await.unwrap().is_some());
        assert!(auth.check_session(&second).await.unwrap().is_some());
        assert!(auth.sign_in("correct horse", "test").await.is_err());

        auth.sign_out(&second).await.unwrap();
        assert!(!auth.check_session(&second).await.unwrap().is_some());

        auth.reset().await.unwrap();
        assert!(!auth.is_set_up().await.unwrap());
    }
}
