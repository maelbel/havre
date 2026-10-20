//! CalDAV (RFC 4791): Nextcloud, iCloud, Fastmail, Radicale, SOGo…
//!
//! CalDAV is WebDAV (HTTP with extra methods and XML bodies) where each
//! event is an `.ics` file:
//!
//! 1. **Discovery**: `PROPFIND` the address you gave for your
//!    *principal* (who you are), then your *calendar home*, then list the
//!    calendars in it.
//! 2. **Reading**: a `REPORT` asks one calendar for the events in a period
//!    (their `.ics` text, expanded by [`super::ics`]).
//! 3. **Writing**: `PUT` an `.ics` file, `DELETE` it. `ETag`s make sure we
//!    don't overwrite a change made elsewhere in between.

use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use reqwest::{Method, StatusCode};

use super::{EventInput, RemoteCalendar, RemoteEvent, ics, store::CalDavLogin};
use crate::{Error, Result, oauth::http};

pub(crate) const DAV: &str = "DAV:";
pub(crate) const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
const APPLE: &str = "http://apple.com/ns/ical/";

fn method(name: &str) -> Method {
    Method::from_bytes(name.as_bytes()).expect("valid method")
}

/// `https://cloud.example.com` from `https://cloud.example.com/remote.php/dav`.
pub(crate) fn origin(url: &str) -> String {
    match url.find("://").map(|i| i + 3) {
        Some(start) => match url[start..].find('/') {
            Some(slash) => url[..start + slash].to_owned(),
            None => url.to_owned(),
        },
        None => url.to_owned(),
    }
}

pub(crate) fn absolute(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_owned()
    } else {
        format!("{}{href}", origin(base))
    }
}

pub(crate) async fn request(
    login: &CalDavLogin,
    verb: &str,
    url: &str,
    depth: &str,
    body: String,
) -> Result<(StatusCode, String)> {
    let response = http()
        .request(method(verb), url)
        .basic_auth(&login.username, Some(&login.password))
        .header("Depth", depth)
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(body)
        .send()
        .await
        .map_err(|e| Error::Config(format!("Can't reach {}: {e}", origin(url))))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    match status.as_u16() {
        401 => Err(Error::SignIn("The CalDAV server refused the user name or password.".into())),
        403 => Err(Error::SignIn("The CalDAV server refused access.".into())),
        _ => Ok((status, text)),
    }
}

/// A property value: the text of the first matching element under `<prop>`
/// of a successful `<propstat>`.
pub(crate) fn prop<'a>(
    response: roxmltree::Node<'a, 'a>,
    ns: &str,
    name: &str,
) -> Option<roxmltree::Node<'a, 'a>> {
    response
        .descendants()
        .filter(|n| n.has_tag_name((DAV, "propstat")))
        .filter(|ps| {
            ps.children()
                .find(|n| n.has_tag_name((DAV, "status")))
                .and_then(|s| s.text())
                .is_none_or(|s| s.contains(" 200"))
        })
        .find_map(|ps| ps.descendants().find(|n| n.has_tag_name((ns, name))))
}

pub(crate) fn href_in(node: roxmltree::Node<'_, '_>) -> Option<String> {
    node.descendants().find(|n| n.has_tag_name((DAV, "href")))?.text().map(|t| t.trim().to_owned())
}

/// A calendar collection on the server: it can hold events, tasks, or both.
#[derive(Debug, Clone)]
pub(crate) struct Collection {
    pub href: String,
    pub name: String,
    pub color: Option<String>,
    pub read_only: bool,
    pub events: bool,
    pub todos: bool,
}

/// Finds your calendar collections from the address you gave (server root,
/// `.well-known` or a principal URL all work). The Agenda uses those with
/// events, Tasks those with to-dos (`VTODO`).
pub(crate) async fn collections(login: &CalDavLogin) -> Result<Vec<Collection>> {
    let principal_query =
        r#"<d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;
    let mut principal = None;
    for url in [login.url.clone(), format!("{}/.well-known/caldav", origin(&login.url))] {
        let (status, xml) = request(login, "PROPFIND", &url, "0", principal_query.into()).await?;
        if status.is_success() || status.as_u16() == 207 {
            let doc = roxmltree::Document::parse(&xml).ok();
            principal = doc.and_then(|d| {
                d.descendants().find(|n| n.has_tag_name((DAV, "current-user-principal"))).and_then(href_in)
            });
            if principal.is_some() {
                break;
            }
        }
    }
    let principal = absolute(
        &login.url,
        &principal.ok_or_else(|| Error::Config("No CalDAV account found at this address.".into()))?,
    );

    let home_query = r#"<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-home-set/></d:prop></d:propfind>"#;
    let (_, xml) = request(login, "PROPFIND", &principal, "0", home_query.into()).await?;
    let home = roxmltree::Document::parse(&xml)
        .ok()
        .and_then(|d| {
            d.descendants().find(|n| n.has_tag_name((CALDAV, "calendar-home-set"))).and_then(href_in)
        })
        .ok_or_else(|| Error::Config("This CalDAV account has no calendars.".into()))?;
    let home = absolute(&login.url, &home);

    let list_query = r#"<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/">
  <d:prop><d:displayname/><d:resourcetype/><a:calendar-color/><c:supported-calendar-component-set/><d:current-user-privilege-set/></d:prop>
</d:propfind>"#;
    let (_, xml) = request(login, "PROPFIND", &home, "1", list_query.into()).await?;
    let doc = roxmltree::Document::parse(&xml)
        .map_err(|e| Error::Config(format!("CalDAV: unreadable answer ({e})")))?;
    let mut out = Vec::new();
    for response in doc.descendants().filter(|n| n.has_tag_name((DAV, "response"))) {
        let Some(href) = href_in(response) else { continue };
        let is_calendar = prop(response, DAV, "resourcetype")
            .is_some_and(|rt| rt.children().any(|c| c.has_tag_name((CALDAV, "calendar"))));
        if !is_calendar {
            continue;
        }
        // No component set announced: the server allows everything.
        let set = prop(response, CALDAV, "supported-calendar-component-set");
        let supports = |name: &str| {
            set.is_none_or(|set| {
                set.children().any(|c| c.has_tag_name((CALDAV, "comp")) && c.attribute("name") == Some(name))
            })
        };
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
                href.trim_end_matches('/').rsplit('/').next().unwrap_or("Calendar").to_owned()
            });
        let color = prop(response, APPLE, "calendar-color")
            .and_then(|n| n.text())
            .map(|c| c.trim().to_lowercase())
            .filter(|c| c.starts_with('#') && c.len() >= 7)
            .map(|c| c[..7].to_owned());
        out.push(Collection {
            href: absolute(&login.url, &href),
            name,
            color,
            read_only,
            events: supports("VEVENT"),
            todos: supports("VTODO"),
        });
    }
    Ok(out)
}

/// The calendars with events.
pub async fn calendars(login: &CalDavLogin) -> Result<Vec<RemoteCalendar>> {
    let mut out = Vec::new();
    for c in collections(login).await?.into_iter().filter(|c| c.events) {
        out.push(RemoteCalendar {
            remote_id: c.href,
            name: c.name,
            color: c.color,
            read_only: c.read_only,
            is_default: out.is_empty(),
        });
    }
    if out.is_empty() {
        return Err(Error::Config("No calendar with events found in this CalDAV account.".into()));
    }
    Ok(out)
}

pub(crate) fn stamp(ts: i64) -> String {
    Utc.timestamp_opt(ts, 0).single().unwrap_or_default().format("%Y%m%dT%H%M%SZ").to_string()
}

pub async fn events(
    login: &CalDavLogin,
    calendar: &str,
    from: i64,
    to: i64,
    read_only: bool,
) -> Result<Vec<RemoteEvent>> {
    let query = format!(
        r#"<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/><c:calendar-data/></d:prop>
  <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">
    <c:time-range start="{}" end="{}"/>
  </c:comp-filter></c:comp-filter></c:filter>
</c:calendar-query>"#,
        stamp(from),
        stamp(to)
    );
    let (status, xml) = request(login, "REPORT", calendar, "1", query).await?;
    if !(status.is_success() || status.as_u16() == 207) {
        return Err(Error::Config(format!("CalDAV: the calendar answered {status}")));
    }
    let doc = roxmltree::Document::parse(&xml)
        .map_err(|e| Error::Config(format!("CalDAV: unreadable answer ({e})")))?;
    let mut out = Vec::new();
    for response in doc.descendants().filter(|n| n.has_tag_name((DAV, "response"))) {
        let Some(href) = href_in(response) else { continue };
        let Some(data) = prop(response, CALDAV, "calendar-data").and_then(|n| n.text()) else { continue };
        let etag = prop(response, DAV, "getetag").and_then(|n| n.text()).map(|t| t.trim().to_owned());
        let href = absolute(calendar, &href);
        match ics::events_between(data, Some(&href), etag.as_deref(), from, to, false) {
            Ok(events) => out.extend(events.into_iter().map(|mut e| {
                e.read_only |= read_only;
                e
            })),
            Err(err) => tracing::warn!(href, "skipping an unreadable event: {err}"),
        }
    }
    Ok(out)
}

/// Writes a resource. `conflict`: what to say when it changed elsewhere.
pub(crate) async fn put_with(
    login: &CalDavLogin,
    href: &str,
    etag: Option<&str>,
    body: &str,
    conflict: &str,
) -> Result<Option<String>> {
    let mut request = http()
        .put(href)
        .basic_auth(&login.username, Some(&login.password))
        .header("Content-Type", "text/calendar; charset=utf-8")
        .body(body.to_owned());
    request = match etag {
        Some(etag) => request.header("If-Match", etag),
        None => request.header("If-None-Match", "*"),
    };
    let response =
        request.send().await.map_err(|e| Error::Config(format!("Can't reach {}: {e}", origin(href))))?;
    match response.status().as_u16() {
        200..=299 => Ok(response.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_owned)),
        412 => Err(Error::Config(conflict.into())),
        401 | 403 => Err(Error::SignIn("The CalDAV server refused the change.".into())),
        status => Err(Error::Config(format!("CalDAV: the server answered {status}"))),
    }
}

fn read_back(text: &str, href: &str, etag: Option<String>) -> Result<RemoteEvent> {
    let mut event = ics::events_between(text, Some(href), etag.as_deref(), i64::MIN / 4, i64::MAX / 4, true)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Config("the saved event can't be read back".into()))?;
    event.etag = etag;
    Ok(event)
}

/// Creates the event in its own `.ics` file (a repeating one, kept in `tz`,
/// when `input.repeat` is set).
pub async fn create(
    login: &CalDavLogin,
    calendar: &str,
    input: &EventInput,
    tz: Option<Tz>,
) -> Result<RemoteEvent> {
    let uid = format!("{}@havre", crate::agenda::manager::random_id());
    let href = format!("{}/{uid}.ics", calendar.trim_end_matches('/'));
    let text = match &input.repeat {
        Some(repeat) => ics::new_series_ics(&uid, input, repeat, tz.unwrap_or(Tz::UTC)),
        None => ics::new_ics(&uid, input),
    };
    let etag = put(login, &href, None, &text).await?;
    read_back(&text, &href, etag)
}

pub async fn update(
    login: &CalDavLogin,
    href: &str,
    etag: Option<&str>,
    raw: &str,
    input: &EventInput,
) -> Result<RemoteEvent> {
    let text = ics::update_ics(raw, input)?;
    let etag = put(login, href, etag, &text).await?;
    read_back(&text, href, etag)
}

const EVENT_CONFLICT: &str = "This event was changed elsewhere: refresh the Agenda and try again.";

/// Writes an event's `.ics` file (only if it didn't change elsewhere: `etag`).
pub(crate) async fn put(
    login: &CalDavLogin,
    href: &str,
    etag: Option<&str>,
    body: &str,
) -> Result<Option<String>> {
    put_with(login, href, etag, body, EVENT_CONFLICT).await
}

pub async fn delete(login: &CalDavLogin, href: &str, etag: Option<&str>) -> Result<()> {
    delete_with(login, href, etag, EVENT_CONFLICT).await
}

pub(crate) async fn delete_with(
    login: &CalDavLogin,
    href: &str,
    etag: Option<&str>,
    conflict: &str,
) -> Result<()> {
    let mut request = http().delete(href).basic_auth(&login.username, Some(&login.password));
    if let Some(etag) = etag {
        request = request.header("If-Match", etag);
    }
    let response =
        request.send().await.map_err(|e| Error::Config(format!("Can't reach {}: {e}", origin(href))))?;
    match response.status().as_u16() {
        200..=299 | 404 | 410 => Ok(()),
        412 => Err(Error::Config(conflict.into())),
        status => Err(Error::Config(format!("CalDAV: the server answered {status}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_urls() {
        assert_eq!(origin("https://cloud.example.com/remote.php/dav/"), "https://cloud.example.com");
        assert_eq!(absolute("https://x.org/dav/", "/dav/cal/"), "https://x.org/dav/cal/");
        assert_eq!(absolute("https://x.org/dav/", "https://y.org/c/"), "https://y.org/c/");
    }
}
