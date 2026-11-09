//! Outlook contacts (Microsoft 365, Outlook.com), through Microsoft Graph.
//!
//! <https://learn.microsoft.com/graph/api/resources/contact>. The same
//! sign-in as the mailbox, with the `Contacts.ReadWrite` permission. Havre
//! syncs your main Contacts folder.
//!
//! - **Reading**: the whole folder each time (one request per 500
//!   contacts); a contact whose `changeKey` didn't change is skipped.
//! - **Phones**: Outlook has fixed places, not a list: one mobile, two work
//!   and two home numbers. Havre's labels go to those places (`mobile`, `home`,
//!   anything else at work); more than fit are left out.
//! - **Birthdays** without a year are stored by Outlook in the year 1604.

use chrono::{DateTime, Datelike};
use serde_json::{Value, json};

use super::{Details, Phone, books::RemoteContact, kinds, sync::Changes, vcard::PhotoRef};
use crate::{Error, Result, oauth::http};

const SELECT: &str = "id,changeKey,displayName,givenName,surname,emailAddresses,mobilePhone,businessPhones,\
homePhones,companyName,jobTitle,birthday,personalNotes";

/// Outlook's year for "no year".
const NO_YEAR: i32 = 1604;

fn base() -> String {
    std::env::var("HAVRE_MICROSOFT_GRAPH_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "https://graph.microsoft.com/v1.0".into())
        .trim_end_matches('/')
        .to_owned()
}

async fn call(request: reqwest::RequestBuilder) -> Result<Value> {
    let response = request
        .header("Prefer", "outlook.body-content-type=\"text\"")
        .send()
        .await
        .map_err(|e| Error::Config(format!("Can't reach Microsoft: {e}")))?;
    let status = response.status();
    if status == reqwest::StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(body);
    }
    let message = body["error"]["message"].as_str().unwrap_or_default();
    Err(match status.as_u16() {
        401 | 403 => Error::SignIn(format!(
            "Microsoft refused access to your contacts ({message}). Reconnect the account in Settings; your admin \
             may need to approve the Contacts.ReadWrite permission."
        )),
        404 => Error::NotFound("This contact no longer exists in Outlook.".into()),
        _ => Error::Config(format!("Microsoft: {message} ({status})")),
    })
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Every contact of your main Contacts folder.
pub async fn changes(token: &str) -> Result<Changes> {
    let mut out = Changes { full: true, ..Changes::default() };
    let mut url = format!("{}/me/contacts?$top=500&$select={SELECT}", base());
    for _ in 0..200 {
        let body = call(http().get(&url).bearer_auth(token)).await?;
        out.contacts.extend(body["value"].as_array().into_iter().flatten().filter_map(to_remote));
        match body["@odata.nextLink"].as_str() {
            Some(next) => url = next.to_owned(),
            None => break,
        }
    }
    Ok(out)
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().trim().to_owned()
}

pub(super) fn to_remote(c: &Value) -> Option<RemoteContact> {
    let mut name = text(&c["displayName"]);
    if name.is_empty() {
        name = [text(&c["givenName"]), text(&c["surname"])]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
    let mut emails: Vec<String> = Vec::new();
    for e in c["emailAddresses"].as_array().into_iter().flatten() {
        let email = text(&e["address"]).to_lowercase();
        if email.contains('@') && !emails.contains(&email) {
            emails.push(email);
        }
    }
    let mut phones = Vec::new();
    let mobile = text(&c["mobilePhone"]);
    if !mobile.is_empty() {
        phones.push(Phone { label: "mobile".into(), number: mobile });
    }
    for (list, label) in [("businessPhones", "work"), ("homePhones", "home")] {
        for number in c[list].as_array().into_iter().flatten().map(text).filter(|n| !n.is_empty()) {
            phones.push(Phone { label: label.into(), number });
        }
    }
    // "1988-11-12T11:59:00Z": the date counts (Outlook puts noon-ish to dodge time zones).
    let birthday = c["birthday"].as_str().and_then(|b| DateTime::parse_from_rfc3339(b).ok()).and_then(|d| {
        let text = if d.year() == NO_YEAR {
            format!("--{:02}-{:02}", d.month(), d.day())
        } else {
            format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())
        };
        kinds::parse_birthday(&text).map(|_| text)
    });
    Some(RemoteContact {
        remote_id: c["id"].as_str()?.to_owned(),
        etag: c["changeKey"].as_str().map(str::to_owned),
        details: Details {
            name,
            company: text(&c["companyName"]),
            role: text(&c["jobTitle"]),
            phones,
            birthday,
            note: text(&c["personalNotes"]),
        },
        emails,
        // Outlook says whether there's a photo only when asked for it.
        photo: Some(PhotoRef::Url("graph".into())),
        ..RemoteContact::default()
    })
}

/// The fields Havre writes, in Outlook's places.
fn body(details: &Details) -> Value {
    let name = details.name.trim();
    let (given, surname) = match name.rsplit_once(' ') {
        Some((given, surname)) => (given.trim(), surname.trim()),
        None => (name, ""),
    };
    let mut mobile: Option<&str> = None;
    let (mut business, mut home): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    for p in &details.phones {
        let number = p.number.trim();
        match p.label.as_str() {
            "mobile" | "cell" if mobile.is_none() => mobile = Some(number),
            "home" if home.len() < 2 => home.push(number),
            _ if business.len() < 2 => business.push(number),
            _ => {} // no room left in Outlook
        }
    }
    let birthday = details
        .birthday
        .as_deref()
        .and_then(kinds::parse_birthday)
        .map(|(m, d, y)| format!("{:04}-{m:02}-{d:02}T11:59:00Z", y.unwrap_or(NO_YEAR)));
    json!({
        "displayName": name,
        "givenName": given,
        "surname": surname,
        "companyName": details.company.trim(),
        "jobTitle": details.role.trim(),
        "mobilePhone": mobile,
        "businessPhones": business,
        "homePhones": home,
        "birthday": birthday,
        "personalNotes": details.note.trim(),
    })
}

pub async fn update(token: &str, id: &str, details: &Details) -> Result<RemoteContact> {
    let url = format!("{}/me/contacts/{}", base(), enc(id));
    let changed = call(http().patch(url).bearer_auth(token).json(&body(details))).await?;
    to_remote(&changed).ok_or_else(|| Error::Config("Microsoft didn't return the contact".into()))
}

pub async fn create(token: &str, details: &Details, emails: &[String]) -> Result<RemoteContact> {
    let mut body = body(details);
    body["emailAddresses"] = json!(
        emails.iter().map(|e| json!({ "address": e, "name": details.name.trim() })).collect::<Vec<_>>()
    );
    let created = call(http().post(format!("{}/me/contacts", base())).bearer_auth(token).json(&body)).await?;
    to_remote(&created).ok_or_else(|| Error::Config("Microsoft didn't return the contact".into()))
}

/// A contact's photo (`None`: it has none).
pub async fn photo(token: &str, id: &str) -> Result<Option<(String, Vec<u8>)>> {
    let url = format!("{}/me/contacts/{}/photo/$value", base(), enc(id));
    let response = http()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| Error::Config(format!("Can't reach Microsoft: {e}")))?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let mime = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_owned();
    let data = response.bytes().await.map_err(|e| Error::Config(format!("Microsoft: {e}")))?;
    Ok(Some((mime, data.to_vec())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_outlook_contacts() {
        let c = json!({
            "id": "AAMkAD", "changeKey": "EQAAABYA", "displayName": "Tom Leroy", "givenName": "Tom",
            "surname": "Leroy", "emailAddresses": [{ "name": "Tom", "address": "Tom.Leroy@acme.fr" }],
            "mobilePhone": "06 11", "businessPhones": ["01 22"], "homePhones": [],
            "companyName": "Acme", "jobTitle": "Buyer", "birthday": "1604-03-04T11:59:00Z",
            "personalNotes": "Golf on Fridays"
        });
        let r = to_remote(&c).unwrap();
        assert_eq!(r.emails, ["tom.leroy@acme.fr"]);
        assert_eq!(r.details.birthday.as_deref(), Some("--03-04"), "1604 is Outlook's no year");
        assert_eq!(
            r.details.phones,
            [
                Phone { label: "mobile".into(), number: "06 11".into() },
                Phone { label: "work".into(), number: "01 22".into() }
            ]
        );

        // Back to Outlook's places: three "other" numbers fit as two at work.
        let mut d = r.details.clone();
        d.phones =
            ["a", "b", "c"].iter().map(|n| Phone { label: "other".into(), number: n.to_string() }).collect();
        d.birthday = Some("1990-01-02".into());
        let b = body(&d);
        assert_eq!(b["businessPhones"], json!(["a", "b"]));
        assert!(b["mobilePhone"].is_null());
        assert_eq!(b["birthday"], "1990-01-02T11:59:00Z");
        assert_eq!((b["givenName"].as_str(), b["surname"].as_str()), (Some("Tom"), Some("Leroy")));
        d.birthday = Some("--03-04".into());
        assert_eq!(body(&d)["birthday"], "1604-03-04T11:59:00Z");
    }
}
