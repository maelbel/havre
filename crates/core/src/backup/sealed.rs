//! Sealed backups: a backup file encrypted for leaving the server.
//!
//! An off-site copy sits on someone else's disk (a Nextcloud, a bucket), so it
//! must be unreadable there, and Havre must notice if it was changed or cut
//! short. The file is encrypted with **XChaCha20-Poly1305 in STREAM mode**:
//!
//! - it is cut in chunks of 64 KiB, each encrypted and authenticated on its
//!   own, so memory stays small whatever the size of the backup;
//! - each chunk's nonce holds a counter, and the last one is marked as last:
//!   a chunk changed, swapped, dropped or cut off makes decryption fail,
//!   instead of restoring half a database;
//! - the header (in clear) is authenticated with every chunk, so it can't be
//!   changed either.
//!
//! ```text
//! HAVREBK1 | key fingerprint (12) | made at (8, big-endian) | nonce prefix (19) | chunks…
//! ```
//!
//! The key is derived from the recovery key (`Secrets::derive`), so whoever
//! can restore a normal backup can open a sealed one, and nobody else.
//!
//! What it teaches: authenticated encryption on a stream, why chunks need a
//! counter and a "last" flag, reading a file in fixed-size pieces.

use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};

use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305,
    aead::{
        OsRng, Payload,
        rand_core::RngCore,
        stream::{DecryptorBE32, EncryptorBE32},
    },
};

use crate::{Error, Result, secrets::Secrets};

/// The first bytes of every sealed backup (the format's name and version).
pub const MAGIC: &[u8; 8] = b"HAVREBK1";
/// File extension of sealed backups.
pub const EXTENSION: &str = "havre";
const LABEL: &str = "havre off-site backup v1";
const FINGERPRINT_LEN: usize = 12;
/// XChaCha20's nonce is 24 bytes; STREAM keeps 5 for its counter and flag.
const PREFIX_LEN: usize = 19;
const HEADER_LEN: usize = MAGIC.len() + FINGERPRINT_LEN + 8 + PREFIX_LEN;
/// Plaintext per chunk. Each encrypted chunk is 16 bytes longer (its tag).
const CHUNK: usize = 64 * 1024;
const TAG: usize = 16;

/// What a sealed file's header says.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// Which recovery key opens it (the key's fingerprint, not the key).
    pub key_fingerprint: String,
    /// When it was sealed (unix seconds).
    pub created_at: i64,
}

/// Is this file a sealed backup? (Reads its first 8 bytes.)
pub fn is_sealed(path: &Path) -> bool {
    let mut magic = [0u8; 8];
    File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && &magic == MAGIC
}

/// Reads the header without decrypting anything.
pub fn read_header(path: &Path) -> Result<Header> {
    let mut bytes = [0u8; HEADER_LEN];
    File::open(path)?
        .read_exact(&mut bytes)
        .map_err(|_| Error::Config("This file is too short to be a sealed Havre backup.".into()))?;
    parse_header(&bytes).map(|(header, _)| header)
}

fn parse_header(bytes: &[u8; HEADER_LEN]) -> Result<(Header, [u8; PREFIX_LEN])> {
    if &bytes[..8] != MAGIC {
        return Err(Error::Config("This file isn't a sealed Havre backup.".into()));
    }
    let fingerprint = std::str::from_utf8(&bytes[8..8 + FINGERPRINT_LEN])
        .map_err(|_| Error::Config("This sealed backup's header is damaged.".into()))?;
    let at = 8 + FINGERPRINT_LEN;
    let created_at = i64::from_be_bytes(bytes[at..at + 8].try_into().expect("8 bytes"));
    let prefix: [u8; PREFIX_LEN] = bytes[at + 8..].try_into().expect("19 bytes");
    Ok((Header { key_fingerprint: fingerprint.to_string(), created_at }, prefix))
}

fn cipher(secrets: &Secrets) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(&secrets.derive(LABEL).into())
}

/// Encrypts `src` into `dst` (written as `dst.partial`, then renamed).
pub fn seal(src: &Path, dst: &Path, secrets: &Secrets, created_at: i64) -> Result<()> {
    let partial = dst.with_extension(format!("{EXTENSION}.partial"));
    let result = (|| {
        let mut prefix = [0u8; PREFIX_LEN];
        OsRng.fill_bytes(&mut prefix);
        let fingerprint = secrets.fingerprint();
        debug_assert_eq!(fingerprint.len(), FINGERPRINT_LEN);
        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(fingerprint.as_bytes());
        header.extend_from_slice(&created_at.to_be_bytes());
        header.extend_from_slice(&prefix);

        let mut out = BufWriter::new(File::create(&partial)?);
        out.write_all(&header)?;
        let mut input = BufReader::new(File::open(src)?);
        let mut encryptor = EncryptorBE32::from_aead(cipher(secrets), prefix.as_slice().into());
        let sealing = |_| Error::Config("Couldn't encrypt the backup.".into());

        // Read one chunk ahead: a chunk is the last one only when nothing follows.
        let mut current = read_chunk(&mut input, CHUNK)?;
        loop {
            let next = read_chunk(&mut input, CHUNK)?;
            if next.is_empty() {
                let last =
                    encryptor.encrypt_last(Payload { msg: &current, aad: &header }).map_err(sealing)?;
                out.write_all(&last)?;
                break;
            }
            let chunk = encryptor.encrypt_next(Payload { msg: &current, aad: &header }).map_err(sealing)?;
            out.write_all(&chunk)?;
            current = next;
        }
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            crate::secrets::restrict_permissions(&partial);
            std::fs::rename(&partial, dst)?;
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::remove_file(&partial);
            Err(err)
        }
    }
}

/// Decrypts `src` into `dst`. Fails, leaving no `dst`, if the key is wrong
/// or the file was changed or cut short.
pub fn unseal(src: &Path, dst: &Path, secrets: &Secrets) -> Result<Header> {
    let mut input = BufReader::new(File::open(src)?);
    let mut header = [0u8; HEADER_LEN];
    input
        .read_exact(&mut header)
        .map_err(|_| Error::Config("This file is too short to be a sealed Havre backup.".into()))?;
    let (info, prefix) = parse_header(&header)?;
    if info.key_fingerprint != secrets.fingerprint() {
        return Err(Error::Config(format!(
            "This copy was sealed with another recovery key (fingerprint {}, this one is {}).",
            info.key_fingerprint,
            secrets.fingerprint()
        )));
    }
    let result = (|| {
        let mut out = BufWriter::new(File::create(dst)?);
        let mut decryptor = DecryptorBE32::from_aead(cipher(secrets), prefix.as_slice().into());
        let broken = |_| {
            Error::Config("This copy is damaged or incomplete (it doesn't decrypt): use another one.".into())
        };
        let mut current = read_chunk(&mut input, CHUNK + TAG)?;
        if current.is_empty() {
            return Err(Error::Config("This copy is empty: use another one.".into()));
        }
        loop {
            let next = read_chunk(&mut input, CHUNK + TAG)?;
            if next.is_empty() {
                let last = decryptor.decrypt_last(Payload { msg: &current, aad: &header }).map_err(broken)?;
                out.write_all(&last)?;
                break;
            }
            let chunk = decryptor.decrypt_next(Payload { msg: &current, aad: &header }).map_err(broken)?;
            out.write_all(&chunk)?;
            current = next;
        }
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            crate::secrets::restrict_permissions(dst);
            Ok(info)
        }
        Err(err) => {
            let _ = std::fs::remove_file(dst);
            Err(err)
        }
    }
}

/// Reads up to `size` bytes (fewer only at the end of the file).
fn read_chunk(input: &mut impl Read, size: usize) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(size);
    input.take(size as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("havre-sealed-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Seals `data`, unseals it, and checks it came back the same.
    fn round_trip(name: &str, data: &[u8]) {
        let dir = temp_dir(name);
        let (plain, sealed, back) = (dir.join("a.db"), dir.join("a.havre"), dir.join("b.db"));
        std::fs::write(&plain, data).unwrap();
        let secrets = Secrets::random();
        seal(&plain, &sealed, &secrets, 1_794_000_000).unwrap();
        assert!(is_sealed(&sealed));
        assert!(!dir.join("a.havre.partial").exists());
        let header = read_header(&sealed).unwrap();
        assert_eq!(header, Header { key_fingerprint: secrets.fingerprint(), created_at: 1_794_000_000 });
        assert_eq!(unseal(&sealed, &back, &secrets).unwrap(), header);
        assert_eq!(std::fs::read(&back).unwrap(), data);
        // Encrypted: the content isn't in the sealed file.
        if data.len() > 32 {
            let sealed_bytes = std::fs::read(&sealed).unwrap();
            assert!(!sealed_bytes.windows(32).any(|w| w == &data[..32]));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn seals_and_unseals_files_of_any_size() {
        round_trip("empty", b"");
        round_trip("small", b"SQLite format 3\0 and a few bytes");
        round_trip("one-chunk", &vec![7u8; CHUNK]);
        let many: Vec<u8> = (0..(3 * CHUNK + 1234)).map(|i| (i * 31 % 251) as u8).collect();
        round_trip("many", &many);
    }

    #[test]
    fn refuses_a_changed_or_cut_copy_and_the_wrong_key() {
        let dir = temp_dir("tamper");
        let (plain, sealed, back) = (dir.join("a.db"), dir.join("a.havre"), dir.join("b.db"));
        let data: Vec<u8> = (0..(2 * CHUNK + 10)).map(|i| (i % 256) as u8).collect();
        std::fs::write(&plain, &data).unwrap();
        let secrets = Secrets::random();
        seal(&plain, &sealed, &secrets, 0).unwrap();
        let good = std::fs::read(&sealed).unwrap();

        // One byte flipped in the middle.
        let mut flipped = good.clone();
        flipped[HEADER_LEN + CHUNK] ^= 1;
        std::fs::write(&sealed, &flipped).unwrap();
        assert!(unseal(&sealed, &back, &secrets).is_err());
        assert!(!back.exists(), "nothing half-written is left");

        // The last chunk missing: the one before isn't marked as last.
        std::fs::write(&sealed, &good[..HEADER_LEN + 2 * (CHUNK + TAG)]).unwrap();
        assert!(unseal(&sealed, &back, &secrets).is_err());

        // The header changed (it's authenticated with every chunk).
        let mut header_changed = good.clone();
        header_changed[8 + FINGERPRINT_LEN + 7] ^= 1; // the date
        std::fs::write(&sealed, &header_changed).unwrap();
        assert!(unseal(&sealed, &back, &secrets).is_err());

        // Another key.
        std::fs::write(&sealed, &good).unwrap();
        let error = unseal(&sealed, &back, &Secrets::random()).unwrap_err().to_string();
        assert!(error.contains("another recovery key"), "{error}");
        assert!(unseal(&sealed, &back, &secrets).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sealing_key_is_not_the_recovery_key() {
        let secrets = Secrets::random();
        let derived = secrets.derive(LABEL);
        assert_ne!(hex::encode(derived), secrets.export_hex());
        assert_eq!(derived, secrets.derive(LABEL), "the same key every time");
        assert_ne!(derived, secrets.derive("another purpose"));
    }
}
