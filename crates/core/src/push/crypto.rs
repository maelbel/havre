//! The two pieces of Web Push that are cryptography, each from its RFC.
//!
//! **RFC 8291, encrypting the message.** The browser gave us its public key
//! (`p256dh`) and a shared secret (`auth`) when it subscribed. For each
//! message we make a throwaway key pair, agree on a secret with the browser's
//! key (ECDH on the P-256 curve: both sides compute the same point without
//! sending it), stretch it into a key and a nonce with HKDF, and encrypt with
//! AES-128-GCM. The push service (Google, Apple, Mozilla) only ever carries
//! ciphertext.
//!
//! ```text
//! ecdh     = ECDH(our throwaway private key, browser's public key)
//! ikm      = HKDF(salt: auth, secret: ecdh, info: "WebPush: info\0" | browser key | our key)
//! cek      = HKDF(salt: random 16 bytes, ikm, "Content-Encoding: aes128gcm\0")  → 16 bytes
//! nonce    = HKDF(same salt, ikm, "Content-Encoding: nonce\0")                  → 12 bytes
//! body     = salt | record size | key length | our public key | AES-GCM(text | 0x02)
//! ```
//!
//! **RFC 8292 (VAPID), saying who sends it.** Each request carries a small
//! JWT signed with Havre's own long-lived P-256 key (ES256). The push service
//! checks it against the public key the browser subscribed with, so nobody
//! else can push to your phone even if they learn the address.
//!
//! What it teaches: elliptic-curve Diffie–Hellman, HKDF (built from HMAC),
//! authenticated encryption, ECDSA signatures, JWTs — and testing against the
//! RFC's own example.

use aes_gcm::{Aes128Gcm, KeyInit as _, aead::Aead};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use hmac::{Hmac, Mac};
use p256::{
    PublicKey, SecretKey,
    ecdsa::{Signature, SigningKey, signature::Signer},
};
use sha2::Sha256;

use crate::{Error, Result};

/// One record holds the whole message (pushes are at most 4 KiB anyway).
const RECORD_SIZE: u32 = 4096;
/// What a push service accepts at most, encryption overhead included.
pub const MAX_TEXT: usize = 3000;

/// What the browser gives when it subscribes (`PushSubscription.toJSON()`).
#[derive(Debug, Clone)]
pub struct BrowserKeys {
    /// The browser's P-256 public key, 65 bytes (uncompressed point).
    pub p256dh: Vec<u8>,
    /// A 16-byte secret shared with the browser.
    pub auth: Vec<u8>,
}

impl BrowserKeys {
    /// From the base64url strings the browser sends.
    pub fn from_base64(p256dh: &str, auth: &str) -> Result<Self> {
        let p256dh = B64.decode(p256dh.trim_end_matches('=')).map_err(|_| bad("p256dh"))?;
        let auth = B64.decode(auth.trim_end_matches('=')).map_err(|_| bad("auth"))?;
        PublicKey::from_sec1_bytes(&p256dh).map_err(|_| bad("p256dh"))?;
        if auth.len() != 16 {
            return Err(bad("auth"));
        }
        Ok(Self { p256dh, auth })
    }
}

fn bad(what: &str) -> Error {
    Error::Config(format!("This browser sent an unreadable push subscription ({what})."))
}

/// HMAC-SHA-256 of `parts` (one after the other) with `key`.
fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    // `new_from_slice` comes from `KeyInit`: HMAC takes a key of any length.
    let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(key).expect("HMAC takes any key size");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

/// HKDF (RFC 5869) when one block of output is enough, as here: "extract"
/// mixes `ikm` with `salt` into a pseudo-random key, "expand" derives a key
/// for a purpose (`info`) from it. That's all HKDF is: two HMACs.
fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8]) -> [u8; 32] {
    let prk = hmac(salt, &[ikm]);
    hmac(&prk, &[info, &[1]])
}

/// Encrypts `text` for one browser (RFC 8291, `aes128gcm`). `ours` is a
/// throwaway key for this message and `salt` 16 random bytes: both are
/// parameters so the RFC's example can be checked; [`seal`] picks them.
pub fn seal_with(text: &[u8], to: &BrowserKeys, ours: &SecretKey, salt: [u8; 16]) -> Result<Vec<u8>> {
    let theirs = PublicKey::from_sec1_bytes(&to.p256dh).map_err(|_| bad("p256dh"))?;
    let our_public = ours.public_key().to_sec1_bytes(); // 65 bytes: 0x04, x, y
    let ecdh = ours.diffie_hellman(&theirs);

    let key_info = [b"WebPush: info\0".as_slice(), &to.p256dh, &our_public].concat();
    let ikm = hkdf(&to.auth, ecdh.raw_secret_bytes(), &key_info);
    let cek = hkdf(&salt, &ikm, b"Content-Encoding: aes128gcm\0");
    let nonce = hkdf(&salt, &ikm, b"Content-Encoding: nonce\0");

    // The text, then 0x02: "this is the last record" (and no padding).
    let mut record = text.to_vec();
    record.push(2);
    let cipher = Aes128Gcm::new_from_slice(&cek[..16]).expect("16 bytes");
    let nonce: [u8; 12] = nonce[..12].try_into().expect("12 bytes");
    let sealed = cipher
        .encrypt(&nonce.into(), record.as_slice())
        .map_err(|_| Error::Config("couldn't encrypt the notification".into()))?;

    // The header tells the browser how to decrypt: salt, record size, our key.
    let mut body = Vec::with_capacity(16 + 4 + 1 + our_public.len() + sealed.len());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(our_public.len() as u8);
    body.extend_from_slice(&our_public);
    body.extend_from_slice(&sealed);
    Ok(body)
}

/// Encrypts `text` for one browser, with a fresh key and salt.
pub fn seal(text: &[u8], to: &BrowserKeys) -> Result<Vec<u8>> {
    if text.len() > MAX_TEXT {
        return Err(Error::Config("a notification is limited to 3000 bytes".into()));
    }
    seal_with(text, to, &random_key(), random_bytes())
}

/// 16 random bytes from the system's generator.
fn random_bytes() -> [u8; 16] {
    use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

/// A new random P-256 private key. Made from 32 random bytes (a number below
/// the curve's order: the loop retries in the 1-in-2³² case it isn't).
pub fn random_key() -> SecretKey {
    use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
    loop {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        if let Ok(key) = SecretKey::from_slice(&bytes) {
            return key;
        }
    }
}

/// Havre's long-lived key for VAPID.
pub struct Vapid {
    key: SigningKey,
    /// Who to contact about this server's pushes (`mailto:` or `https:`).
    contact: String,
}

impl Vapid {
    pub fn new(key: &SecretKey, contact: &str) -> Self {
        Self { key: SigningKey::from(key), contact: contact.to_owned() }
    }

    /// The public key, base64url, as the browser's `subscribe()` wants it
    /// (`applicationServerKey`).
    pub fn public_key(&self) -> String {
        B64.encode(self.key.verifying_key().to_sec1_point(false).as_bytes())
    }

    /// The `Authorization` header for a push to `endpoint`, valid 12 hours.
    pub fn authorization(&self, endpoint: &str, now: i64) -> Result<String> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| Error::Config("bad push address".into()))?;
        // The JWT's audience is the push service's origin, e.g. https://fcm.googleapis.com.
        let audience = url.origin().ascii_serialization();
        let header = B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = serde_json::json!({ "aud": audience, "exp": now + 12 * 3600, "sub": self.contact });
        let claims = B64.encode(claims.to_string());
        let signed = format!("{header}.{claims}");
        // ES256: ECDSA with SHA-256 over "header.claims", as 64 bytes (r | s).
        let signature: Signature = self.key.sign(signed.as_bytes());
        Ok(format!("vapid t={signed}.{}, k={}", B64.encode(signature.to_bytes()), self.public_key()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn b64(s: &str) -> Vec<u8> {
        B64.decode(s).unwrap()
    }

    // RFC 8291, Appendix A: every value of the example.
    const PLAINTEXT: &str = "When I grow up, I want to be a watermelon";
    const AS_PRIVATE: &str = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";
    pub(crate) const UA_PRIVATE: &str = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94";
    pub(crate) const UA_PUBLIC: &str =
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
    pub(crate) const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
    const SALT: &str = "DGv6ra1nlYgDCS1FRnbzlw";
    const BODY: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";

    #[test]
    fn encrypts_the_rfc_example() {
        let to = BrowserKeys::from_base64(UA_PUBLIC, AUTH).unwrap();
        let ours = SecretKey::from_slice(&b64(AS_PRIVATE)).unwrap();
        let salt: [u8; 16] = b64(SALT).try_into().unwrap();
        let body = seal_with(PLAINTEXT.as_bytes(), &to, &ours, salt).unwrap();
        assert_eq!(B64.encode(body), BODY);
    }

    /// What the browser does: the other side of the key agreement.
    pub(crate) fn open(body: &[u8], ua_private: &SecretKey, auth: &[u8]) -> Vec<u8> {
        let (salt, rest) = body.split_at(16);
        let key_len = rest[4] as usize;
        let (their_public, sealed) = rest[5..].split_at(key_len);
        let ua_public = ua_private.public_key().to_sec1_bytes();
        let ecdh = ua_private.diffie_hellman(&PublicKey::from_sec1_bytes(their_public).unwrap());
        let key_info = [b"WebPush: info\0".as_slice(), &ua_public, their_public].concat();
        let ikm = hkdf(auth, ecdh.raw_secret_bytes(), &key_info);
        let cek = hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0");
        let nonce: [u8; 12] = hkdf(salt, &ikm, b"Content-Encoding: nonce\0")[..12].try_into().unwrap();
        let mut text = Aes128Gcm::new_from_slice(&cek[..16]).unwrap().decrypt(&nonce.into(), sealed).unwrap();
        assert_eq!(text.pop(), Some(2), "the last-record mark");
        text
    }

    #[test]
    fn a_browser_can_read_what_we_seal() {
        let ua_private = SecretKey::from_slice(&b64(UA_PRIVATE)).unwrap();
        let to = BrowserKeys::from_base64(UA_PUBLIC, AUTH).unwrap();
        let text = r#"{"title":"Claire Dubois","body":"Kick-off — the planning"}"#;
        let first = seal(text.as_bytes(), &to).unwrap();
        let second = seal(text.as_bytes(), &to).unwrap();
        assert_ne!(first, second, "a new key and salt each time");
        assert_eq!(open(&first, &ua_private, &b64(AUTH)), text.as_bytes());
    }

    #[test]
    fn refuses_bad_subscriptions_and_long_texts() {
        assert!(BrowserKeys::from_base64("not-a-key", AUTH).is_err());
        assert!(BrowserKeys::from_base64(UA_PUBLIC, "c2hvcnQ").is_err(), "auth is 16 bytes");
        let to = BrowserKeys::from_base64(UA_PUBLIC, AUTH).unwrap();
        assert!(seal(&[b'x'; MAX_TEXT + 1], &to).is_err());
    }

    #[test]
    fn vapid_header_verifies() {
        use p256::ecdsa::{VerifyingKey, signature::Verifier};
        let key = SecretKey::from_slice(&b64(AS_PRIVATE)).unwrap();
        let vapid = Vapid::new(&key, "mailto:admin@example.org");
        let header =
            vapid.authorization("https://fcm.googleapis.com/fcm/send/abc:123", 1_800_000_000).unwrap();
        let (t, k) = header.strip_prefix("vapid t=").unwrap().split_once(", k=").unwrap();
        assert_eq!(k, vapid.public_key());
        assert_eq!(b64(k).len(), 65);
        let (signed, signature) = t.rsplit_once('.').unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&b64(signed.split('.').nth(1).unwrap())).unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["exp"], 1_800_000_000 + 12 * 3600);
        assert_eq!(claims["sub"], "mailto:admin@example.org");
        let verifying = VerifyingKey::from_sec1_bytes(&b64(k)).unwrap();
        let signature = Signature::from_slice(&b64(signature)).unwrap();
        assert!(verifying.verify(signed.as_bytes(), &signature).is_ok());
    }
}
