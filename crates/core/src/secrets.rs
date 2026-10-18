//! Encrypting the mail passwords stored in the database.
//!
//! Passwords are encrypted with XChaCha20-Poly1305 using a 32-byte key that
//! lives *outside* the database:
//!
//! - `HAVRE_SECRET_KEY` (64 hex characters) if set — best for Docker secrets;
//! - otherwise a `secret.key` file next to the database, created on first run.
//!
//! What this protects against: someone getting a copy of `havre.db` alone
//! (a backup, a sync folder…) cannot read your passwords. Someone with access
//! to the whole data folder can — keep it private, or use `HAVRE_SECRET_KEY`.

use std::path::Path;

use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};

use crate::{Error, Result};

/// A nonce is 24 random bytes, stored in front of each ciphertext.
const NONCE_LEN: usize = 24;

#[derive(Clone)]
pub struct Secrets {
    cipher: XChaCha20Poly1305,
    /// The key itself, to show it once as the "recovery key" (Settings ›
    /// Backups): a backup doesn't contain it, on purpose.
    key: std::sync::Arc<[u8; 32]>,
}

impl Secrets {
    /// Loads the key from `HAVRE_SECRET_KEY`, or from `<data_dir>/secret.key`
    /// (creating it the first time).
    pub fn load_or_create(data_dir: &Path) -> Result<Self> {
        if let Ok(hex_key) = std::env::var("HAVRE_SECRET_KEY") {
            return Self::from_hex(hex_key.trim());
        }
        let path = data_dir.join("secret.key");
        match std::fs::read_to_string(&path) {
            Ok(hex_key) => Self::from_hex(hex_key.trim()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir_all(data_dir)?;
                let key = XChaCha20Poly1305::generate_key(&mut OsRng);
                std::fs::write(&path, hex::encode(key))?;
                restrict_permissions(&path);
                tracing::info!("created a new encryption key in {}", path.display());
                Ok(Self::from_bytes(key.into()))
            }
            Err(err) => Err(err.into()),
        }
    }

    /// The key already there (`HAVRE_SECRET_KEY` or `<data_dir>/secret.key`),
    /// without creating one: `None` if there is none.
    pub fn load_existing(data_dir: &Path) -> Result<Option<Self>> {
        if let Ok(hex_key) = std::env::var("HAVRE_SECRET_KEY") {
            return Self::from_hex(hex_key.trim()).map(Some);
        }
        match std::fs::read_to_string(data_dir.join("secret.key")) {
            Ok(hex_key) => Self::from_hex(hex_key.trim()).map(Some),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn from_hex(hex_key: &str) -> Result<Self> {
        let bytes = hex::decode(hex_key)
            .ok()
            .filter(|b| b.len() == 32)
            .ok_or_else(|| Error::Config("the secret key must be 64 hexadecimal characters".into()))?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| Error::Config("invalid secret key".into()))?;
        Ok(Self::from_bytes(key))
    }

    fn from_bytes(key: [u8; 32]) -> Self {
        Self { cipher: XChaCha20Poly1305::new(&key.into()), key: std::sync::Arc::new(key) }
    }

    /// A random key, for tests.
    pub fn random() -> Self {
        Self::from_bytes(XChaCha20Poly1305::generate_key(&mut OsRng).into())
    }

    /// The key as 64 hex characters (what `secret.key` / `HAVRE_SECRET_KEY` hold).
    pub fn export_hex(&self) -> String {
        hex::encode(*self.key)
    }

    /// A short, harmless name for the key ("which key does this backup
    /// need?"): the start of its SHA-256, which doesn't reveal the key.
    pub fn fingerprint(&self) -> String {
        fingerprint_of(self.key.as_slice())
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>> {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| Error::Config("encryption failed".into()))?;
        Ok([nonce.as_slice(), &ciphertext].concat())
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<String> {
        if data.len() < NONCE_LEN {
            return Err(Error::Config("stored password is corrupted".into()));
        }
        let (nonce, ciphertext) = data.split_at(NONCE_LEN);
        let plaintext = self.cipher.decrypt(XNonce::from_slice(nonce), ciphertext).map_err(|_| {
            Error::Config(
                "cannot decrypt a stored password: the secret key changed? Re-enter the password in Settings"
                    .into(),
            )
        })?;
        String::from_utf8(plaintext).map_err(|_| Error::Config("stored password is corrupted".into()))
    }
}

pub(crate) fn fingerprint_of(key: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(&Sha256::digest(key)[..6])
}

/// Only the owner may read the key file (Unix). On Windows, files in your
/// user folder are already private to you.
pub(crate) fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_wrong_key() {
        let secrets = Secrets::random();
        let sealed = secrets.encrypt("hunter2").unwrap();
        assert_ne!(&sealed[NONCE_LEN..], b"hunter2");
        assert_eq!(secrets.decrypt(&sealed).unwrap(), "hunter2");
        // Same text twice → different ciphertexts (random nonce).
        assert_ne!(sealed, secrets.encrypt("hunter2").unwrap());
        assert!(Secrets::random().decrypt(&sealed).is_err());
    }

    #[test]
    fn key_file_is_created_then_reused() {
        let dir = std::env::temp_dir().join(format!("havre-secrets-{}", std::process::id()));
        let first = Secrets::load_or_create(&dir).unwrap();
        let sealed = first.encrypt("pw").unwrap();
        let again = Secrets::load_or_create(&dir).unwrap();
        assert_eq!(again.decrypt(&sealed).unwrap(), "pw");
        let _ = std::fs::remove_dir_all(dir);
    }
}
