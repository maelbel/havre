//! Google Contacts, through the People API (v1).
//!
//! <https://developers.google.com/people/api/rest>. The same sign-in as
//! Gmail, with the `contacts` permission.
//!
//! - **Reading**: `people/me/connections` lists your contacts. Asked with a
//!   *sync token* (from the previous read), it gives only what changed since,
//!   deleted contacts included. A token older than 7 days expires: Google
//!   answers `410 Gone` and the whole list is read again.
//! - **Writing**: `updateContact` needs the contact's `etag`, so a change made
//!   elsewhere in between isn't overwritten (Google refuses, and the next
//!   sync brings the newer version).

use serde_json::{Value, json};

use super::{Details, Phone, books::RemoteContact, kinds, sync::Changes, vcard::PhotoRef};
use crate::{Error, Result, oauth::http};

/// What Havre reads of a contact.
const FIELDS: &str = "names,emailAddresses,phoneNumbers,organizations,birthdays,biographies,photos,metadata";
/// What Havre changes.
const UPDATED: &str = "names,phoneNumbers,organizations,birthdays,biographies";

fn base() -> String {
    std::env::var("HAVRE_GOOGLE_PEOPLE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "https://people.googleapis.com/v1".into())
        .trim_end_matches('/')
        .to_owned()
}

async fn call(request: reqwest::RequestBuilder) -> Result<Value> {
    let response = request.send().await.map_err(|e| Error::Config(format!("Can't reach Google: {e}")))?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(body);
    }
    Err(error(status, &body))
}

/// Google's error, as a sentence the user can act on.
fn error(status: reqwest::StatusCode, body: &Value) -> Error {
    let message = body["error"]["message"].as_str().unwrap_or_default();
    match status.as_u16() {
        401 | 403 => Error::SignIn(format!(
            "Google refused access to your contacts ({message}). Reconnect the account in Settings, and check \
             that the People API is enabled in your Google Cloud project."
        )),
        404 => Error::NotFound("This contact no longer exists in Google Contacts.".into()),
        // `FAILED_PRECONDITION`: the etag is old, someone changed it meanwhile.
        400 if message.contains("etag") || body["error"]["status"] == "FAILED_PRECONDITION" => Error::Config(
            "This contact was changed in Google Contacts meanwhile: Havre will try again after the next sync."
                .into(),
        ),
        _ => Error::Config(format!("Google: {message} ({status})")),
    }
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Your contacts: those changed since `token`, or all of them (`token` is
/// `None`). `Ok(None)`: the token expired, read everything again.
pub async fn changes(access: &str, token: Option<&str>) -> Result<Option<Changes>> {
    let mut out = Changes { full: token.is_none(), ..Changes::default() };
    let mut page: Option<String> = None;
    for _ in 0..200 {
        let mut url = format!(
            "{}/people/me/connections?personFields={FIELDS}&pageSize=1000&requestSyncToken=true",
            base()
        );
        if let Some(token) = token {
            url.push_str(&format!("&syncToken={}", enc(token)));
        }
        if let Some(page) = &page {
            url.push_str(&format!("&pageToken={}", enc(page)));
        }
        let response = http()
            .get(&url)
            .bearer_auth(access)
            .send()
            .await
            .map_err(|e| Error::Config(format!("Can't reach Google: {e}")))?;
        if response.status() == reqwest::StatusCode::GONE {
            return Ok(None);
        }
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(error(status, &body));
        }
        for person in body["connections"].as_array().into_iter().flatten() {
            if person["metadata"]["deleted"].as_bool() == Some(true) {
                out.deleted.extend(person["resourceName"].as_str().map(str::to_owned));
            } else if let Some(contact) = to_remote(person) {
                out.contacts.push(contact);
            }
        }
        if let Some(next) = body["nextSyncToken"].as_str() {
            out.token = Some(next.to_owned());
        }
        match body["nextPageToken"].as_str() {
            Some(next) => page = Some(next.to_owned()),
            None => break,
        }
    }
    Ok(Some(out))
}

/// The first value of a list field (`names[0].displayName`).
fn first<'a>(person: &'a Value, list: &str, field: &str) -> &'a str {
    person[list].as_array().and_then(|l| l.first()).and_then(|v| v[field].as_str()).unwrap_or_default()
}

pub(super) fn to_remote(person: &Value) -> Option<RemoteContact> {
    let remote_id = person["resourceName"].as_str()?.to_owned();
    let mut name = first(person, "names", "displayName").trim().to_owned();
    if name.is_empty() {
        name = first(person, "names", "unstructuredName").trim().to_owned();
    }
    let emails = person["emailAddresses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["value"].as_str())
        .map(|e| e.trim().to_lowercase())
        .filter(|e| e.contains('@'))
        .fold(Vec::new(), |mut list, e| {
            if !list.contains(&e) {
                list.push(e);
            }
            list
        });
    let phones = person["phoneNumbers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let number = p["value"].as_str()?.trim().to_owned();
            // Google's types: mobile, home, work, main, other… or what you typed.
            let label = p["type"].as_str().unwrap_or_default().trim().to_lowercase();
            (!number.is_empty()).then_some(Phone { label, number })
        })
        .collect();
    let birthday = person["birthdays"].as_array().and_then(|l| l.first()).and_then(|b| {
        let date = &b["date"];
        let (m, d) = (date["month"].as_u64()?, date["day"].as_u64()?);
        let text = match date["year"].as_u64().filter(|y| *y > 0) {
            Some(y) => format!("{y:04}-{m:02}-{d:02}"),
            None => format!("--{m:02}-{d:02}"),
        };
        kinds::parse_birthday(&text).map(|_| text)
    });
    // `default: true` is Google's coloured letter, not a photo.
    let photo = person["photos"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["default"].as_bool() != Some(true))
        .and_then(|p| p["url"].as_str())
        .map(|url| PhotoRef::Url(sized(url)));
    Some(RemoteContact {
        remote_id,
        etag: person["etag"].as_str().map(str::to_owned),
        details: Details {
            name,
            company: first(person, "organizations", "name").trim().to_owned(),
            role: first(person, "organizations", "title").trim().to_owned(),
            phones,
            birthday,
            note: first(person, "biographies", "value").trim().to_owned(),
        },
        emails,
        photo,
        ..RemoteContact::default()
    })
}

/// Google's photo addresses end with their size (`=s100`): ask for 128 px.
fn sized(url: &str) -> String {
    match url.rsplit_once("=s") {
        Some((start, size)) if !size.is_empty() && size.chars().all(|c| c.is_ascii_digit()) => {
            format!("{start}=s128")
        }
        _ => url.to_owned(),
    }
}

/// The fields Havre writes, as Google wants them.
fn body(details: &Details) -> Value {
    let birthday = details.birthday.as_deref().and_then(kinds::parse_birthday).map(|(m, d, y)| {
        let mut date = json!({ "month": m, "day": d });
        if let Some(y) = y {
            date["year"] = json!(y);
        }
        json!([{ "date": date }])
    });
    let organizations = if details.company.trim().is_empty() && details.role.trim().is_empty() {
        json!([])
    } else {
        json!([{ "name": details.company.trim(), "title": details.role.trim() }])
    };
    json!({
        "names": if details.name.trim().is_empty() { json!([]) } else { json!([{ "unstructuredName": details.name.trim() }]) },
        "phoneNumbers": details.phones.iter().map(|p| json!({ "value": p.number, "type": p.label })).collect::<Vec<_>>(),
        "organizations": organizations,
        "birthdays": birthday.unwrap_or_else(|| json!([])),
        "biographies": if details.note.trim().is_empty() {
            json!([])
        } else {
            json!([{ "value": details.note.trim(), "contentType": "TEXT_PLAIN" }])
        },
    })
}

pub async fn update(
    access: &str,
    resource: &str,
    etag: Option<&str>,
    details: &Details,
) -> Result<RemoteContact> {
    let mut body = body(details);
    body["etag"] = json!(etag);
    let url =
        format!("{}/{resource}:updateContact?updatePersonFields={UPDATED}&personFields={FIELDS}", base());
    let person = call(http().patch(url).bearer_auth(access).json(&body)).await?;
    to_remote(&person).ok_or_else(|| Error::Config("Google didn't return the contact".into()))
}

pub async fn create(access: &str, details: &Details, emails: &[String]) -> Result<RemoteContact> {
    let mut body = body(details);
    body["emailAddresses"] = json!(emails.iter().map(|e| json!({ "value": e })).collect::<Vec<_>>());
    let url = format!("{}/people:createContact?personFields={FIELDS}", base());
    let person = call(http().post(url).bearer_auth(access).json(&body)).await?;
    to_remote(&person).ok_or_else(|| Error::Config("Google didn't return the contact".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_google_contacts() {
        // A connection as the People API sends it (trimmed).
        let person = json!({
            "resourceName": "people/c123", "etag": "%EgUBAi43PRoEAQIFByIM",
            "metadata": { "sources": [{ "type": "CONTACT", "id": "123" }] },
            "names": [{ "displayName": "Claire Martin", "givenName": "Claire", "familyName": "Martin" }],
            "emailAddresses": [{ "value": "Claire.Martin@acme.fr" }, { "value": "claire.martin@acme.fr" }],
            "phoneNumbers": [{ "value": "+33 6 12 34 56 78", "type": "mobile", "formattedType": "Mobile" }],
            "organizations": [{ "name": "Acme", "title": "Project lead" }],
            "birthdays": [{ "date": { "month": 11, "day": 12 } }],
            "biographies": [{ "value": "Prefers calls", "contentType": "TEXT_PLAIN" }],
            "photos": [{ "url": "https://lh3.googleusercontent.com/a/x=s100", "default": false }]
        });
        let c = to_remote(&person).unwrap();
        assert_eq!(c.remote_id, "people/c123");
        assert_eq!(c.emails, ["claire.martin@acme.fr"], "lower case, each once");
        assert_eq!(c.details.company, "Acme");
        assert_eq!(c.details.role, "Project lead");
        assert_eq!(c.details.birthday.as_deref(), Some("--11-12"));
        assert_eq!(c.details.phones[0].label, "mobile");
        assert_eq!(c.photo, Some(PhotoRef::Url("https://lh3.googleusercontent.com/a/x=s128".into())));
        // Google's default letter isn't a photo.
        let plain = json!({ "resourceName": "people/c2", "photos": [{ "url": "https://x/l=s100", "default": true }] });
        assert_eq!(to_remote(&plain).unwrap().photo, None);

        // Writing back: the same fields, Google's way; empty ones are cleared.
        let mut d = c.details.clone();
        d.birthday = Some("1988-11-12".into());
        d.note.clear();
        let b = body(&d);
        assert_eq!(b["names"][0]["unstructuredName"], "Claire Martin");
        assert_eq!(b["birthdays"][0]["date"], json!({ "year": 1988, "month": 11, "day": 12 }));
        assert_eq!(b["biographies"], json!([]));
        assert_eq!(b["phoneNumbers"][0], json!({ "value": "+33 6 12 34 56 78", "type": "mobile" }));
    }
}
