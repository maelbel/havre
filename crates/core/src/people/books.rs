//! Address books and the links between their contacts and Havre's: what the
//! sync ([`super::sync`]) stores, and how a contact from an address book is
//! merged into Havre's, field by field. No network here, so it's all tested
//! with a database in memory.
//!
//! ## Merging, field by field
//!
//! Each link keeps the details **as last synced** (`synced`): what both
//! sides agreed on. When an address book sends a contact, each field is one
//! of:
//!
//! | address book vs synced | Havre vs synced | result                              |
//! |------------------------|-----------------|-------------------------------------|
//! | same                   | same            | nothing to do                       |
//! | changed                | same            | Havre takes the address book's      |
//! | same                   | changed         | Havre's goes to the address book    |
//! | changed                | changed         | the address book wins               |
//!
//! It's a *three-way* merge: with only the two sides, you can't tell who
//! changed a field. A new link starts from empty details, so on the first
//! sync the address book fills Havre's empty fields (and wins where both
//! have something), and Havre's details fill the address book's gaps.

use serde::Serialize;
use sqlx::Row;

use super::{
    Details, People, now,
    sync::Changes,
    vcard::{self, Card, PhotoRef},
};
use crate::{Error, Result};

/// An address book, for the settings and the sync.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Book {
    pub id: i64,
    /// `google`, `microsoft` or `carddav`.
    pub source: String,
    pub account: Option<i64>,
    pub caldav: Option<i64>,
    #[serde(skip)]
    pub remote_id: String,
    pub name: String,
    pub read_only: bool,
    #[serde(skip)]
    pub sync_token: Option<String>,
    pub synced_at: Option<i64>,
    pub error: Option<String>,
    /// How many of its contacts are linked.
    pub contacts: i64,
}

/// An address book as a provider lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteBook {
    pub remote_id: String,
    pub name: String,
    pub read_only: bool,
}

/// A contact as an address book sends it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemoteContact {
    pub remote_id: String,
    pub href: Option<String>,
    pub etag: Option<String>,
    /// CardDAV: the vCard text.
    pub raw: Option<String>,
    pub details: Details,
    pub emails: Vec<String>,
    pub photo: Option<PhotoRef>,
}

/// A Havre contact ↔ an address book's contact.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub id: i64,
    pub contact: i64,
    pub book: i64,
    pub remote_id: String,
    pub href: Option<String>,
    pub etag: Option<String>,
    pub raw: Option<String>,
    /// The details as last synced.
    pub synced: Details,
    pub photo_url: Option<String>,
}

/// What a `.vcf` import did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ImportReport {
    /// New people.
    pub added: usize,
    /// People who already were in Havre, given what they lacked.
    pub updated: usize,
    /// Cards with nothing to keep (no name, no address), and groups.
    pub skipped: usize,
}

/// A photo to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Photo {
    pub mime: String,
    pub data: Vec<u8>,
}

/// Where a contact's photo can come from: a link, and what's stored for it.
#[derive(Debug, Clone, PartialEq)]
pub struct PhotoSource {
    pub link: i64,
    pub source: String,
    pub account: Option<i64>,
    pub remote_id: String,
    pub photo_url: Option<String>,
    /// `Some(empty)`: fetched, there was none.
    pub stored: Option<Photo>,
    pub fetched_at: Option<i64>,
}

fn link_from(r: &sqlx::sqlite::SqliteRow) -> Link {
    Link {
        id: r.get("id"),
        contact: r.get("contact_id"),
        book: r.get("book_id"),
        remote_id: r.get("remote_id"),
        href: r.get("href"),
        etag: r.get("etag"),
        raw: r.get("raw"),
        synced: serde_json::from_str(r.get("synced")).unwrap_or_default(),
        photo_url: r.get("photo_url"),
    }
}

const LINK_COLUMNS: &str = "id, contact_id, book_id, remote_id, href, etag, raw, synced, photo_url";

/// `IS` compares like `=` but also matches two NULLs (an account's books have
/// no CalDAV server, and the reverse).
const SAME_SOURCE: &str = "source = ? AND account_id IS ? AND caldav_id IS ?";

impl People {
    // ---- Sources and books ---------------------------------------------------------------

    /// Is contacts sync on for this account (`account:3`) or server (`caldav:2`)?
    pub async fn source_enabled(&self, key: &str) -> Result<bool> {
        let on: Option<bool> = sqlx::query_scalar("SELECT enabled FROM contact_sources WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(on.unwrap_or(false))
    }

    pub async fn set_source_enabled(&self, key: &str, enabled: bool) -> Result<()> {
        sqlx::query(
            "INSERT INTO contact_sources (key, enabled) VALUES (?, ?)
             ON CONFLICT (key) DO UPDATE SET enabled = excluded.enabled",
        )
        .bind(key)
        .bind(enabled)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn books(&self) -> Result<Vec<Book>> {
        let rows = sqlx::query(
            "SELECT b.*, (SELECT COUNT(*) FROM contact_links l WHERE l.book_id = b.id) AS contacts
             FROM contact_books b ORDER BY b.id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| Book {
                id: r.get("id"),
                source: r.get("source"),
                account: r.get("account_id"),
                caldav: r.get("caldav_id"),
                remote_id: r.get("remote_id"),
                name: r.get("name"),
                read_only: r.get("read_only"),
                sync_token: r.get("sync_token"),
                synced_at: r.get("synced_at"),
                error: r.get("error"),
                contacts: r.get("contacts"),
            })
            .collect())
    }

    pub async fn book(&self, id: i64) -> Result<Book> {
        self.books()
            .await?
            .into_iter()
            .find(|b| b.id == id)
            .ok_or_else(|| Error::NotFound(format!("address book {id}")))
    }

    /// A source's address books as it lists them now: known ones keep their
    /// id (and links), new ones are added, gone ones removed.
    pub async fn replace_books(
        &self,
        source: &str,
        account: Option<i64>,
        caldav: Option<i64>,
        books: &[RemoteBook],
    ) -> Result<()> {
        let sql = format!("SELECT id, remote_id FROM contact_books WHERE {SAME_SOURCE}");
        let known: Vec<(i64, String)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(source)
            .bind(account)
            .bind(caldav)
            .fetch_all(&self.pool)
            .await?;
        for book in books {
            match known.iter().find(|(_, remote)| *remote == book.remote_id) {
                Some((id, _)) => {
                    sqlx::query(
                        "UPDATE contact_books SET name = ?, read_only = ?, error = NULL WHERE id = ?",
                    )
                    .bind(&book.name)
                    .bind(book.read_only)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                }
                None => {
                    sqlx::query(
                        "INSERT INTO contact_books (source, account_id, caldav_id, remote_id, name, read_only)
                         VALUES (?, ?, ?, ?, ?, ?)",
                    )
                    .bind(source)
                    .bind(account)
                    .bind(caldav)
                    .bind(&book.remote_id)
                    .bind(&book.name)
                    .bind(book.read_only)
                    .execute(&self.pool)
                    .await?;
                }
            }
        }
        for (id, remote) in &known {
            if !books.iter().any(|b| b.remote_id == *remote) {
                self.remove_book(*id).await?;
            }
        }
        Ok(())
    }

    /// A source switched off (or its account removed): its books go, and the
    /// people that only came from them.
    pub async fn remove_books(&self, source: &str, account: Option<i64>, caldav: Option<i64>) -> Result<()> {
        let sql = format!("SELECT id FROM contact_books WHERE {SAME_SOURCE}");
        let ids: Vec<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(source)
            .bind(account)
            .bind(caldav)
            .fetch_all(&self.pool)
            .await?;
        for id in ids {
            self.remove_book(id).await?;
        }
        Ok(())
    }

    async fn remove_book(&self, id: i64) -> Result<()> {
        // `ON DELETE CASCADE` removes the links (and their photos).
        sqlx::query("DELETE FROM contact_books WHERE id = ?").bind(id).execute(&self.pool).await?;
        self.prune().await
    }

    /// The error of a source's books when it couldn't list them.
    pub async fn set_source_error(
        &self,
        source: &str,
        account: Option<i64>,
        caldav: Option<i64>,
        error: &str,
    ) -> Result<()> {
        let sql = format!("UPDATE contact_books SET error = ? WHERE {SAME_SOURCE}");
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(error)
            .bind(source)
            .bind(account)
            .bind(caldav)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// After syncing a book: where to start next time, or what went wrong.
    pub async fn set_book_synced(&self, id: i64, token: Option<&str>, error: Option<&str>) -> Result<()> {
        sqlx::query(
            "UPDATE contact_books SET sync_token = COALESCE(?, sync_token), error = ?,
                    synced_at = CASE WHEN ? IS NULL THEN ? ELSE synced_at END
             WHERE id = ?",
        )
        .bind(token)
        .bind(error)
        .bind(error)
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Forgets where to start: the next sync reads the whole book.
    pub async fn reset_book_token(&self, id: i64) -> Result<()> {
        sqlx::query("UPDATE contact_books SET sync_token = NULL WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn default_book(&self) -> Result<Option<i64>> {
        Ok(sqlx::query_scalar("SELECT default_book FROM contact_settings WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?
            .flatten())
    }

    pub async fn set_default_book(&self, book: Option<i64>) -> Result<()> {
        if let Some(id) = book
            && self.book(id).await?.read_only
        {
            return Err(Error::Config("This address book is read-only: pick another one.".into()));
        }
        sqlx::query("UPDATE contact_settings SET default_book = ? WHERE id = 1")
            .bind(book)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---- Links -----------------------------------------------------------------------------

    pub async fn links(&self, book: i64) -> Result<Vec<Link>> {
        let sql = format!("SELECT {LINK_COLUMNS} FROM contact_links WHERE book_id = ? ORDER BY id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(book).fetch_all(&self.pool).await?;
        Ok(rows.iter().map(link_from).collect())
    }

    pub async fn links_of(&self, contact: i64) -> Result<Vec<Link>> {
        let sql = format!("SELECT {LINK_COLUMNS} FROM contact_links WHERE contact_id = ? ORDER BY id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(contact).fetch_all(&self.pool).await?;
        Ok(rows.iter().map(link_from).collect())
    }

    /// A contact's details as Havre has them now.
    pub async fn details_of(&self, contact: i64) -> Result<(Details, Vec<String>)> {
        let row = self
            .contacts()
            .await?
            .into_iter()
            .find(|c| c.id == contact)
            .ok_or_else(|| Error::NotFound(format!("person ~{contact}")))?;
        Ok((row.details(), row.emails))
    }

    /// Merges a contact sent by an address book (see the module's table).
    /// Returns the link, and whether Havre has something to send back.
    pub async fn apply_remote(&self, book: &Book, remote: &RemoteContact) -> Result<(i64, bool)> {
        let mut tx = self.pool.begin().await?;
        let sql = format!("SELECT {LINK_COLUMNS} FROM contact_links WHERE book_id = ? AND remote_id = ?");
        let link = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(book.id)
            .bind(&remote.remote_id)
            .fetch_optional(&mut *tx)
            .await?
            .map(|r| link_from(&r));

        // Who it is in Havre: the linked contact, else whoever has one of its
        // addresses, else someone new.
        let contact = match &link {
            Some(link) => link.contact,
            None => {
                let mut found = None;
                for email in &remote.emails {
                    found = sqlx::query_scalar("SELECT contact_id FROM contact_emails WHERE email = ?")
                        .bind(email)
                        .fetch_optional(&mut *tx)
                        .await?;
                    if found.is_some() {
                        break;
                    }
                }
                match found {
                    Some(id) => id,
                    None => {
                        sqlx::query_scalar(
                            "INSERT INTO contacts (from_sync, created_at, updated_at) VALUES (1, ?, ?) RETURNING id",
                        )
                        .bind(now())
                        .bind(now())
                        .fetch_one(&mut *tx)
                        .await?
                    }
                }
            }
        };
        // Its addresses nobody has yet join this person (one taken by
        // someone else stays theirs: Havre doesn't merge people on its own).
        for email in &remote.emails {
            sqlx::query(
                "INSERT INTO contact_emails (email, contact_id, position)
                 VALUES (?1, ?2, (SELECT COALESCE(MAX(position) + 1, 0) FROM contact_emails WHERE contact_id = ?2))
                 ON CONFLICT (email) DO NOTHING",
            )
            .bind(email)
            .bind(contact)
            .execute(&mut *tx)
            .await?;
        }

        let local = read_details(&mut tx, contact).await?;
        let base = link.as_ref().map(|l| l.synced.clone()).unwrap_or_default();
        let merged = merge_fields(&base, &local, &remote.details);
        if merged != local {
            write_details(&mut tx, contact, &merged, Some(&book.name)).await?;
        }

        let photo_url = match &remote.photo {
            Some(PhotoRef::Url(url)) => Some(url.clone()),
            _ => None,
        };
        let synced = serde_json::to_string(&remote.details).unwrap_or_default();
        let link_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_links (contact_id, book_id, remote_id, href, etag, raw, synced, photo_url)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (book_id, remote_id) DO UPDATE SET
                href = excluded.href, etag = excluded.etag, raw = excluded.raw, synced = excluded.synced,
                photo_url = excluded.photo_url
             RETURNING id",
        )
        .bind(contact)
        .bind(book.id)
        .bind(&remote.remote_id)
        .bind(&remote.href)
        .bind(&remote.etag)
        .bind(&remote.raw)
        .bind(&synced)
        .bind(&photo_url)
        .fetch_one(&mut *tx)
        .await?;
        match &remote.photo {
            Some(PhotoRef::Inline { mime, data }) => {
                save_photo(&mut tx, link_id, mime, data).await?;
            }
            // A new address (or none): fetch again when asked.
            _ if link.as_ref().and_then(|l| l.photo_url.as_ref()) != photo_url.as_ref()
                || photo_url.is_none() =>
            {
                sqlx::query("DELETE FROM contact_photos WHERE link_id = ?")
                    .bind(link_id)
                    .execute(&mut *tx)
                    .await?;
            }
            _ => {}
        }
        tx.commit().await?;
        Ok((link_id, merged != remote.details && !book.read_only))
    }

    /// Applies what an address book says changed: each contact whose ETag
    /// differs from the link's is merged, deleted ones (or, for a full
    /// listing, missing ones) are unlinked. `links`: the book's links before.
    pub async fn apply_changes(&self, book: &Book, links: &[Link], changes: &Changes) -> Result<()> {
        let etags: std::collections::HashMap<&str, Option<&str>> =
            links.iter().map(|l| (l.remote_id.as_str(), l.etag.as_deref())).collect();
        for contact in &changes.contacts {
            // Same ETag: nothing changed there since the last sync.
            let unchanged = contact.etag.is_some()
                && etags.get(contact.remote_id.as_str()) == Some(&contact.etag.as_deref());
            if !unchanged {
                self.apply_remote(book, contact).await?;
            }
        }
        for link in links {
            let deleted = changes.deleted.contains(&link.remote_id);
            let missing = changes.full && !changes.contacts.iter().any(|c| c.remote_id == link.remote_id);
            if deleted || missing {
                self.remove_link(link.id).await?;
            }
        }
        Ok(())
    }

    /// A new contact in an address book for someone in Havre (created there
    /// by the sync, from your edit).
    pub async fn link_new(&self, contact: i64, book: &Book, remote: &RemoteContact) -> Result<i64> {
        let synced = serde_json::to_string(&remote.details).unwrap_or_default();
        Ok(sqlx::query_scalar(
            "INSERT INTO contact_links (contact_id, book_id, remote_id, href, etag, raw, synced)
             VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(contact)
        .bind(book.id)
        .bind(&remote.remote_id)
        .bind(&remote.href)
        .bind(&remote.etag)
        .bind(&remote.raw)
        .bind(synced)
        .fetch_one(&self.pool)
        .await?)
    }

    /// After Havre's details were written to an address book: both sides now
    /// agree on what it answered (which Havre takes too, should the address
    /// book have shaped it its own way: otherwise each sync would send it
    /// again).
    pub async fn pushed(&self, link: &Link, remote: &RemoteContact) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE contact_links SET etag = ?, raw = COALESCE(?, raw), href = COALESCE(?, href), synced = ? WHERE id = ?")
            .bind(&remote.etag)
            .bind(&remote.raw)
            .bind(&remote.href)
            .bind(serde_json::to_string(&remote.details).unwrap_or_default())
            .bind(link.id)
            .execute(&mut *tx)
            .await?;
        if read_details(&mut tx, link.contact).await? != remote.details {
            write_details(&mut tx, link.contact, &remote.details, None).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// The links whose contact changed in Havre since the last sync: to send.
    pub async fn pending(&self, book: i64) -> Result<Vec<(Link, Details)>> {
        // Everyone's details at once (not one query per link).
        let details: std::collections::HashMap<i64, Details> =
            self.contacts().await?.into_iter().map(|c| (c.id, c.details())).collect();
        let mut out = Vec::new();
        for link in self.links(book).await? {
            if let Some(d) = details.get(&link.contact)
                && *d != link.synced
            {
                out.push((link, d.clone()));
            }
        }
        Ok(out)
    }

    /// The contact left the address book: the link goes, and the person too
    /// if they only came from address books (see [`People::prune`]).
    pub async fn remove_link(&self, link: i64) -> Result<()> {
        sqlx::query("DELETE FROM contact_links WHERE id = ?").bind(link).execute(&self.pool).await?;
        self.prune().await
    }

    /// Removes the people a sync created who are in no address book any
    /// more, unless you made them a favourite or said what they are.
    async fn prune(&self) -> Result<()> {
        sqlx::query(
            "DELETE FROM contacts WHERE from_sync = 1 AND favorite = 0 AND kind IS NULL
               AND NOT EXISTS (SELECT 1 FROM contact_links l WHERE l.contact_id = contacts.id)",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ---- Photos ------------------------------------------------------------------------------

    /// Where this person's photo can come from (their address books, in
    /// the order they were linked).
    pub async fn photo_sources(&self, contact: i64) -> Result<Vec<PhotoSource>> {
        let rows = sqlx::query(
            "SELECT l.id, l.remote_id, l.photo_url, b.source, b.account_id, p.mime, p.data, p.fetched_at
             FROM contact_links l JOIN contact_books b ON b.id = l.book_id
             LEFT JOIN contact_photos p ON p.link_id = l.id
             WHERE l.contact_id = ? ORDER BY l.id",
        )
        .bind(contact)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| PhotoSource {
                link: r.get("id"),
                source: r.get("source"),
                account: r.get("account_id"),
                remote_id: r.get("remote_id"),
                photo_url: r.get("photo_url"),
                stored: r.get::<Option<String>, _>("mime").map(|mime| Photo {
                    mime,
                    data: r.get::<Option<Vec<u8>>, _>("data").unwrap_or_default(),
                }),
                fetched_at: r.get("fetched_at"),
            })
            .collect())
    }

    /// Keeps a fetched photo (empty `data`: there was none).
    pub async fn store_photo(&self, link: i64, mime: &str, data: &[u8]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        save_photo(&mut tx, link, mime, data).await?;
        tx.commit().await?;
        Ok(())
    }

    // ---- .vcf files ------------------------------------------------------------------------

    /// Reads a `.vcf` file into Havre only (no address book is changed).
    /// Someone already here (by address, or by name for a card without
    /// one) gets the details they lacked; your details are never replaced.
    pub async fn import_vcf(&self, text: &str) -> Result<ImportReport> {
        let mut report = ImportReport::default();
        let contacts = self.contacts().await?;
        for card in vcard::parse(text) {
            let details = card.details();
            let emails = card.emails();
            if card.is_group() || (emails.is_empty() && details.name.trim().is_empty()) {
                report.skipped += 1;
                continue;
            }
            let mut existing = None;
            for email in &emails {
                if let Some(id) = self.contact_of(email).await? {
                    existing = Some(id);
                    break;
                }
            }
            if existing.is_none() && emails.is_empty() {
                let name = details.name.trim().to_lowercase();
                existing = contacts
                    .iter()
                    .find(|c| c.emails.is_empty() && c.name.trim().to_lowercase() == name)
                    .map(|c| c.id);
            }
            let mut tx = self.pool.begin().await?;
            let id = match existing {
                Some(id) => {
                    report.updated += 1;
                    id
                }
                None => {
                    report.added += 1;
                    sqlx::query_scalar(
                        "INSERT INTO contacts (created_at, updated_at) VALUES (?, ?) RETURNING id",
                    )
                    .bind(now())
                    .bind(now())
                    .fetch_one(&mut *tx)
                    .await?
                }
            };
            for email in &emails {
                sqlx::query(
                    "INSERT INTO contact_emails (email, contact_id, position)
                     VALUES (?1, ?2, (SELECT COALESCE(MAX(position) + 1, 0) FROM contact_emails WHERE contact_id = ?2))
                     ON CONFLICT (email) DO NOTHING",
                )
                .bind(email)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            }
            let local = read_details(&mut tx, id).await?;
            let filled = fill_empty(&local, &details);
            if filled != local {
                write_details(&mut tx, id, &filled, None).await?;
            }
            tx.commit().await?;
        }
        Ok(report)
    }

    /// Everyone you added details to (or who came from an address book), as
    /// one `.vcf` file.
    pub async fn export_vcf(&self) -> Result<String> {
        let cards: Vec<Card> = self
            .contacts()
            .await?
            .iter()
            .map(|c| Card::new(&format!("havre-{}", c.id), &c.details(), &c.emails))
            .collect();
        Ok(vcard::to_file(&cards))
    }
}

/// The three-way merge of one contact (see the module's table).
fn merge_fields(base: &Details, local: &Details, remote: &Details) -> Details {
    // For each field: the address book's when it changed there, else Havre's.
    fn pick<T: PartialEq + Clone>(base: &T, local: &T, remote: &T) -> T {
        if remote != base { remote.clone() } else { local.clone() }
    }
    Details {
        name: pick(&base.name, &local.name, &remote.name),
        company: pick(&base.company, &local.company, &remote.company),
        role: pick(&base.role, &local.role, &remote.role),
        phones: pick(&base.phones, &local.phones, &remote.phones),
        birthday: pick(&base.birthday, &local.birthday, &remote.birthday),
        note: pick(&base.note, &local.note, &remote.note),
    }
}

/// An import: what's empty in Havre is taken from the file.
fn fill_empty(local: &Details, file: &Details) -> Details {
    let text = |l: &String, f: &String| if l.trim().is_empty() { f.clone() } else { l.clone() };
    Details {
        name: text(&local.name, &file.name),
        company: text(&local.company, &file.company),
        role: text(&local.role, &file.role),
        phones: if local.phones.is_empty() { file.phones.clone() } else { local.phones.clone() },
        birthday: local.birthday.clone().or_else(|| file.birthday.clone()),
        note: text(&local.note, &file.note),
    }
}

async fn read_details(tx: &mut sqlx::SqliteConnection, contact: i64) -> Result<Details> {
    let r = sqlx::query("SELECT name, company, role, phones, birthday, note FROM contacts WHERE id = ?")
        .bind(contact)
        .fetch_one(&mut *tx)
        .await?;
    Ok(Details {
        name: r.get("name"),
        company: r.get("company"),
        role: r.get("role"),
        phones: serde_json::from_str(r.get("phones")).unwrap_or_default(),
        birthday: r.get("birthday"),
        note: r.get("note"),
    })
}

/// Writes the details; `from`: the address book they came from (shown on
/// the person's page).
async fn write_details(
    tx: &mut sqlx::SqliteConnection,
    contact: i64,
    d: &Details,
    from: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE contacts SET name = ?, company = ?, role = ?, phones = ?, birthday = ?, note = ?, updated_at = ?,
                updated_from = COALESCE(?, updated_from),
                updated_from_at = CASE WHEN ? IS NULL THEN updated_from_at ELSE ? END
         WHERE id = ?",
    )
    .bind(&d.name)
    .bind(&d.company)
    .bind(&d.role)
    .bind(serde_json::to_string(&d.phones).unwrap_or_default())
    .bind(&d.birthday)
    .bind(&d.note)
    .bind(now())
    .bind(from)
    .bind(from)
    .bind(now())
    .bind(contact)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Photos bigger than this aren't kept (they're shown at 96 pixels).
const MAX_PHOTO: usize = 512 * 1024;

async fn save_photo(tx: &mut sqlx::SqliteConnection, link: i64, mime: &str, data: &[u8]) -> Result<()> {
    let (mime, data) =
        if data.len() > MAX_PHOTO || !mime.starts_with("image/") { ("", &[][..]) } else { (mime, data) };
    sqlx::query(
        "INSERT INTO contact_photos (link_id, mime, data, fetched_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (link_id) DO UPDATE SET mime = excluded.mime, data = excluded.data, fetched_at = excluded.fetched_at",
    )
    .bind(link)
    .bind(mime)
    .bind(data)
    .bind(now())
    .execute(&mut *tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{people::Phone, store::Database};

    fn remote(id: &str, name: &str, emails: &[&str]) -> RemoteContact {
        RemoteContact {
            remote_id: id.into(),
            etag: Some("1".into()),
            details: Details { name: name.into(), ..Details::default() },
            emails: emails.iter().map(|e| e.to_string()).collect(),
            ..RemoteContact::default()
        }
    }

    async fn google_book(people: &People) -> Book {
        people
            .replace_books(
                "google",
                None,
                None,
                &[RemoteBook {
                    remote_id: "people/me".into(),
                    name: "Google Contacts".into(),
                    read_only: false,
                }],
            )
            .await
            .unwrap();
        people.books().await.unwrap().remove(0)
    }

    #[tokio::test]
    async fn address_books_merge_field_by_field() {
        let db = Database::in_memory().await.unwrap();
        let people = db.people();
        // Claire is in Havre with your note, and a company Google doesn't know yet.
        people
            .update(
                "claire@acme.fr",
                &crate::people::PersonUpdate {
                    note: Some("Prefers calls".into()),
                    company: Some("Acme".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let book = google_book(&people).await;

        // First sync: Google has her name, a phone and another note.
        let mut claire = remote("people/c1", "Claire Martin", &["claire@acme.fr", "claire@gmail.com"]);
        claire.details.phones = vec![Phone { label: "mobile".into(), number: "06".into() }];
        claire.details.note = "From Google".into();
        let (link, push) = people.apply_remote(&book, &claire).await.unwrap();
        let page = people.person("claire@gmail.com").await.unwrap();
        assert_eq!(page.person.email, "claire@acme.fr", "one person, both addresses");
        assert_eq!(page.person.name, "Claire Martin");
        assert_eq!(page.note, "From Google", "the address book wins where both had something");
        assert_eq!(page.person.company, "Acme", "Havre fills the address book's gaps…");
        assert!(push, "…so there's something to send");
        assert_eq!(page.books, ["Google Contacts"]);
        assert_eq!(page.updated_from.as_deref(), Some("Google Contacts"));
        let pending = people.pending(book.id).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.company, "Acme");

        // Sent: Google answers with the details as it keeps them.
        let mut sent = claire.clone();
        sent.details.company = "Acme".into();
        sent.etag = Some("2".into());
        people.pushed(&pending[0].0, &sent).await.unwrap();
        assert!(people.pending(book.id).await.unwrap().is_empty());

        // You change her role in Havre; meanwhile her phone changed in Google.
        people
            .update(
                "claire@acme.fr",
                &crate::people::PersonUpdate { role: Some("Lead".into()), ..Default::default() },
            )
            .await
            .unwrap();
        let mut changed = sent.clone();
        changed.details.phones[0].number = "07".into();
        let (same_link, push) = people.apply_remote(&book, &changed).await.unwrap();
        assert_eq!(same_link, link);
        assert!(push, "the role still has to go");
        let page = people.person("claire@acme.fr").await.unwrap();
        assert_eq!((page.person.phones[0].number.as_str(), page.person.role.as_str()), ("07", "Lead"));

        // Both change the note: the address book wins.
        people
            .update(
                "claire@acme.fr",
                &crate::people::PersonUpdate { note: Some("Mine".into()), ..Default::default() },
            )
            .await
            .unwrap();
        changed.details.note = "Theirs".into();
        people.apply_remote(&book, &changed).await.unwrap();
        assert_eq!(people.person("claire@acme.fr").await.unwrap().note, "Theirs");
    }

    #[tokio::test]
    async fn people_without_an_address_and_leaving_a_book() {
        let db = Database::in_memory().await.unwrap();
        let people = db.people();
        let book = google_book(&people).await;

        // A plumber with only a phone: a person of their own, keyed by contact.
        let mut plumber = remote("people/p1", "Marc (plumber)", &[]);
        plumber.details.phones = vec![Phone { label: "mobile".into(), number: "06 00".into() }];
        let (link, push) = people.apply_remote(&book, &plumber).await.unwrap();
        assert!(!push);
        let list = people.list(&crate::people::ListQuery::default()).await.unwrap();
        let marc = list.iter().find(|p| p.name == "Marc (plumber)").expect("listed");
        assert!(marc.email.starts_with('~') && marc.emails.is_empty() && !marc.automated);
        let page = people.person(&marc.email).await.unwrap();
        assert_eq!(page.person.phones[0].number, "06 00");
        // Found by its number, however it's typed.
        let found =
            people.list(&crate::people::ListQuery { q: "0600".into(), ..Default::default() }).await.unwrap();
        assert_eq!(found.len(), 1);

        // A note about him, before he has an address…
        let note = crate::notes::NoteInput {
            title: "Boiler".into(),
            people: vec![crate::tasks::PersonLink { email: marc.email.clone(), name: None }],
            ..Default::default()
        };
        db.notes().create(&note).await.unwrap();
        // …you give him an address: his page moves to it, and the note follows.
        let page = people.merge(&marc.email, "marc@plomberie.fr").await.unwrap();
        assert_eq!(page.person.email, "marc@plomberie.fr");
        let about =
            crate::notes::NoteQuery { person: Some("marc@plomberie.fr".into()), ..Default::default() };
        assert_eq!(db.notes().notes(&about).await.unwrap().len(), 1);

        // Deleted from Google: he leaves Havre too (he only came from there)…
        people.remove_link(link).await.unwrap();
        assert!(people.list(&crate::people::ListQuery::default()).await.unwrap().is_empty());

        // …unless you made him a favourite.
        let (link, _) = people.apply_remote(&book, &plumber).await.unwrap();
        let key = people.list(&Default::default()).await.unwrap()[0].email.clone();
        people
            .update(&key, &crate::people::PersonUpdate { favorite: Some(true), ..Default::default() })
            .await
            .unwrap();
        people.remove_link(link).await.unwrap();
        assert_eq!(people.list(&Default::default()).await.unwrap().len(), 1);

        // Switching the source off removes its books and what only came from them.
        people.apply_remote(&book, &remote("people/x", "Someone", &["x@y.org"])).await.unwrap();
        people.remove_books("google", None, None).await.unwrap();
        assert!(people.books().await.unwrap().is_empty());
        let left: Vec<String> =
            people.list(&Default::default()).await.unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(left, ["Marc (plumber)"]);
    }

    #[tokio::test]
    async fn the_same_person_in_two_books_and_merging() {
        let db = Database::in_memory().await.unwrap();
        let people = db.people();
        let google = google_book(&people).await;
        people
            .replace_books(
                "carddav",
                None,
                None,
                &[RemoteBook {
                    remote_id: "https://dav/ab/".into(),
                    name: "iCloud".into(),
                    read_only: false,
                }],
            )
            .await
            .unwrap();
        let icloud = people.books().await.unwrap().into_iter().find(|b| b.name == "iCloud").unwrap();

        people.apply_remote(&google, &remote("people/a", "Anna", &["anna@example.org"])).await.unwrap();
        people.apply_remote(&icloud, &remote("ab/1.vcf", "Anna", &["anna@example.org"])).await.unwrap();
        let page = people.person("anna@example.org").await.unwrap();
        assert_eq!(page.books, ["Google Contacts", "iCloud"]);

        // Two cards with different addresses, merged by hand: both books follow.
        people.apply_remote(&google, &remote("people/b", "Bob", &["bob@work.org"])).await.unwrap();
        people.apply_remote(&icloud, &remote("ab/2.vcf", "Bobby", &["bob@home.org"])).await.unwrap();
        let page = people.merge("bob@work.org", "bob@home.org").await.unwrap();
        assert_eq!(page.books, ["Google Contacts", "iCloud"]);
        assert_eq!(page.person.emails, ["bob@work.org", "bob@home.org"]);
    }

    #[tokio::test]
    async fn applying_a_listing() {
        let db = Database::in_memory().await.unwrap();
        let people = db.people();
        let book = google_book(&people).await;
        let (a, b) = (remote("r/a", "Ann", &["a@x.org"]), remote("r/b", "Ben", &["b@x.org"]));
        let first = Changes { contacts: vec![a.clone(), b.clone()], full: true, ..Changes::default() };
        people.apply_changes(&book, &[], &first).await.unwrap();
        let links = people.links(book.id).await.unwrap();
        assert_eq!(links.len(), 2);

        // Ann renamed in Havre; the book lists her again with the same ETag:
        // not merged again (Havre's change stays, to be sent).
        people
            .update(
                "a@x.org",
                &crate::people::PersonUpdate { name: Some("Anne".into()), ..Default::default() },
            )
            .await
            .unwrap();
        let again = Changes { contacts: vec![a.clone()], full: true, ..Changes::default() };
        people.apply_changes(&book, &links, &again).await.unwrap();
        assert_eq!(people.person("a@x.org").await.unwrap().person.name, "Anne");
        assert_eq!(people.pending(book.id).await.unwrap().len(), 1);
        // Ben wasn't in that full listing: gone.
        assert_eq!(people.links(book.id).await.unwrap().len(), 1);
        assert!(people.person("b@x.org").await.unwrap().person.contact.is_none());

        // A change list (not full) only removes what it says was deleted.
        let links = people.links(book.id).await.unwrap();
        let some = Changes { deleted: vec!["r/a".into()], ..Changes::default() };
        people.apply_changes(&book, &links, &some).await.unwrap();
        assert!(people.links(book.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn vcf_files_in_and_out() {
        let db = Database::in_memory().await.unwrap();
        let people = db.people();
        people
            .update(
                "claire@acme.fr",
                &crate::people::PersonUpdate { note: Some("Mine".into()), ..Default::default() },
            )
            .await
            .unwrap();
        let file = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Claire Martin\r\nEMAIL:claire@acme.fr\r\nNOTE:Theirs\r\n\
ORG:Acme\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nFN:Marc\r\nTEL:06\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:3.0\r\nTEL:07\r\nEND:VCARD\r\n";
        let report = people.import_vcf(file).await.unwrap();
        assert_eq!(report, ImportReport { added: 1, updated: 1, skipped: 1 });
        let claire = people.person("claire@acme.fr").await.unwrap();
        assert_eq!(
            (claire.note.as_str(), claire.person.company.as_str()),
            ("Mine", "Acme"),
            "your note stays"
        );
        // The same file again: Marc (no address) is found by name, not added twice.
        assert_eq!(people.import_vcf(file).await.unwrap(), ImportReport { added: 0, updated: 2, skipped: 1 });

        let out = people.export_vcf().await.unwrap();
        let cards = vcard::parse(&out);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].emails(), ["claire@acme.fr"]);
        assert_eq!(cards[1].details().phones[0].number, "06");
    }
}
