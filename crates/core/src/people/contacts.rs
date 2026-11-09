//! Your details about people: editing, favourites, robots, and merging two
//! addresses of the same person.
//!
//! A **contact** is created the first time you change something about an
//! address; until then the person is just that address. A contact can hold
//! several addresses (`contact_emails`, one row each); an address belongs to
//! one contact at most (it's the table's primary key, so SQLite enforces it).
//!
//! A contact can also have **no address** (a phone-only contact from your
//! address book, someone you added by name): its key is then `~` and its
//! id, `~42` (see [`Key`]).

use serde::{Deserialize, Serialize};
use sqlx::Row;

use super::{People, PersonDetail, kinds, now};
use crate::{Error, Result};

/// A phone number with what it is ("mobile", "work"…).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Phone {
    #[serde(default)]
    pub label: String,
    pub number: String,
}

/// The details you keep about someone, the part that address books share
/// (synced with Google, Microsoft and CardDAV, read from `.vcf` files).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Details {
    pub name: String,
    pub company: String,
    pub role: String,
    pub phones: Vec<Phone>,
    /// `YYYY-MM-DD`, or `--MM-DD` without the year.
    pub birthday: Option<String>,
    pub note: String,
}

/// A stored contact (see the migration `0023_people.sql`).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ContactRow {
    pub id: i64,
    /// Main address first.
    pub emails: Vec<String>,
    pub name: String,
    pub company: String,
    pub role: String,
    pub phones: Vec<Phone>,
    pub birthday: Option<String>,
    pub note: String,
    pub favorite: bool,
    pub kind: Option<String>,
    /// Created by a sync (see `0024_contact_sync.sql`).
    pub from_sync: bool,
    /// The address book that last changed the details, and when.
    pub updated_from: Option<String>,
    pub updated_from_at: Option<i64>,
    /// The address books they're in.
    pub books: Vec<String>,
    /// One of those books has a photo of them (or may have: Outlook's).
    pub photo: bool,
}

impl ContactRow {
    pub fn details(&self) -> Details {
        Details {
            name: self.name.clone(),
            company: self.company.clone(),
            role: self.role.clone(),
            phones: self.phones.clone(),
            birthday: self.birthday.clone(),
            note: self.note.clone(),
        }
    }
}

/// How a person is named in a request: by one of their addresses, or by
/// their contact (`~42`) when they have none.
#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Address(String),
    Contact(i64),
}

impl Key {
    pub fn parse(key: &str) -> Result<Key> {
        match key.trim().strip_prefix('~') {
            Some(id) => id.parse().map(Key::Contact).map_err(|_| Error::NotFound(format!("person {key}"))),
            None => address(key).map(Key::Address),
        }
    }
}

/// A change to someone's details: only the fields that are present change.
/// `birthday` and `kind` can also be cleared (`null`), hence
/// `Option<Option<…>>`: missing, `null`, or a value.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PersonUpdate {
    pub name: Option<String>,
    pub company: Option<String>,
    pub role: Option<String>,
    pub note: Option<String>,
    pub phones: Option<Vec<Phone>>,
    #[serde(default, deserialize_with = "some")]
    pub birthday: Option<Option<String>>,
    pub favorite: Option<bool>,
    /// `person` or `automated`; `null`: guess from the address again.
    #[serde(default, deserialize_with = "some")]
    pub kind: Option<Option<String>>,
}

/// A field that is present (even `null`) becomes `Some(…)`; a missing one
/// stays `None` thanks to `#[serde(default)]`.
fn some<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

/// An address as Havre keys it: trimmed, lowercase, with an `@`.
fn address(email: &str) -> Result<String> {
    let email = email.trim().to_lowercase();
    if email.len() < 3 || !email.contains('@') || email.contains(char::is_whitespace) {
        return Err(Error::Config(format!("“{email}” isn't an email address.")));
    }
    Ok(email)
}

/// Text fields: trimmed, and not a novel.
fn short(text: &str, what: &str, max: usize) -> Result<String> {
    let text = text.trim();
    if text.chars().count() > max {
        return Err(Error::Config(format!("The {what} is too long (at most {max} characters).")));
    }
    Ok(text.to_owned())
}

impl People {
    /// Every contact, with its addresses.
    pub(super) async fn contacts(&self) -> Result<Vec<ContactRow>> {
        let rows = sqlx::query(
            "SELECT c.id, c.name, c.company, c.role, c.phones, c.birthday, c.note, c.favorite, c.kind,
                    COALESCE((SELECT json_group_array(email) FROM
                                (SELECT email FROM contact_emails WHERE contact_id = c.id ORDER BY position, email)),
                             '[]') AS emails,
                    c.from_sync, c.updated_from, c.updated_from_at,
                    (SELECT json_group_array(b.name) FROM contact_links l JOIN contact_books b ON b.id = l.book_id
                     WHERE l.contact_id = c.id) AS books,
                    EXISTS (SELECT 1 FROM contact_links l LEFT JOIN contact_photos p ON p.link_id = l.id
                            WHERE l.contact_id = c.id
                              AND ((l.photo_url IS NOT NULL AND p.link_id IS NULL) OR length(p.data) > 0)) AS photo
             FROM contacts c ORDER BY c.id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| ContactRow {
                id: r.get("id"),
                emails: serde_json::from_str(r.get("emails")).unwrap_or_default(),
                name: r.get("name"),
                company: r.get("company"),
                role: r.get("role"),
                phones: serde_json::from_str(r.get("phones")).unwrap_or_default(),
                birthday: r.get("birthday"),
                note: r.get("note"),
                favorite: r.get("favorite"),
                kind: r.get("kind"),
                from_sync: r.get("from_sync"),
                updated_from: r.get("updated_from"),
                updated_from_at: r.get("updated_from_at"),
                books: serde_json::from_str(r.get("books")).unwrap_or_default(),
                photo: r.get("photo"),
            })
            .collect())
    }

    pub(super) async fn contact_of(&self, email: &str) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar("SELECT contact_id FROM contact_emails WHERE email = ?")
            .bind(email)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// The contact for a key: the one named, or this address's (created,
    /// empty, when it has none).
    pub(super) async fn contact_for_key(&self, key: &Key) -> Result<i64> {
        match key {
            Key::Address(email) => self.contact_for(email).await,
            Key::Contact(id) => {
                let found: Option<i64> = sqlx::query_scalar("SELECT id FROM contacts WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await?;
                found.ok_or_else(|| Error::NotFound(format!("person ~{id}")))
            }
        }
    }

    /// The contact for this address, created (empty) when it has none.
    async fn contact_for(&self, email: &str) -> Result<i64> {
        if let Some(id) = self.contact_of(email).await? {
            return Ok(id);
        }
        let mut tx = self.pool.begin().await?;
        let id: i64 =
            sqlx::query_scalar("INSERT INTO contacts (created_at, updated_at) VALUES (?, ?) RETURNING id")
                .bind(now())
                .bind(now())
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query("INSERT INTO contact_emails (email, contact_id, position) VALUES (?, ?, 0)")
            .bind(email)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Someone new, by name (and an address if you know one); returns their
    /// page. An address already known gives that person, named.
    pub async fn create(&self, name: &str, email: Option<&str>) -> Result<PersonDetail> {
        let name = short(name, "name", 200)?;
        let email = email.map(str::trim).filter(|e| !e.is_empty()).map(address).transpose()?;
        if name.is_empty() && email.is_none() {
            return Err(Error::Config("Give a name or an address.".into()));
        }
        let id = match &email {
            Some(email) => self.contact_for(email).await?,
            None => {
                sqlx::query_scalar("INSERT INTO contacts (created_at, updated_at) VALUES (?, ?) RETURNING id")
                    .bind(now())
                    .bind(now())
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        if !name.is_empty() {
            sqlx::query("UPDATE contacts SET name = ?, updated_at = ? WHERE id = ?")
                .bind(&name)
                .bind(now())
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        self.person(&format!("~{id}")).await
    }

    /// Changes someone's details (see [`PersonUpdate`]); returns their page.
    pub async fn update(&self, key: &str, u: &PersonUpdate) -> Result<PersonDetail> {
        let key = Key::parse(key)?;
        // Check everything before writing anything.
        let name = u.name.as_deref().map(|t| short(t, "name", 200)).transpose()?;
        let company = u.company.as_deref().map(|t| short(t, "company", 200)).transpose()?;
        let role = u.role.as_deref().map(|t| short(t, "role", 200)).transpose()?;
        let note = u.note.as_deref().map(|t| short(t, "note", 4000)).transpose()?;
        let phones = match &u.phones {
            None => None,
            Some(list) => {
                let mut clean = Vec::new();
                for p in list {
                    let number = short(&p.number, "phone number", 40)?;
                    if number.is_empty() {
                        continue; // an empty row in the form
                    }
                    if !number.chars().any(|c| c.is_ascii_digit()) {
                        return Err(Error::Config(format!("“{number}” isn't a phone number.")));
                    }
                    clean.push(Phone { label: short(&p.label, "phone label", 30)?, number });
                }
                if clean.len() > 10 {
                    return Err(Error::Config("At most 10 phone numbers.".into()));
                }
                Some(serde_json::to_string(&clean).unwrap_or_default())
            }
        };
        let birthday = match &u.birthday {
            Some(Some(b)) if !b.trim().is_empty() => {
                if kinds::parse_birthday(b).is_none() {
                    return Err(Error::Config(format!(
                        "“{b}” isn't a date (YYYY-MM-DD, or --MM-DD without the year)."
                    )));
                }
                Some(Some(b.trim().to_owned()))
            }
            Some(_) => Some(None),
            None => None,
        };
        if let Some(Some(kind)) = &u.kind
            && !matches!(kind.as_str(), "person" | "automated")
        {
            return Err(Error::Config("A person is either “person” or “automated”.".into()));
        }

        let id = self.contact_for_key(&key).await?;
        // One `UPDATE`, each column kept unless its field was sent: `CASE WHEN ? THEN ? ELSE col END`
        // (a flag says "sent", since the value itself may be NULL on purpose).
        sqlx::query(
            "UPDATE contacts SET
                name = COALESCE(?, name), company = COALESCE(?, company), role = COALESCE(?, role),
                note = COALESCE(?, note), phones = COALESCE(?, phones), favorite = COALESCE(?, favorite),
                birthday = CASE WHEN ? THEN ? ELSE birthday END,
                kind = CASE WHEN ? THEN ? ELSE kind END,
                updated_at = ?
             WHERE id = ?",
        )
        .bind(name)
        .bind(company)
        .bind(role)
        .bind(note)
        .bind(phones)
        .bind(u.favorite)
        .bind(birthday.is_some())
        .bind(birthday.flatten())
        .bind(u.kind.is_some())
        .bind(u.kind.clone().flatten())
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.person(&format!("~{id}")).await
    }

    /// `other` is the same person as `key`: one contact, their addresses
    /// (and address books) together. Details already set on `key` win; empty
    /// ones are taken from `other`'s contact, if it had one. `other` can be
    /// a new address: it's added to them.
    pub async fn merge(&self, key: &str, other: &str) -> Result<PersonDetail> {
        let (key, other) = (Key::parse(key)?, Key::parse(other)?);
        if key == other {
            return Err(Error::Config("Pick another address than this one.".into()));
        }
        let keep = self.contact_for_key(&key).await?;
        let mut tx = self.pool.begin().await?;
        let next: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM contact_emails WHERE contact_id = ?",
        )
        .bind(keep)
        .fetch_one(&mut *tx)
        .await?;
        // Keys tasks and notes may know them by, before: `~keep`, and `~from` below.
        let mut old_keys = vec![format!("~{keep}")];
        let theirs: Option<i64> = match &other {
            Key::Address(email) => {
                sqlx::query_scalar("SELECT contact_id FROM contact_emails WHERE email = ?")
                    .bind(email)
                    .fetch_optional(&mut *tx)
                    .await?
            }
            Key::Contact(id) => Some(*id),
        };
        match theirs {
            Some(same) if same == keep => {} // already one person
            Some(from) => {
                old_keys.push(format!("~{from}"));
                // Fill what's empty here from there, then move its addresses over and drop it.
                sqlx::query(
                    "UPDATE contacts SET
                        name = CASE WHEN name = '' THEN (SELECT name FROM contacts WHERE id = ?2) ELSE name END,
                        company = CASE WHEN company = '' THEN (SELECT company FROM contacts WHERE id = ?2) ELSE company END,
                        role = CASE WHEN role = '' THEN (SELECT role FROM contacts WHERE id = ?2) ELSE role END,
                        note = CASE WHEN note = '' THEN (SELECT note FROM contacts WHERE id = ?2) ELSE note END,
                        phones = CASE WHEN phones = '[]' THEN (SELECT phones FROM contacts WHERE id = ?2) ELSE phones END,
                        birthday = COALESCE(birthday, (SELECT birthday FROM contacts WHERE id = ?2)),
                        kind = COALESCE(kind, (SELECT kind FROM contacts WHERE id = ?2)),
                        favorite = favorite OR (SELECT favorite FROM contacts WHERE id = ?2),
                        -- Made by you if either was: then a sync never removes it.
                        from_sync = from_sync AND (SELECT from_sync FROM contacts WHERE id = ?2),
                        updated_at = ?3
                     WHERE id = ?1",
                )
                .bind(keep)
                .bind(from)
                .bind(now())
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE contact_emails SET contact_id = ?, position = position + ? WHERE contact_id = ?",
                )
                .bind(keep)
                .bind(next)
                .bind(from)
                .execute(&mut *tx)
                .await?;
                // Its address books follow (the same person in Google and in iCloud).
                sqlx::query("UPDATE contact_links SET contact_id = ? WHERE contact_id = ?")
                    .bind(keep)
                    .bind(from)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM contacts WHERE id = ?").bind(from).execute(&mut *tx).await?;
            }
            None => {
                let Key::Address(email) = &other else {
                    return Err(Error::NotFound("this person".into()));
                };
                sqlx::query("INSERT INTO contact_emails (email, contact_id, position) VALUES (?, ?, ?)")
                    .bind(email)
                    .bind(keep)
                    .bind(next)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        // Their key now: the main address, else `~keep`. Tasks and notes
        // about them under an older `~…` key follow.
        let main: Option<String> = sqlx::query_scalar(
            "SELECT email FROM contact_emails WHERE contact_id = ? ORDER BY position, email LIMIT 1",
        )
        .bind(keep)
        .fetch_optional(&mut *tx)
        .await?;
        let key = main.unwrap_or_else(|| format!("~{keep}"));
        for old in old_keys.iter().filter(|old| **old != key) {
            sqlx::query("UPDATE tasks SET person = ? WHERE person = ?")
                .bind(&key)
                .bind(old)
                .execute(&mut *tx)
                .await?;
            // `OR IGNORE`: a note already about them under the new key keeps that row.
            sqlx::query("UPDATE OR IGNORE note_people SET email = ? WHERE email = ?")
                .bind(&key)
                .bind(old)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM note_people WHERE email = ?").bind(old).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        self.person(&format!("~{keep}")).await
    }

    /// Takes `other` out of this person's contact: it's someone else again
    /// (or no one in particular). The address you're on becomes the main one.
    pub async fn remove_address(&self, key: &str, other: &str) -> Result<PersonDetail> {
        let (key, other) = (Key::parse(key)?, address(other)?);
        let mine = match &key {
            Key::Address(email) if *email == other => {
                return Err(Error::Config("Open another of their addresses to remove this one.".into()));
            }
            Key::Address(email) => self.contact_of(email).await?,
            Key::Contact(id) => Some(*id),
        };
        let Some(mine) = mine else {
            return Err(Error::NotFound(format!("{other} on this person")));
        };
        if self.contact_of(&other).await? != Some(mine) {
            return Err(Error::NotFound(format!("{other} on this person")));
        }
        sqlx::query("DELETE FROM contact_emails WHERE email = ?").bind(&other).execute(&self.pool).await?;
        if let Key::Address(email) = &key {
            sqlx::query(
                "UPDATE contact_emails SET position = CASE WHEN email = ? THEN 0 ELSE position + 1 END
                 WHERE contact_id = ?",
            )
            .bind(email)
            .bind(mine)
            .execute(&self.pool)
            .await?;
        }
        self.person(&format!("~{mine}")).await
    }
}
