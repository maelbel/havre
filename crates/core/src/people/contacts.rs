//! Your details about people: editing, favourites, robots, and merging two
//! addresses of the same person.
//!
//! A **contact** is created the first time you change something about an
//! address; until then the person is just that address. A contact can hold
//! several addresses (`contact_emails`, one row each); an address belongs to
//! one contact at most (it's the table's primary key, so SQLite enforces it).

use serde::{Deserialize, Serialize};
use sqlx::Row;

use super::{People, PersonDetail, kinds, now};
use crate::{Error, Result};

/// A phone number with what it is ("mobile", "work"…).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Phone {
    #[serde(default)]
    pub label: String,
    pub number: String,
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
                             '[]') AS emails
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
            })
            .collect())
    }

    async fn contact_of(&self, email: &str) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar("SELECT contact_id FROM contact_emails WHERE email = ?")
            .bind(email)
            .fetch_optional(&self.pool)
            .await?)
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

    /// Changes someone's details (see [`PersonUpdate`]); returns their page.
    pub async fn update(&self, email: &str, u: &PersonUpdate) -> Result<PersonDetail> {
        let email = address(email)?;
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

        let id = self.contact_for(&email).await?;
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
        self.person(&email).await
    }

    /// `other` is the same person as `email`: one contact, their addresses
    /// together. Details already set on `email` win; empty ones are taken
    /// from `other`'s contact, if it had one.
    pub async fn merge(&self, email: &str, other: &str) -> Result<PersonDetail> {
        let (email, other) = (address(email)?, address(other)?);
        if email == other {
            return Err(Error::Config("Pick another address than this one.".into()));
        }
        let keep = self.contact_for(&email).await?;
        let mut tx = self.pool.begin().await?;
        let next: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM contact_emails WHERE contact_id = ?",
        )
        .bind(keep)
        .fetch_one(&mut *tx)
        .await?;
        let theirs: Option<i64> = sqlx::query_scalar("SELECT contact_id FROM contact_emails WHERE email = ?")
            .bind(&other)
            .fetch_optional(&mut *tx)
            .await?;
        match theirs {
            Some(same) if same == keep => {} // already one person
            Some(from) => {
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
                sqlx::query("DELETE FROM contacts WHERE id = ?").bind(from).execute(&mut *tx).await?;
            }
            None => {
                sqlx::query("INSERT INTO contact_emails (email, contact_id, position) VALUES (?, ?, ?)")
                    .bind(&other)
                    .bind(keep)
                    .bind(next)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        self.person(&email).await
    }

    /// Takes `other` out of `email`'s contact: it's someone else again (or
    /// no one in particular). A contact keeps at least one address.
    pub async fn remove_address(&self, email: &str, other: &str) -> Result<PersonDetail> {
        let (email, other) = (address(email)?, address(other)?);
        if email == other {
            return Err(Error::Config("Open another of their addresses to remove this one.".into()));
        }
        let mine = self.contact_of(&email).await?;
        if mine.is_none() || mine != self.contact_of(&other).await? {
            return Err(Error::NotFound(format!("{other} on this person")));
        }
        sqlx::query("DELETE FROM contact_emails WHERE email = ?").bind(&other).execute(&self.pool).await?;
        // The address you're on becomes the main one if it wasn't.
        sqlx::query("UPDATE contact_emails SET position = CASE WHEN email = ? THEN 0 ELSE position + 1 END WHERE contact_id = ?")
            .bind(&email)
            .bind(mine)
            .execute(&self.pool)
            .await?;
        self.person(&email).await
    }
}
