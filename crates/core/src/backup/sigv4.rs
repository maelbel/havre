//! AWS Signature Version 4, for S3-compatible storage (Backblaze B2, MinIO,
//! Scaleway, AWS…).
//!
//! An S3 request proves who sends it without sending the secret key: the
//! request is written out in a fixed ("canonical") form, hashed, and signed
//! with a key derived from the secret key, the day, the region and the
//! service. The server does the same and compares.
//!
//! ```text
//! canonical request  = method, path, query, headers, which headers, payload hash
//! string to sign     = "AWS4-HMAC-SHA256", date, scope, sha256(canonical request)
//! signing key        = HMAC chain: "AWS4"+secret → day → region → "s3" → "aws4_request"
//! signature          = hex(HMAC(signing key, string to sign))
//! ```
//!
//! Tested against the examples in AWS's documentation.
//!
//! What it teaches: canonical forms, HMAC chains, percent-encoding rules.

use hmac::{Hmac, KeyInit, Mac};
use reqwest::Url;
use sha2::{Digest, Sha256};

/// The hash S3 expects for an empty body.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

pub struct Credentials<'a> {
    pub access_key: &'a str,
    pub secret_key: &'a str,
    pub region: &'a str,
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    // `new_from_slice` comes from the `KeyInit` trait (keyed things: MACs, ciphers).
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC takes any key size");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Percent-encodes as AWS wants: letters, digits and `-_.~` stay; so does
/// `/` in a path.
pub fn encode(text: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Signs a request. `headers` must hold every header to sign, `host` and
/// `x-amz-date` included; returns the `Authorization` header's value.
/// `amz_date` is like `20130524T000000Z` (UTC).
pub fn authorization(
    method: &str,
    url: &Url,
    headers: &[(&str, String)],
    payload_hash: &str,
    amz_date: &str,
    creds: &Credentials,
) -> String {
    // 1. The canonical request.
    let path = if url.path().is_empty() { "/" } else { url.path() };
    // The path in `Url` is already encoded; decode it, then encode it AWS's way.
    let path = encode(&percent_decode(path), true);
    let mut query: Vec<(String, String)> =
        url.query_pairs().map(|(k, v)| (encode(&k, false), encode(&v, false))).collect();
    query.sort();
    let query = query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let mut heads: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.split_whitespace().collect::<Vec<_>>().join(" ")))
        .collect();
    heads.sort();
    let canonical_headers: String = heads.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = heads.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let canonical =
        format!("{method}\n{path}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}");

    // 2. The string to sign.
    let day = &amz_date[..8];
    let scope = format!("{day}/{}/s3/aws4_request", creds.region);
    let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", sha256_hex(canonical.as_bytes()));

    // 3. The signing key, then the signature.
    let key = hmac(format!("AWS4{}", creds.secret_key).as_bytes(), day);
    let key = hmac(&key, creds.region);
    let key = hmac(&key, "s3");
    let key = hmac(&key, "aws4_request");
    let signature = hex::encode(hmac(&key, &to_sign));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope},SignedHeaders={signed_headers},Signature={signature}",
        creds.access_key
    )
}

/// `%20` → space, etc. (a path segment from `Url` is percent-encoded).
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The examples from AWS's documentation ("Signature Calculations for the
    // Authorization Header", Amazon S3), with its example keys.
    const CREDS: Credentials = Credentials {
        access_key: "AKIAIOSFODNN7EXAMPLE",
        secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        region: "us-east-1",
    };

    #[test]
    fn signs_aws_get_object_example() {
        let url = Url::parse("https://examplebucket.s3.amazonaws.com/test.txt").unwrap();
        let headers = [
            ("host", "examplebucket.s3.amazonaws.com".to_string()),
            ("range", "bytes=0-9".to_string()),
            ("x-amz-content-sha256", EMPTY_SHA256.to_string()),
            ("x-amz-date", "20130524T000000Z".to_string()),
        ];
        let auth = authorization("GET", &url, &headers, EMPTY_SHA256, "20130524T000000Z", &CREDS);
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,\
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,\
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn signs_aws_list_objects_example() {
        let url = Url::parse("https://examplebucket.s3.amazonaws.com/?max-keys=2&prefix=J").unwrap();
        let headers = [
            ("host", "examplebucket.s3.amazonaws.com".to_string()),
            ("x-amz-content-sha256", EMPTY_SHA256.to_string()),
            ("x-amz-date", "20130524T000000Z".to_string()),
        ];
        let auth = authorization("GET", &url, &headers, EMPTY_SHA256, "20130524T000000Z", &CREDS);
        assert!(
            auth.ends_with("Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"),
            "{auth}"
        );
    }

    #[test]
    fn encodes_like_aws() {
        assert_eq!(encode("havre-2026 11.havre", false), "havre-2026%2011.havre");
        assert_eq!(encode("a/b~c", true), "a/b~c");
        assert_eq!(encode("a/b", false), "a%2Fb");
        assert_eq!(percent_decode("/my%20file"), "/my file");
    }
}
