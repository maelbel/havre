//! Files: every attachment in your mail, in one list.
//!
//! Havre doesn't store files: your mail server does. What Havre knows is
//! the list of each message's attachments (`messages.files`, a JSON list
//! of `{index, name, mime, size}`), written when a message is downloaded
//! (see `Store::save_digest`). This module reads those lists across all
//! accounts and folders and turns them into one list of files:
//!
//! - the **same file** sent in several emails (same name, same size) shows
//!   once, with the other emails as `copies`;
//! - **versions** of a file (`devis.pdf`, `devis-v2.pdf`, `devis final.pdf`)
//!   are linked: see [`version_key`].
//!
//! With the Files app in the dock, the sync engines also look through older
//! mail (`depth` newest emails of each Inbox and Sent), so the list grows to
//! cover more than the mail you opened.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::{Error, Result, mail::FileInfo};

#[derive(Clone)]
pub struct FileStore {
    pool: SqlitePool,
}

/// Which files to list (`GET /api/files?…`). Every filter is optional.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FileQuery {
    /// Words in the file's name, the email's subject or the sender.
    pub q: Option<String>,
    /// `pdf`, `doc`, `sheet`, `slides`, `image`, `archive` or `other`.
    /// `documents` means PDFs and documents together.
    pub kind: Option<String>,
    /// From this address.
    pub person: Option<String>,
    /// `true`: only files you sent; `false`: only files you received.
    pub sent: Option<bool>,
    pub limit: Option<usize>,
}

/// Where a file is: a message, and its number among the attachments.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileEmail {
    pub account: i64,
    pub folder: String,
    pub uid: u32,
    pub subject: String,
    pub date: Option<String>,
    /// The sender, as shown (a name, or the address).
    pub from: Option<String>,
    pub sent: bool,
}

/// Another version of a file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileVersion {
    pub key: String,
    pub name: String,
    pub size: usize,
    pub date: Option<String>,
    pub from: Option<String>,
    pub sent: bool,
}

/// One file in the list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileItem {
    /// `account:index:uid:folder` — names one attachment, for the URL.
    pub key: String,
    pub name: String,
    pub mime: String,
    pub size: usize,
    pub kind: &'static str,
    pub index: usize,
    /// The newest email with this file.
    #[serde(flatten)]
    pub email: FileEmail,
    /// Its address, to filter by person.
    pub from_address: Option<String>,
    /// Other emails with the same file, newest first.
    pub copies: Vec<FileEmail>,
    /// Other versions of the file, newest first.
    pub versions: Vec<FileVersion>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct FileCounts {
    pub all: usize,
    pub received: usize,
    pub sent: usize,
    /// By kind (`pdf`, `doc`…).
    pub kinds: BTreeMap<&'static str, usize>,
}

/// Someone who sent you files, for the sidebar.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FilePerson {
    pub email: String,
    pub name: Option<String>,
    pub files: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileList {
    pub files: Vec<FileItem>,
    /// How many match (`files` may be cut at the limit).
    pub total: usize,
    /// Counts over every file, before the filters (the sidebar).
    pub counts: FileCounts,
    pub people: Vec<FilePerson>,
}

/// How far the look through older mail has gone.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct FilesProgress {
    /// Emails looked at, among the newest `depth` of each Inbox and Sent…
    pub looked: i64,
    /// …out of this many.
    pub total: i64,
    /// The setting (0: all of them).
    pub depth: i64,
}

/// An attachment as read from one message.
struct Found {
    file: FileInfo,
    email: FileEmail,
    from_address: Option<String>,
}

impl FileStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Every attachment Havre knows of, newest email first. Files in the
    /// Trash, Junk, Drafts and Gmail's "All Mail" (copies of the rest) are
    /// left out, and so are invitations (`.ics`: they're in the Agenda).
    async fn found(&self) -> Result<Vec<Found>> {
        // `json_each(m.files)` turns each message's JSON list into rows: one
        // row per attachment, next to its message's columns. SQL over JSON.
        let rows = sqlx::query(
            "SELECT m.account_id, m.folder, m.uid, m.subject, m.date, m.from_name, m.from_address,
                    f.role, j.value AS file
             FROM messages m
             JOIN folders f ON f.account_id = m.account_id AND f.path = m.folder,
                  json_each(m.files) j
             WHERE m.files IS NOT NULL AND m.files != '[]'
               AND f.role NOT IN ('trash', 'junk', 'drafts', 'all')
             ORDER BY datetime(m.date) DESC, m.uid DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for r in &rows {
            let Ok(file) = serde_json::from_str::<FileInfo>(r.get::<&str, _>("file")) else { continue };
            if file.mime == "text/calendar" || file.name.to_lowercase().ends_with(".ics") {
                continue;
            }
            let from_name: Option<String> = r.get("from_name");
            let from_address: Option<String> = r.get("from_address");
            let sent = r.get::<String, _>("role") == "sent";
            out.push(Found {
                email: FileEmail {
                    account: r.get("account_id"),
                    folder: r.get("folder"),
                    uid: r.get::<i64, _>("uid") as u32,
                    subject: r.get("subject"),
                    date: r.get("date"),
                    from: from_name.filter(|n| !n.is_empty()).or(from_address.clone()),
                    sent,
                },
                from_address: from_address.map(|a| a.to_lowercase()),
                file,
            });
        }
        Ok(out)
    }

    /// The files, once each, with their copies and versions.
    async fn all(&self) -> Result<Vec<FileItem>> {
        let mut items: Vec<FileItem> = Vec::new();
        // (name, size) → its place in `items`: the same file in another email.
        let mut same: HashMap<(String, usize), usize> = HashMap::new();
        for f in self.found().await? {
            let id = (f.file.name.to_lowercase(), f.file.size);
            if let Some(&at) = same.get(&id) {
                let item = &mut items[at];
                // The same email in two folders (a copy in Archive) isn't
                // another email: same subject, same date.
                let same_email = |e: &FileEmail| e.subject == f.email.subject && e.date == f.email.date;
                let already = same_email(&item.email) || item.copies.iter().any(same_email);
                if !already {
                    item.copies.push(f.email);
                }
                continue;
            }
            same.insert(id, items.len());
            items.push(FileItem {
                key: file_key(f.email.account, f.file.index, f.email.uid, &f.email.folder),
                kind: kind_of(&f.file.mime, &f.file.name),
                name: f.file.name,
                mime: f.file.mime,
                size: f.file.size,
                index: f.file.index,
                email: f.email,
                from_address: f.from_address,
                copies: Vec::new(),
                versions: Vec::new(),
            });
        }
        // Versions: files whose names are the same once "v2", "final"… are
        // left out. Grouped by that key, each file lists the others.
        let mut by_version: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, item) in items.iter().enumerate() {
            by_version.entry(version_key(&item.name)).or_default().push(i);
        }
        for group in by_version.values().filter(|g| g.len() > 1) {
            for &i in group {
                let versions = group
                    .iter()
                    .filter(|&&j| j != i)
                    .map(|&j| FileVersion {
                        key: items[j].key.clone(),
                        name: items[j].name.clone(),
                        size: items[j].size,
                        date: items[j].email.date.clone(),
                        from: items[j].email.from.clone(),
                        sent: items[j].email.sent,
                    })
                    .collect();
                items[i].versions = versions;
            }
        }
        Ok(items)
    }

    /// The Files page: the files matching `query`, plus the sidebar's counts.
    pub async fn list(&self, query: &FileQuery) -> Result<FileList> {
        let all = self.all().await?;
        let mut counts = FileCounts { all: all.len(), ..FileCounts::default() };
        let mut people: HashMap<String, FilePerson> = HashMap::new();
        for item in &all {
            if item.email.sent {
                counts.sent += 1;
            } else {
                counts.received += 1;
                if let Some(address) = &item.from_address {
                    let person = people.entry(address.clone()).or_insert(FilePerson {
                        email: address.clone(),
                        name: item.email.from.clone().filter(|n| n != address),
                        files: 0,
                    });
                    person.files += 1;
                }
            }
            *counts.kinds.entry(item.kind).or_default() += 1;
        }
        let mut people: Vec<FilePerson> = people.into_values().collect();
        // Most files first, then by name: the same order every time.
        people.sort_by(|a, b| b.files.cmp(&a.files).then_with(|| a.email.cmp(&b.email)));
        people.truncate(8);

        let words: Vec<String> =
            query.q.as_deref().unwrap_or_default().split_whitespace().map(str::to_lowercase).collect();
        let person = query.person.as_deref().map(str::to_lowercase).filter(|p| !p.is_empty());
        let kind = query.kind.as_deref().filter(|k| !k.is_empty());
        let matching: Vec<FileItem> = all
            .into_iter()
            .filter(|f| query.sent.is_none_or(|sent| f.email.sent == sent))
            .filter(|f| match kind {
                None => true,
                Some("documents") => matches!(f.kind, "pdf" | "doc"),
                Some(k) => f.kind == k,
            })
            .filter(|f| person.is_none() || f.from_address == person)
            .filter(|f| {
                let text =
                    format!("{} {} {}", f.name, f.email.subject, f.email.from.as_deref().unwrap_or(""))
                        .to_lowercase();
                words.iter().all(|w| text.contains(w.as_str()))
            })
            .collect();
        let total = matching.len();
        let limit = query.limit.unwrap_or(300).clamp(1, 2000);
        Ok(FileList { files: matching.into_iter().take(limit).collect(), total, counts, people })
    }

    /// How many different files there are (the App library card).
    pub async fn count(&self) -> Result<usize> {
        Ok(self.all().await?.len())
    }

    /// One email's attachments, with their other versions (Mail's hint).
    pub async fn in_message(&self, account: i64, folder: &str, uid: u32) -> Result<Vec<FileItem>> {
        let all = self.all().await?;
        Ok(all
            .into_iter()
            .filter(|f| {
                let here = |e: &FileEmail| e.account == account && e.folder == folder && e.uid == uid;
                here(&f.email) || f.copies.iter().any(here)
            })
            .collect())
    }

    // ---- How far back to look ---------------------------------------------------

    /// The setting: how many of the newest emails of each Inbox and Sent to
    /// look through (0: all).
    pub async fn depth(&self) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT depth FROM files_settings WHERE id = 1").fetch_one(&self.pool).await?)
    }

    pub async fn set_depth(&self, depth: i64) -> Result<()> {
        if !(0..=1_000_000).contains(&depth) {
            return Err(Error::Config("Choose how many emails to look through.".into()));
        }
        sqlx::query("UPDATE files_settings SET depth = ? WHERE id = 1")
            .bind(depth)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// How far the look has gone, over every Inbox and Sent.
    pub async fn progress(&self) -> Result<FilesProgress> {
        let depth = self.depth().await?;
        // In SQLite, `LIMIT -1` means no limit: "all of them".
        let limit = if depth == 0 { -1 } else { depth };
        let folders: Vec<(i64, String)> =
            sqlx::query_as("SELECT account_id, path FROM folders WHERE role IN ('inbox', 'sent')")
                .fetch_all(&self.pool)
                .await?;
        let (mut looked, mut total) = (0, 0);
        for (account, path) in folders {
            let (n, done): (i64, i64) = sqlx::query_as(
                "SELECT COUNT(*), COALESCE(SUM(files IS NOT NULL), 0) FROM (
                     SELECT files FROM messages WHERE account_id = ? AND folder = ? ORDER BY uid DESC LIMIT ?)",
            )
            .bind(account)
            .bind(&path)
            .bind(limit)
            .fetch_one(&self.pool)
            .await?;
            total += n;
            looked += done;
        }
        Ok(FilesProgress { looked, total, depth })
    }
}

/// `account:index:uid:folder` (the folder last: it may contain `:`).
fn file_key(account: i64, index: usize, uid: u32, folder: &str) -> String {
    format!("{account}:{index}:{uid}:{folder}")
}

/// What kind of file, for the icon, the colour and the filters.
pub fn kind_of(mime: &str, name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    let mime = mime.to_lowercase();
    match ext.as_str() {
        "pdf" => "pdf",
        "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "pages" => "doc",
        "xls" | "xlsx" | "ods" | "csv" | "numbers" => "sheet",
        "ppt" | "pptx" | "odp" | "key" => "slides",
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" => "archive",
        _ if mime.starts_with("image/") => "image",
        _ if mime == "application/pdf" => "pdf",
        _ => "other",
    }
}

/// The part of a name that stays the same from one version to the next:
/// `Devis-maintenance_Q4 v2 (1).pdf` and `devis maintenance q4 FINAL.pdf`
/// both give `devis maintenance q4.pdf`. Words like `v2`, `final`, `draft`,
/// `copy` and lone numbers in brackets are left out; the extension stays,
/// so a PDF and a Word file of the same name aren't versions.
pub fn version_key(name: &str) -> String {
    let lower = name.to_lowercase();
    let (stem, ext) = lower.rsplit_once('.').unwrap_or((lower.as_str(), ""));
    let words: Vec<&str> = stem
        // An array of chars: split at any of them.
        .split(['-', '_', ' ', '.', '(', ')'])
        .filter(|w| !w.is_empty())
        .filter(|w| {
            let version = w.len() > 1 && w.starts_with('v') && w[1..].chars().all(|c| c.is_ascii_digit());
            let lone_digit = w.len() == 1 && w.chars().all(|c| c.is_ascii_digit());
            !(version
                || lone_digit
                || matches!(
                    *w,
                    "final" | "finale" | "draft" | "copy" | "copie" | "def" | "definitif" | "définitif"
                ))
        })
        .collect();
    format!("{}.{ext}", words.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Database;

    #[test]
    fn kinds() {
        assert_eq!(kind_of("application/pdf", "devis.pdf"), "pdf");
        assert_eq!(kind_of("application/octet-stream", "Budget.XLSX"), "sheet");
        assert_eq!(kind_of("image/jpeg", "photo"), "image");
        assert_eq!(kind_of("application/x-what", "plans.zip"), "archive");
        assert_eq!(kind_of("application/x-what", "thing.bin"), "other");
    }

    #[test]
    fn versions_share_a_key() {
        let key = version_key("devis-maintenance-q4.pdf");
        assert_eq!(key, "devis maintenance q4.pdf");
        assert_eq!(version_key("Devis_maintenance_Q4 v2 (1).pdf"), key);
        assert_eq!(version_key("devis maintenance q4 FINAL.pdf"), key);
        assert_ne!(version_key("devis-maintenance-q4.docx"), key, "another kind of file");
        assert_ne!(version_key("devis-maintenance-q3.pdf"), key, "q3 isn't a version number");
    }

    /// A message with attachments in `folder` (the JSON the digest writes).
    async fn message(
        db: &Database,
        account: i64,
        folder: &str,
        uid: u32,
        subject: &str,
        date: &str,
        files: &str,
    ) {
        sqlx::query(
            "INSERT INTO messages (account_id, folder, uid, subject, from_name, from_address, date, unread, search, files)
             VALUES (?, ?, ?, ?, 'Thomas Renard', 'Thomas@renard.fr', ?, 0, '', ?)",
        )
        .bind(account)
        .bind(folder)
        .bind(uid as i64)
        .bind(subject)
        .bind(date)
        .bind(files)
        .execute(&db.pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn one_list_with_copies_and_versions() {
        let db = Database::in_memory().await.unwrap();
        let account = db.insert_account(&crate::store::tests::new_account("me@x.test")).await.unwrap().id;
        db.store(account)
            .replace_folders(&[
                crate::store::tests::folder("INBOX", crate::mail::FolderRole::Inbox),
                crate::store::tests::folder("Sent", crate::mail::FolderRole::Sent),
                crate::store::tests::folder("Trash", crate::mail::FolderRole::Trash),
            ])
            .await
            .unwrap();
        let pdf = |name: &str, size: usize| {
            format!(r#"[{{"index":0,"name":"{name}","mime":"application/pdf","size":{size}}}]"#)
        };
        message(&db, account, "INBOX", 1, "Devis", "2026-10-07T16:48:00+02:00", &pdf("devis-q4.pdf", 100))
            .await;
        message(
            &db,
            account,
            "INBOX",
            2,
            "Re: Devis",
            "2026-10-17T08:15:00+02:00",
            &pdf("devis-q4-v2.pdf", 120),
        )
        .await;
        message(
            &db,
            account,
            "Sent",
            3,
            "Fwd: Devis",
            "2026-10-08T09:00:00+02:00",
            &pdf("devis-q4.pdf", 100),
        )
        .await;
        message(&db, account, "Trash", 4, "Old", "2026-10-01T09:00:00+02:00", &pdf("old.pdf", 5)).await;
        message(
            &db,
            account,
            "INBOX",
            5,
            "Invitation",
            "2026-10-02T09:00:00+02:00",
            r#"[{"index":0,"name":"invite.ics","mime":"text/calendar","size":900}]"#,
        )
        .await;

        let files = db.files();
        let list = files.list(&FileQuery::default()).await.unwrap();
        let names: Vec<&str> = list.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["devis-q4-v2.pdf", "devis-q4.pdf"], "newest first; no Trash, no invitation");
        let first = &list.files[1];
        assert_eq!(first.email.subject, "Fwd: Devis", "shown with its newest email…");
        assert_eq!(first.copies.len(), 1, "…and the other one as a copy");
        assert_eq!(list.files[0].versions[0].name, "devis-q4.pdf");
        assert_eq!((list.counts.all, list.counts.sent, list.counts.received), (2, 1, 1));
        assert_eq!(list.people[0].email, "thomas@renard.fr");

        let sent = files.list(&FileQuery { sent: Some(true), ..Default::default() }).await.unwrap();
        assert_eq!(sent.total, 1);
        let found = files.list(&FileQuery { q: Some("V2 re:".into()), ..Default::default() }).await.unwrap();
        assert_eq!(found.files.len(), 1, "words from the name and the subject");
        assert_eq!(files.in_message(account, "INBOX", 1).await.unwrap()[0].name, "devis-q4.pdf");
    }

    #[tokio::test]
    async fn progress_counts_the_newest_of_inbox_and_sent() {
        let db = Database::in_memory().await.unwrap();
        let account = db.insert_account(&crate::store::tests::new_account("me@x.test")).await.unwrap().id;
        db.store(account)
            .replace_folders(&[crate::store::tests::folder("INBOX", crate::mail::FolderRole::Inbox)])
            .await
            .unwrap();
        for uid in 1..=4 {
            let files = if uid >= 3 { Some("[]") } else { None };
            sqlx::query(
                "INSERT INTO messages (account_id, folder, uid, subject, date, unread, search, files)
                 VALUES (?, 'INBOX', ?, 's', NULL, 0, '', ?)",
            )
            .bind(account)
            .bind(uid)
            .bind(files)
            .execute(&db.pool())
            .await
            .unwrap();
        }
        let files = db.files();
        files.set_depth(3).await.unwrap();
        let p = files.progress().await.unwrap();
        assert_eq!((p.looked, p.total), (2, 3), "the newest 3: two looked at");
        files.set_depth(0).await.unwrap();
        assert_eq!(files.progress().await.unwrap().total, 4, "0: all of them");
        assert!(files.set_depth(-1).await.is_err());
    }
}
