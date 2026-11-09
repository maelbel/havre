//! CardDAV (RFC 6352): address books on the servers added for the Agenda
//! (iCloud, Nextcloud, Fastmail, Radicale…). Most CalDAV servers serve
//! contacts too, with the same login.
//!
//! It's WebDAV again (see `agenda/caldav.rs`, whose helpers are reused),
//! with vCards instead of `.ics` files:
//!
//! 1. **Discovery**: your *principal*, then its `addressbook-home-set`,
//!    then the address books in it.
//! 2. **What changed**: an address book's `getctag` changes whenever
//!    anything in it does, so an unchanged one costs one request. Otherwise
//!    each vCard's `ETag` is compared with the one Havre has, and only the
//!    changed ones are read (`addressbook-multiget`).
//! 3. **Writing**: `PUT` the vCard with `If-Match`, so a change made
//!    elsewhere in between isn't overwritten.

use std::collections::HashMap;

use super::{
    Details,
    books::{RemoteBook, RemoteContact},
    sync::Changes,
    vcard::{self, Card},
};
use crate::{
    Error, Result,
    agenda::{
        caldav::{DAV, absolute, href_in, origin, prop, request},
        store::CalDavLogin,
    },
    oauth::http,
};

const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";
const CALSERVER: &str = "http://calendarserver.org/ns/";
const CONFLICT: &str =
    "This contact was changed elsewhere meanwhile: Havre will try again after the next sync.";

fn ok(status: reqwest::StatusCode) -> bool {
    status.is_success() || status.as_u16() == 207
}

fn parse(xml: &str) -> Result<roxmltree::Document<'_>> {
    roxmltree::Document::parse(xml).map_err(|e| Error::Config(format!("CardDAV: unreadable answer ({e})")))
}

/// Where to look for your address books, from the address you gave for the
/// Agenda: that address, the server's `.well-known/carddav`, and iCloud's
/// contacts server (its calendars and contacts live on different hosts).
fn starting_points(url: &str) -> Vec<String> {
    let mut out = vec![url.to_owned(), format!("{}/.well-known/carddav", origin(url))];
    if url.contains("icloud.com") {
        out.push("https://contacts.icloud.com".into());
    }
    out
}

/// The address books of this account.
pub async fn books(login: &CalDavLogin) -> Result<Vec<RemoteBook>> {
    let principal_query =
        r#"<d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;
    let home_query = r#"<d:propfind xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav"><d:prop><card:addressbook-home-set/></d:prop></d:propfind>"#;
    let mut home = None;
    for start in starting_points(&login.url) {
        let Ok((status, xml)) = request(login, "PROPFIND", &start, "0", principal_query.into()).await else {
            continue;
        };
        let principal = parse(&xml).ok().filter(|_| ok(status)).and_then(|d| {
            d.descendants().find(|n| n.has_tag_name((DAV, "current-user-principal"))).and_then(href_in)
        });
        let Some(principal) = principal else { continue };
        let principal = absolute(&start, &principal);
        let Ok((status, xml)) = request(login, "PROPFIND", &principal, "0", home_query.into()).await else {
            continue;
        };
        home = parse(&xml).ok().filter(|_| ok(status)).and_then(|d| {
            d.descendants().find(|n| n.has_tag_name((CARDDAV, "addressbook-home-set"))).and_then(href_in)
        });
        if let Some(found) = home.take() {
            home = Some(absolute(&principal, &found));
            break;
        }
    }
    let home = home.ok_or_else(|| Error::Config("This server has no address book (CardDAV).".into()))?;

    let list_query = r#"<d:propfind xmlns:d="DAV:"><d:prop><d:displayname/><d:resourcetype/><d:current-user-privilege-set/></d:prop></d:propfind>"#;
    let (_, xml) = request(login, "PROPFIND", &home, "1", list_query.into()).await?;
    let doc = parse(&xml)?;
    let mut out = Vec::new();
    for response in doc.descendants().filter(|n| n.has_tag_name((DAV, "response"))) {
        let Some(href) = href_in(response) else { continue };
        let is_book = prop(response, DAV, "resourcetype")
            .is_some_and(|rt| rt.children().any(|c| c.has_tag_name((CARDDAV, "addressbook"))));
        if !is_book {
            continue;
        }
        let read_only = prop(response, DAV, "current-user-privilege-set").is_some_and(|set| {
            !set.descendants().any(|n| {
                n.has_tag_name((DAV, "write"))
                    || n.has_tag_name((DAV, "write-content"))
                    || n.has_tag_name((DAV, "all"))
            })
        });
        let name = prop(response, DAV, "displayname")
            .and_then(|n| n.text())
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                href.trim_end_matches('/').rsplit('/').next().unwrap_or("Contacts").to_owned()
            });
        out.push(RemoteBook { remote_id: absolute(&home, &href), name, read_only });
    }
    if out.is_empty() {
        return Err(Error::Config("This server has no address book (CardDAV).".into()));
    }
    Ok(out)
}

/// What changed in an address book since Havre last looked. `ctag`: what
/// it was then; `known`: each vCard's address (Havre's remote id) and ETag.
/// `Ok(None)`: nothing changed.
pub async fn changes(
    login: &CalDavLogin,
    book: &str,
    ctag: Option<&str>,
    known: &HashMap<String, Option<String>>,
) -> Result<Option<Changes>> {
    let ctag_query = r#"<d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/"><d:prop><cs:getctag/><d:sync-token/></d:prop></d:propfind>"#;
    let (status, xml) = request(login, "PROPFIND", book, "0", ctag_query.into()).await?;
    let now_tag = if ok(status) {
        parse(&xml).ok().and_then(|d| {
            let response = d.descendants().find(|n| n.has_tag_name((DAV, "response")))?;
            prop(response, CALSERVER, "getctag")
                .or_else(|| prop(response, DAV, "sync-token"))
                .and_then(|n| n.text())
                .map(|t| t.trim().to_owned())
        })
    } else {
        None
    };
    if now_tag.is_some() && now_tag.as_deref() == ctag {
        return Ok(None);
    }

    // Every vCard's ETag.
    let etag_query =
        r#"<d:propfind xmlns:d="DAV:"><d:prop><d:getetag/><d:resourcetype/></d:prop></d:propfind>"#;
    let (status, xml) = request(login, "PROPFIND", book, "1", etag_query.into()).await?;
    if !ok(status) {
        return Err(Error::Config(format!("CardDAV: the address book answered {status}")));
    }
    let doc = parse(&xml)?;
    let mut listed: HashMap<String, Option<String>> = HashMap::new();
    // The address as the server spells it (to ask for it), by its key (to compare).
    let mut spelled: HashMap<String, String> = HashMap::new();
    for response in doc.descendants().filter(|n| n.has_tag_name((DAV, "response"))) {
        let Some(href) = href_in(response) else { continue };
        let is_collection = prop(response, DAV, "resourcetype")
            .is_some_and(|rt| rt.children().any(|c| c.has_tag_name((DAV, "collection"))));
        if is_collection {
            continue;
        }
        let etag = prop(response, DAV, "getetag").and_then(|n| n.text()).map(|t| t.trim().to_owned());
        listed.insert(key(book, &href), etag);
        spelled.insert(key(book, &href), absolute(book, &href));
    }

    let changed: Vec<&String> = listed
        .iter()
        .filter(|(href, etag)| known.get(*href) != Some(*etag) || etag.is_none())
        .map(|(h, _)| h)
        .collect();
    let mut out = Changes {
        deleted: known.keys().filter(|href| !listed.contains_key(*href)).cloned().collect(),
        token: now_tag,
        ..Changes::default()
    };
    // Read the changed ones, 50 at a time.
    for chunk in changed.chunks(50) {
        let hrefs: String = chunk
            .iter()
            .map(|h| format!("<d:href>{}</d:href>", xml_escape(&path_of(spelled.get(*h).unwrap_or(h)))))
            .collect();
        let query = format!(
            r#"<card:addressbook-multiget xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav"><d:prop><d:getetag/><card:address-data/></d:prop>{hrefs}</card:addressbook-multiget>"#
        );
        let (status, xml) = request(login, "REPORT", book, "1", query).await?;
        if !ok(status) {
            return Err(Error::Config(format!("CardDAV: reading the contacts answered {status}")));
        }
        let doc = parse(&xml)?;
        for response in doc.descendants().filter(|n| n.has_tag_name((DAV, "response"))) {
            let Some(href) = href_in(response) else { continue };
            let Some(data) = prop(response, CARDDAV, "address-data").and_then(|n| n.text()) else { continue };
            let etag = prop(response, DAV, "getetag").and_then(|n| n.text()).map(|t| t.trim().to_owned());
            if let Some(mut contact) = read(data, &key(book, &href), etag) {
                // Written back where the server said it is.
                contact.href = Some(absolute(book, &href));
                out.contacts.push(contact);
            }
        }
    }
    Ok(Some(out))
}

/// A vCard's address as Havre keys it: absolute, with `%40` and the like
/// decoded, since servers spell the same address either way.
fn key(book: &str, href: &str) -> String {
    let url = absolute(book, href);
    let bytes = url.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = if bytes[i] == b'%' { url.get(i + 1..i + 3) } else { None };
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or(url)
}

/// `https://x.org/dav/a.vcf` → `/dav/a.vcf` (multiget wants paths).
fn path_of(url: &str) -> String {
    url.strip_prefix(&origin(url)).filter(|p| !p.is_empty()).unwrap_or(url).to_owned()
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A vCard resource as a contact (`None`: not a person, or unreadable).
pub(super) fn read(text: &str, href: &str, etag: Option<String>) -> Option<RemoteContact> {
    let card = vcard::parse(text).into_iter().next()?;
    if card.is_group() {
        return None;
    }
    Some(RemoteContact {
        remote_id: href.to_owned(),
        href: Some(href.to_owned()),
        etag,
        raw: Some(text.to_owned()),
        details: card.details(),
        emails: card.emails(),
        photo: card.photo(),
    })
}

async fn put(login: &CalDavLogin, href: &str, etag: Option<&str>, text: &str) -> Result<Option<String>> {
    let mut req = http()
        .put(href)
        .basic_auth(&login.username, Some(&login.password))
        .header("Content-Type", "text/vcard; charset=utf-8")
        .body(text.to_owned());
    req = match etag {
        Some(etag) => req.header("If-Match", etag),
        None => req.header("If-None-Match", "*"),
    };
    let response =
        req.send().await.map_err(|e| Error::Config(format!("Can't reach {}: {e}", origin(href))))?;
    match response.status().as_u16() {
        200..=299 => Ok(response.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_owned)),
        412 => Err(Error::Config(CONFLICT.into())),
        401 | 403 => Err(Error::SignIn("The CardDAV server refused the change.".into())),
        status => Err(Error::Config(format!("CardDAV: the server answered {status}"))),
    }
}

/// Some servers don't send the new ETag after a `PUT`: ask for it.
async fn etag_of(login: &CalDavLogin, href: &str) -> Option<String> {
    let query = r#"<d:propfind xmlns:d="DAV:"><d:prop><d:getetag/></d:prop></d:propfind>"#;
    let (_, xml) = request(login, "PROPFIND", href, "0", query.into()).await.ok()?;
    let doc = roxmltree::Document::parse(&xml).ok()?;
    let response = doc.descendants().find(|n| n.has_tag_name((DAV, "response")))?;
    prop(response, DAV, "getetag").and_then(|n| n.text()).map(|t| t.trim().to_owned())
}

pub async fn update(
    login: &CalDavLogin,
    href: &str,
    etag: Option<&str>,
    raw: &str,
    details: &Details,
) -> Result<RemoteContact> {
    let mut card =
        vcard::parse(raw).into_iter().next().ok_or_else(|| Error::Config("an empty vCard".into()))?;
    card.apply(details);
    let text = card.to_vcf();
    let etag = match put(login, href, etag, &text).await? {
        Some(etag) => Some(etag),
        None => etag_of(login, href).await,
    };
    read(&text, href, etag).ok_or_else(|| Error::Config("the saved contact can't be read back".into()))
}

pub async fn create(
    login: &CalDavLogin,
    book: &str,
    details: &Details,
    emails: &[String],
) -> Result<RemoteContact> {
    // Only letters and digits in the file name: nothing for servers to encode.
    let id = crate::agenda::random_id();
    let uid = format!("{id}@havre");
    let href = format!("{}/{id}.vcf", book.trim_end_matches('/'));
    let text = Card::new(&uid, details, emails).to_vcf();
    let etag = match put(login, &href, None, &text).await? {
        Some(etag) => Some(etag),
        None => etag_of(login, &href).await,
    };
    read(&text, &href, etag).ok_or_else(|| Error::Config("the saved contact can't be read back".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_books_and_reads_cards() {
        assert_eq!(
            starting_points("https://caldav.icloud.com"),
            [
                "https://caldav.icloud.com",
                "https://caldav.icloud.com/.well-known/carddav",
                "https://contacts.icloud.com"
            ]
        );
        assert_eq!(path_of("https://x.org/dav/a%20b.vcf"), "/dav/a%20b.vcf");
        // `%40` and `@` are the same card.
        assert_eq!(
            key("https://x.org/ab/", "/ab/x%40havre.vcf"),
            key("https://x.org/ab/", "https://x.org/ab/x@havre.vcf")
        );

        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ana\r\nEMAIL:ana@x.org\r\nEND:VCARD\r\n";
        let c = read(text, "https://x.org/ab/1.vcf", Some("\"7\"".into())).unwrap();
        assert_eq!((c.remote_id.as_str(), c.details.name.as_str()), ("https://x.org/ab/1.vcf", "Ana"));
        assert_eq!(c.raw.as_deref(), Some(text));
        let group = "BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:group\r\nFN:Team\r\nEND:VCARD\r\n";
        assert!(read(group, "h", None).is_none());
    }
}
