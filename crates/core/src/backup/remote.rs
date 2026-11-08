//! Where off-site copies go: a WebDAV folder or an S3-compatible bucket.
//!
//! Four things are needed from either: try the connection, send a file, list
//! the copies Havre made, delete one. Only names Havre writes
//! (`havre-….havre`, see `is_copy_name`) are ever listed or deleted: other
//! files in the folder or bucket are left alone.
//!
//! - **WebDAV** (Nextcloud, a NAS): `PUT` to a temporary name, then `MOVE` to
//!   the real one, so a half-sent file never looks like a copy; `PROPFIND`
//!   lists; basic auth (for Nextcloud, an *app password*).
//! - **S3** (Backblaze B2, MinIO, Scaleway…): `PUT`, `HEAD`,
//!   `ListObjectsV2`, `DELETE`, each signed with AWS Signature V4
//!   (`sigv4.rs`).
//!
//! Addresses must be `https://`, except on the same machine (a MinIO next to
//! Havre): passwords and keys never cross a network in clear.
//!
//! What it teaches: HTTP methods beyond GET/POST, reading XML, an `enum` with
//! data, building requests by hand.

use reqwest::{Client, Method, StatusCode, Url};
use serde::{Deserialize, Serialize};

use super::sigv4;
use crate::{Error, Result};

/// The two kinds of destination.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Webdav,
    S3,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Webdav => "webdav",
            Kind::S3 => "s3",
        }
    }
    pub fn parse(text: &str) -> Kind {
        if text == "s3" { Kind::S3 } else { Kind::Webdav }
    }
}

/// A destination, with its secret (password or S3 secret key) in clear: this
/// only lives in memory while a copy is being sent.
#[derive(Clone)]
pub struct Remote {
    pub kind: Kind,
    /// WebDAV: the folder's address. S3: the endpoint (`https://s3.eu-central-003.backblazeb2.com`).
    pub url: Url,
    pub bucket: String,
    pub region: String,
    /// S3: a folder inside the bucket (`havre/`), may be empty.
    pub prefix: String,
    /// S3: `endpoint/bucket/key` (MinIO, B2) rather than `bucket.endpoint/key`.
    pub path_style: bool,
    /// WebDAV user, or S3 access key.
    pub username: String,
    pub secret: String,
}

/// A copy found at the destination.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RemoteFile {
    pub name: String,
    pub size: u64,
}

/// Names Havre writes off-site, and the only ones it lists or deletes.
pub fn is_copy_name(name: &str) -> bool {
    name.starts_with("havre-")
        && name.ends_with(".havre")
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !name.contains("..")
}

/// Checks an address typed in Settings: a valid URL, and `https://` unless it
/// points to this machine.
pub fn check_url(text: &str) -> Result<Url> {
    let url = Url::parse(text.trim()).map_err(|_| {
        Error::Config("The address looks like https://cloud.example.org/remote.php/dav/files/me/Havre".into())
    })?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]" | "::1"));
    match url.scheme() {
        "https" => {}
        "http" if local => {}
        "http" => {
            return Err(Error::Config(
                "Use an https:// address: over plain http the password and the copies would cross the network \
                 in clear (http is only accepted on this machine, like http://localhost:9000)."
                    .into(),
            ));
        }
        _ => return Err(Error::Config("The address must start with https://.".into())),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::Config(
            "Put the user name and password in their own fields, not in the address.".into(),
        ));
    }
    Ok(url)
}

pub fn client() -> Result<Client> {
    Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .read_timeout(std::time::Duration::from_secs(120))
        .user_agent(concat!("Havre/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::Config(format!("Couldn't prepare the connection: {e}")))
}

impl Remote {
    /// Tries what sending copies needs: write a small file, see it listed,
    /// delete it. Returns a sentence for Settings.
    pub async fn test(&self, http: &Client) -> Result<String> {
        let name = format!("havre-test-{}.havre", chrono::Utc::now().format("%Y%m%d%H%M%S"));
        self.ensure_folder(http).await?;
        self.put(http, &name, b"Havre connection test, safe to delete.".to_vec()).await?;
        let listed = self.list(http).await?.iter().any(|f| f.name == name);
        self.delete(http, &name).await?;
        if !listed {
            return Err(Error::Config(
                "A test file was written but didn't show up in the list: check the folder or prefix.".into(),
            ));
        }
        Ok(match self.kind {
            Kind::Webdav => "Connected: Havre can write, list and delete in this folder.".into(),
            Kind::S3 => "Connected: Havre can write, list and delete in this bucket.".into(),
        })
    }

    /// Sends one copy and checks its size at the destination.
    pub async fn send(&self, http: &Client, name: &str, bytes: Vec<u8>) -> Result<()> {
        let size = bytes.len() as u64;
        self.ensure_folder(http).await?;
        self.put(http, name, bytes).await?;
        match self.list(http).await?.into_iter().find(|f| f.name == name) {
            Some(f) if f.size == size => Ok(()),
            Some(f) => Err(Error::Config(format!(
                "The copy arrived incomplete ({} of {size} bytes); it will be sent again.",
                f.size
            ))),
            None => Err(Error::Config("The copy was sent but isn't listed at the destination.".into())),
        }
    }

    // ---- WebDAV and S3 below -------------------------------------------------------

    fn webdav_url(&self, name: &str) -> Url {
        let mut url = self.url.clone();
        if !url.path().ends_with('/') {
            url.set_path(&format!("{}/", url.path()));
        }
        url.join(name).expect("a plain file name")
    }

    fn s3_url(&self, key: &str, query: &str) -> Url {
        let mut url = self.url.clone();
        let key = sigv4::encode(key, true);
        if self.path_style {
            url.set_path(&format!("/{}/{key}", self.bucket));
        } else {
            let host = format!("{}.{}", self.bucket, url.host_str().unwrap_or_default());
            let _ = url.set_host(Some(&host));
            url.set_path(&format!("/{key}"));
        }
        url.set_query(if query.is_empty() { None } else { Some(query) });
        url
    }

    fn s3_key(&self, name: &str) -> String {
        let prefix = self.prefix.trim_matches('/');
        if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") }
    }

    /// A signed S3 request.
    async fn s3(&self, http: &Client, method: Method, url: Url, body: Vec<u8>) -> Result<reqwest::Response> {
        let payload =
            if body.is_empty() { sigv4::EMPTY_SHA256.to_string() } else { sigv4::sha256_hex(&body) };
        let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let host = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
            None => url.host_str().unwrap_or_default().to_string(),
        };
        let headers =
            [("host", host), ("x-amz-content-sha256", payload.clone()), ("x-amz-date", amz_date.clone())];
        let creds = sigv4::Credentials {
            access_key: &self.username,
            secret_key: &self.secret,
            region: if self.region.is_empty() { "us-east-1" } else { &self.region },
        };
        let auth = sigv4::authorization(method.as_str(), &url, &headers, &payload, &amz_date, &creds);
        http.request(method, url)
            .header("x-amz-content-sha256", payload)
            .header("x-amz-date", amz_date)
            .header("authorization", auth)
            .body(body)
            .send()
            .await
            .map_err(network_error)
    }

    /// WebDAV: makes the folder if it's missing. S3: the bucket must exist.
    async fn ensure_folder(&self, http: &Client) -> Result<()> {
        if self.kind == Kind::S3 {
            return Ok(());
        }
        let res = self.dav(http, "PROPFIND", self.webdav_url(""), Some("0"), Vec::new()).await?;
        match res.status() {
            s if s.is_success() => Ok(()),
            StatusCode::NOT_FOUND => {
                let made = self.dav(http, "MKCOL", self.webdav_url(""), None, Vec::new()).await?;
                if made.status().is_success() {
                    Ok(())
                } else {
                    Err(http_error(made.status(), self.kind, "Couldn't create the folder"))
                }
            }
            s => Err(http_error(s, self.kind, "Couldn't open the folder")),
        }
    }

    async fn dav(
        &self,
        http: &Client,
        method: &str,
        url: Url,
        depth: Option<&str>,
        body: Vec<u8>,
    ) -> Result<reqwest::Response> {
        let method = Method::from_bytes(method.as_bytes()).expect("a valid method");
        let mut req = http.request(method, url).basic_auth(&self.username, Some(&self.secret));
        if let Some(depth) = depth {
            req = req.header("Depth", depth).header("Content-Type", "application/xml");
        }
        req.body(body).send().await.map_err(network_error)
    }

    async fn put(&self, http: &Client, name: &str, bytes: Vec<u8>) -> Result<()> {
        match self.kind {
            Kind::S3 => {
                let res = self.s3(http, Method::PUT, self.s3_url(&self.s3_key(name), ""), bytes).await?;
                if !res.status().is_success() {
                    return Err(http_error(res.status(), self.kind, "The copy wasn't accepted"));
                }
            }
            Kind::Webdav => {
                // A temporary name first: a half-sent file never looks like a copy.
                let partial = format!("{name}.partial");
                let res = self.dav(http, "PUT", self.webdav_url(&partial), None, bytes).await?;
                if !res.status().is_success() {
                    return Err(http_error(res.status(), self.kind, "The copy wasn't accepted"));
                }
                let res = http
                    .request(Method::from_bytes(b"MOVE").expect("valid"), self.webdav_url(&partial))
                    .basic_auth(&self.username, Some(&self.secret))
                    .header("Destination", self.webdav_url(name).as_str())
                    .header("Overwrite", "T")
                    .send()
                    .await
                    .map_err(network_error)?;
                if !res.status().is_success() {
                    return Err(http_error(res.status(), self.kind, "The copy couldn't be renamed"));
                }
            }
        }
        Ok(())
    }

    /// The copies Havre made (and only those), oldest name first.
    pub async fn list(&self, http: &Client) -> Result<Vec<RemoteFile>> {
        let mut files = match self.kind {
            Kind::Webdav => {
                let body = br#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:getcontentlength/></d:prop></d:propfind>"#;
                let res = self.dav(http, "PROPFIND", self.webdav_url(""), Some("1"), body.to_vec()).await?;
                if !res.status().is_success() {
                    return Err(http_error(res.status(), self.kind, "Couldn't list the folder"));
                }
                parse_propfind(&res.text().await.map_err(network_error)?)?
            }
            Kind::S3 => {
                let prefix = self.s3_key("havre-");
                let query = format!("list-type=2&prefix={}", sigv4::encode(&prefix, false));
                let res = self.s3(http, Method::GET, self.s3_url("", &query), Vec::new()).await?;
                if !res.status().is_success() {
                    return Err(http_error(res.status(), self.kind, "Couldn't list the bucket"));
                }
                parse_list_objects(&res.text().await.map_err(network_error)?)?
            }
        };
        files.retain(|f| is_copy_name(&f.name));
        files.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(files)
    }

    pub async fn delete(&self, http: &Client, name: &str) -> Result<()> {
        if !is_copy_name(name) {
            return Err(Error::Config("Havre only deletes the copies it made.".into()));
        }
        let res = match self.kind {
            Kind::Webdav => self.dav(http, "DELETE", self.webdav_url(name), None, Vec::new()).await?,
            Kind::S3 => {
                self.s3(http, Method::DELETE, self.s3_url(&self.s3_key(name), ""), Vec::new()).await?
            }
        };
        // Already gone counts as deleted.
        if res.status().is_success() || res.status() == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(http_error(res.status(), self.kind, "Couldn't delete an old copy"))
        }
    }
}

/// The file names and sizes in a WebDAV `PROPFIND` answer.
fn parse_propfind(xml: &str) -> Result<Vec<RemoteFile>> {
    let doc = roxmltree::Document::parse(xml).map_err(|_| {
        Error::Config("The server's answer isn't WebDAV: is the address a WebDAV folder?".into())
    })?;
    let mut files = Vec::new();
    for response in doc.descendants().filter(|n| n.tag_name().name() == "response") {
        let href = response.descendants().find(|n| n.tag_name().name() == "href").and_then(|n| n.text());
        let size = response
            .descendants()
            .find(|n| n.tag_name().name() == "getcontentlength")
            .and_then(|n| n.text())
            .and_then(|t| t.trim().parse().ok());
        if let (Some(href), Some(size)) = (href, size) {
            let last = href.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
            let name = percent_decode(last);
            files.push(RemoteFile { name, size });
        }
    }
    Ok(files)
}

/// The keys and sizes in an S3 `ListObjectsV2` answer (names without the prefix).
fn parse_list_objects(xml: &str) -> Result<Vec<RemoteFile>> {
    let doc =
        roxmltree::Document::parse(xml).map_err(|_| Error::Config("The bucket's answer isn't S3.".into()))?;
    let mut files = Vec::new();
    for item in doc.descendants().filter(|n| n.tag_name().name() == "Contents") {
        let key = item.children().find(|n| n.tag_name().name() == "Key").and_then(|n| n.text());
        let size =
            item.children().find(|n| n.tag_name().name() == "Size").and_then(|n| n.text()?.parse().ok());
        if let (Some(key), Some(size)) = (key, size) {
            files.push(RemoteFile { name: key.rsplit('/').next().unwrap_or(key).to_string(), size });
        }
    }
    Ok(files)
}

fn percent_decode(text: &str) -> String {
    let mut out = Vec::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A sentence for a network problem (no address or secret in it).
fn network_error(err: reqwest::Error) -> Error {
    if err.is_timeout() {
        Error::Config("The server took too long to answer; Havre will try again.".into())
    } else if err.is_connect() {
        Error::Config("Couldn't reach the server: check the address, or that it's online.".into())
    } else {
        Error::Config("The connection to the server failed; Havre will try again.".into())
    }
}

/// A sentence for an HTTP status the server sent back.
fn http_error(status: StatusCode, kind: Kind, what: &str) -> Error {
    let why = match (status.as_u16(), kind) {
        (401, Kind::Webdav) => {
            "the server refused the user name or password (for Nextcloud, use an app password)"
        }
        (401 | 403, Kind::S3) => {
            "the bucket refused the keys (check the access key, the secret, and their rights)"
        }
        (403, Kind::Webdav) => "this user may not write there",
        (404, Kind::S3) => "the bucket doesn't exist (check its name, the region and the endpoint)",
        (404, Kind::Webdav) => "the folder's address wasn't found",
        (405, _) => "the address isn't a WebDAV folder",
        (507, _) | (413, _) => "there's no room left at the destination",
        (s, _) if s >= 500 => "the server had a problem; Havre will try again",
        _ => "the server said no",
    };
    Error::Config(format!("{what}: {why} (HTTP {}).", status.as_u16()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_except_on_this_machine() {
        assert!(check_url("https://cloud.example.org/remote.php/dav/files/me/Havre").is_ok());
        assert!(check_url("http://localhost:9000").is_ok());
        assert!(check_url("http://127.0.0.1:8081/dav").is_ok());
        assert!(check_url("http://nas.local/dav").is_err());
        assert!(check_url("ftp://example.org").is_err());
        assert!(check_url("https://me:secret@example.org/dav").is_err());
        assert!(check_url("not an address").is_err());
    }

    #[test]
    fn lists_and_deletes_only_copies_havre_made() {
        assert!(is_copy_name("havre-2026-11-08-030000.havre"));
        assert!(!is_copy_name("havre-2026-11-08-030000.db"));
        assert!(!is_copy_name("photos.havre"));
        assert!(!is_copy_name("havre-../x.havre"));
    }

    #[test]
    fn reads_a_nextcloud_propfind_answer() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">
 <d:response><d:href>/remote.php/dav/files/me/Havre/</d:href><d:propstat><d:prop/></d:propstat></d:response>
 <d:response><d:href>/remote.php/dav/files/me/Havre/havre-2026-11-08-030000.havre</d:href>
  <d:propstat><d:prop><d:getcontentlength>2048</d:getcontentlength></d:prop></d:propstat></d:response>
 <d:response><d:href>/remote.php/dav/files/me/Havre/my%20notes.txt</d:href>
  <d:propstat><d:prop><d:getcontentlength>5</d:getcontentlength></d:prop></d:propstat></d:response>
</d:multistatus>"#;
        let files = parse_propfind(xml).unwrap();
        assert_eq!(
            files,
            vec![
                RemoteFile { name: "havre-2026-11-08-030000.havre".into(), size: 2048 },
                RemoteFile { name: "my notes.txt".into(), size: 5 },
            ]
        );
    }

    #[test]
    fn reads_an_s3_list_answer() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>b</Name>
 <Contents><Key>havre/havre-2026-11-08-030000.havre</Key><Size>4096</Size></Contents>
</ListBucketResult>"#;
        assert_eq!(
            parse_list_objects(xml).unwrap(),
            vec![RemoteFile { name: "havre-2026-11-08-030000.havre".into(), size: 4096 }]
        );
    }

    #[test]
    fn builds_s3_addresses() {
        let mut remote = Remote {
            kind: Kind::S3,
            url: Url::parse("https://s3.eu-central-003.backblazeb2.com").unwrap(),
            bucket: "my-backups".into(),
            region: "eu-central-003".into(),
            prefix: "/havre/".into(),
            path_style: true,
            username: String::new(),
            secret: String::new(),
        };
        let key = remote.s3_key("havre-x.havre");
        assert_eq!(
            remote.s3_url(&key, "").as_str(),
            "https://s3.eu-central-003.backblazeb2.com/my-backups/havre/havre-x.havre"
        );
        remote.path_style = false;
        assert_eq!(
            remote.s3_url(&key, "").as_str(),
            "https://my-backups.s3.eu-central-003.backblazeb2.com/havre/havre-x.havre"
        );
    }
}
