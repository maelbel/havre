//! People: the address book Havre builds by itself, and what you add to it.
//!
//! Everyone who wrote to you, everyone you write to (Mail) and everyone you
//! meet (Agenda guests) is a person; their email address is the key. You can
//! add details (company, phones, birthday, a note), mark favourites, and
//! **merge** two addresses of the same person ([`contacts`]). Your address
//! books (Google, Microsoft, CardDAV) are kept in step with those details
//! ([`books`], [`sync`]); someone there without an address is a person too,
//! keyed `~<contact id>`. The same list feeds:
//!
//! - the **People** app: the list (sorts, filters) and a page per person
//!   (details, keeping in touch, mail, meetings, files);
//! - the suggestions when you type a recipient or invite a guest;
//! - Today's birthdays and "keep in touch".
//!
//! How it's built ([`People::everyone`]): one SQL query counts, per address,
//! the mail from them, the mail you sent them and the meetings, with the
//! dates of the last ones (SQL over tables Mail and the Agenda already fill,
//! so it's always up to date). Then, in Rust, the addresses of one contact are
//! put together and your details laid on top.

pub mod books;
pub(crate) mod carddav;
pub mod contacts;
pub(crate) mod google;
pub(crate) mod graph;
pub mod kinds;
pub mod sync;
pub mod vcard;

use std::collections::HashMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::{
    Result,
    agenda::{AgendaStore, Event},
};
pub use contacts::{Details, Key, PersonUpdate, Phone};

/// Waiting for your reply: only for mail of the last 30 days.
const WAITING_FOR: i64 = 30 * 86_400;
/// Keep in touch: favourites you haven't been in touch with for 30 days.
const KEEP_IN_TOUCH: i64 = 30 * 86_400;

/// One person, in a list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Person {
    /// The main address (the page's address), or `~<contact id>` for someone
    /// without one.
    pub email: String,
    /// Every address of this person, the main one first.
    pub emails: Vec<String>,
    /// Your name for them, else the one they use (empty if we never saw one).
    pub name: String,
    /// Their details, when you added some (`None`: just an address).
    pub contact: Option<i64>,
    pub company: String,
    pub role: String,
    pub favorite: bool,
    /// A program (no-reply, newsletters…), guessed or as you said.
    pub automated: bool,
    /// Messages from them, in all your accounts.
    pub messages: i64,
    /// Messages you sent them (To or Cc).
    pub sent: i64,
    /// Meetings with them (past and to come).
    pub meetings: i64,
    /// The last email either way, or the last meeting (unix seconds).
    pub last_contact: Option<i64>,
    /// Their latest email in an inbox is newer than anything you sent them.
    pub waiting_since: Option<i64>,
    /// `YYYY-MM-DD`, or `--MM-DD` without the year.
    pub birthday: Option<String>,
    /// An address book has a photo of them (`GET /people/{email}/photo`).
    pub photo: bool,
    pub phones: Vec<Phone>,
}

/// A person's page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PersonDetail {
    #[serde(flatten)]
    pub person: Person,
    pub note: String,
    /// What you said: `person` or `automated` (`None`: guessed).
    pub kind: Option<String>,
    /// The address books they're in ("Google Contacts", "iCloud"…).
    pub books: Vec<String>,
    /// The address book that last changed their details, and when.
    pub updated_from: Option<String>,
    pub updated_from_at: Option<i64>,
    pub last_from: Option<i64>,
    pub last_to: Option<i64>,
    pub last_meeting: Option<i64>,
    pub next_meeting: Option<i64>,
    /// Newest first.
    pub recent_messages: Vec<MessageRef>,
    /// Soonest first.
    pub upcoming: Vec<Event>,
    /// Most recent first.
    pub past: Vec<Event>,
    /// Files they sent (from the messages Havre downloaded), newest first.
    pub files: Vec<SharedFile>,
}

/// A message from someone, wherever it is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MessageRef {
    pub account: i64,
    pub folder: String,
    pub uid: u32,
    pub subject: String,
    pub date: Option<String>,
    pub unread: bool,
    /// Who sent it (for lists mixing several people).
    pub from: Option<String>,
    pub preview: Option<String>,
}

/// An attachment someone sent.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SharedFile {
    pub name: String,
    pub mime: String,
    pub size: usize,
    /// The message it came with, and its number there (to download it).
    pub account: i64,
    pub folder: String,
    pub uid: u32,
    pub index: usize,
    pub date: Option<String>,
}

/// Mail and files from some people (an event's guests: "Related emails").
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Related {
    pub messages: Vec<MessageRef>,
    pub files: Vec<SharedFile>,
}

/// How the list is sorted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    /// Last contact first (the default).
    #[default]
    Recent,
    /// The ones you deal with most first.
    Frequent,
    /// A to Z.
    Name,
}

/// Which people the list shows.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    /// Everyone but robots (the default).
    #[default]
    Everyone,
    Favorites,
    /// Their email waits for your reply.
    Waiting,
    /// No-reply senders, newsletters, and whoever you marked so.
    Automated,
    /// Everything, robots included (recipient suggestions).
    All,
}

/// The list's question: a search, a sort, a filter, maybe a company.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub sort: Sort,
    #[serde(default)]
    pub filter: Filter,
    /// Only people of this company (see [`kinds::organization`]).
    #[serde(default)]
    pub company: Option<String>,
    pub limit: Option<u32>,
}

/// A company and how many people you know there.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Company {
    pub name: String,
    pub people: usize,
}

/// Today's People lines.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PeopleToday {
    /// Birthdays today and in the next 7 days, soonest first.
    pub birthdays: Vec<Birthday>,
    /// Up to two favourites you haven't been in touch with for 30 days.
    pub keep_in_touch: Vec<Person>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Birthday {
    pub person: Person,
    /// The day (`YYYY-MM-DD`), and how many days away (0: today).
    pub date: String,
    pub days: i64,
    /// The age they turn, when the year is known.
    pub turns: Option<i32>,
}

/// Everything known about one person, before it's shaped for the list or the page.
#[derive(Debug, Clone, Default)]
struct Entry {
    contact: Option<contacts::ContactRow>,
    emails: Vec<String>,
    seen_name: String,
    messages: i64,
    sent: i64,
    meetings: i64,
    last_from: Option<i64>,
    last_inbox: Option<i64>,
    last_to: Option<i64>,
    last_meeting: Option<i64>,
    next_meeting: Option<i64>,
}

impl Entry {
    /// Adds one address's counts (a contact with several addresses).
    fn add(&mut self, s: &Stats) {
        if self.seen_name.is_empty() {
            self.seen_name = s.name.clone();
        }
        self.messages += s.messages;
        self.sent += s.sent;
        self.meetings += s.meetings;
        // `Option::max` keeps `Some` over `None`; for the next meeting we want the soonest.
        self.last_from = self.last_from.max(s.last_from);
        self.last_inbox = self.last_inbox.max(s.last_inbox);
        self.last_to = self.last_to.max(s.last_to);
        self.last_meeting = self.last_meeting.max(s.last_meeting);
        self.next_meeting = match (self.next_meeting, s.next_meeting) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }

    fn person(&self, now: i64) -> Person {
        let c = self.contact.as_ref();
        // No address: the contact's id is the key.
        let email = match (self.emails.first(), c) {
            (Some(email), _) => email.clone(),
            (None, Some(c)) => format!("~{}", c.id),
            (None, None) => String::new(),
        };
        let automated = match c.and_then(|c| c.kind.as_deref()) {
            Some("automated") => true,
            Some("person") => false,
            // `all` is true for an empty list: someone without an address is a person.
            _ => !self.emails.is_empty() && self.emails.iter().all(|e| kinds::looks_automated(e)),
        };
        let name = c.map(|c| c.name.trim()).filter(|n| !n.is_empty()).unwrap_or(&self.seen_name).to_owned();
        let last_contact = [self.last_from, self.last_to, self.last_meeting].into_iter().flatten().max();
        // Waiting: their latest inbox email came after anything you sent, recently, and they're a person.
        let waiting_since = self
            .last_inbox
            .filter(|&at| !automated && at > now - WAITING_FOR && self.last_to.is_none_or(|sent| sent < at));
        Person {
            email,
            emails: self.emails.clone(),
            name,
            contact: c.map(|c| c.id),
            company: c.map(|c| c.company.clone()).unwrap_or_default(),
            role: c.map(|c| c.role.clone()).unwrap_or_default(),
            favorite: c.is_some_and(|c| c.favorite),
            automated,
            messages: self.messages,
            sent: self.sent,
            meetings: self.meetings,
            last_contact,
            waiting_since,
            birthday: c.and_then(|c| c.birthday.clone()),
            photo: c.is_some_and(|c| c.photo),
            phones: c.map(|c| c.phones.clone()).unwrap_or_default(),
        }
    }
}

/// One address's counts, from SQL.
#[derive(Debug, Clone, Default)]
struct Stats {
    name: String,
    messages: i64,
    sent: i64,
    meetings: i64,
    last_from: Option<i64>,
    last_inbox: Option<i64>,
    last_to: Option<i64>,
    last_meeting: Option<i64>,
    next_meeting: Option<i64>,
}

/// Per address: mail from them, mail you sent them, meetings, and when.
/// Your own addresses (your accounts) are left out.
///
/// - `json_each` turns a JSON list (a message's recipients, an event's
///   guests) into rows, so SQLite can count them like a normal table.
/// - `unixepoch(date)` reads the RFC 3339 date (with its offset) as seconds.
/// - Each kind of sighting fills its own columns; `MAX`/`MIN` per address
///   then give the latest (or next) one.
const STATS: &str = "
    WITH me AS (SELECT lower(email) AS email FROM accounts),
    seen AS (
        SELECT lower(m.from_address) AS email, m.from_name AS name, 1 AS got, 0 AS sent, 0 AS met,
               unixepoch(m.date) AS got_at,
               CASE WHEN f.role = 'inbox' THEN unixepoch(m.date) END AS inbox_at,
               NULL AS sent_at, NULL AS met_at, NULL AS next_at
        FROM messages m LEFT JOIN folders f ON f.account_id = m.account_id AND f.path = m.folder
        WHERE m.from_address IS NOT NULL AND m.from_address <> ''
        UNION ALL
        SELECT lower(r.value), NULL, 0, 1, 0, NULL, NULL, unixepoch(m.date), NULL, NULL
        FROM messages m, json_each(m.recipients) r
        WHERE m.recipients IS NOT NULL AND lower(m.from_address) IN (SELECT email FROM me)
        UNION ALL
        SELECT lower(json_extract(a.value, '$.email')), json_extract(a.value, '$.name'), 0, 0, 1, NULL, NULL, NULL,
               CASE WHEN e.start_ts <= ?1 THEN e.start_ts END, CASE WHEN e.start_ts > ?1 THEN e.start_ts END
        FROM events e, json_each(e.attendees) a
    )
    SELECT email, COALESCE(MAX(NULLIF(name, '')), '') AS name, SUM(got) AS messages, SUM(sent) AS sent,
           SUM(met) AS meetings, MAX(got_at) AS last_from, MAX(inbox_at) AS last_inbox, MAX(sent_at) AS last_to,
           MAX(met_at) AS last_meeting, MIN(next_at) AS next_meeting
    FROM seen
    WHERE email IS NOT NULL AND email <> '' AND email NOT IN (SELECT email FROM me)
    GROUP BY email";

/// The company names you wrote, by email domain: once you write "Acme" for
/// someone at acme.fr, their colleagues at acme.fr are "Acme" too.
fn company_names(people: &[Person]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for p in people.iter().filter(|p| !p.company.trim().is_empty()) {
        for email in &p.emails {
            // `organization` with no company gives the domain (None for gmail.com…).
            if let Some(domain) = kinds::organization("", email) {
                names.entry(domain).or_insert_with(|| p.company.trim().to_owned());
            }
        }
    }
    names
}

/// Someone's company: the one you wrote, else the name a colleague's gives
/// their domain, else the domain itself.
fn company_of(p: &Person, names: &HashMap<String, String>) -> Option<String> {
    let org = kinds::organization(&p.company, &p.email)?;
    // A let chain: both conditions in one `if` (Rust 2024).
    if p.company.trim().is_empty()
        && let Some(name) = names.get(&org)
    {
        return Some(name.clone());
    }
    Some(org)
}

/// Reads people out of the database.
#[derive(Clone)]
pub struct People {
    pool: SqlitePool,
    agenda: AgendaStore,
}

impl People {
    pub fn new(pool: SqlitePool, agenda: AgendaStore) -> Self {
        Self { pool, agenda }
    }

    /// Everyone: one entry per contact (all its addresses together) or per
    /// address you never added details to.
    async fn everyone(&self, now: i64) -> Result<Vec<Entry>> {
        let rows = sqlx::query(STATS).bind(now).fetch_all(&self.pool).await?;
        let mut stats: HashMap<String, Stats> = rows
            .iter()
            .map(|r| {
                let s = Stats {
                    name: r.get("name"),
                    messages: r.get("messages"),
                    sent: r.get("sent"),
                    meetings: r.get("meetings"),
                    last_from: r.get("last_from"),
                    last_inbox: r.get("last_inbox"),
                    last_to: r.get("last_to"),
                    last_meeting: r.get("last_meeting"),
                    next_meeting: r.get("next_meeting"),
                };
                (r.get("email"), s)
            })
            .collect();
        let mut entries = Vec::new();
        // Contacts first: they take their addresses' counts out of the map…
        for contact in self.contacts().await? {
            let mut entry = Entry { emails: contact.emails.clone(), ..Entry::default() };
            for email in &contact.emails {
                if let Some(s) = stats.remove(email) {
                    entry.add(&s);
                }
            }
            entry.contact = Some(contact);
            entries.push(entry);
        }
        // …and every address left is a person of its own.
        for (email, s) in stats {
            let mut entry = Entry { emails: vec![email], ..Entry::default() };
            entry.add(&s);
            entries.push(entry);
        }
        Ok(entries)
    }

    /// The People list: searched (name, address, company), filtered, sorted.
    pub async fn list(&self, q: &ListQuery) -> Result<Vec<Person>> {
        let now = now();
        let query = q.q.trim().to_lowercase();
        // A number searches phones, whatever its spaces and dots: "0612" finds "06 12 34…".
        let digits = only_digits(&query);
        let company = q.company.as_deref().map(str::to_lowercase);
        let everyone: Vec<Person> = self.everyone(now).await?.iter().map(|e| e.person(now)).collect();
        let names = company_names(&everyone);
        let mut people: Vec<Person> = everyone
            .into_iter()
            .filter(|p| match q.filter {
                Filter::Everyone => !p.automated,
                Filter::Favorites => p.favorite,
                Filter::Waiting => p.waiting_since.is_some(),
                Filter::Automated => p.automated,
                Filter::All => true,
            })
            .filter(|p| {
                company.as_ref().is_none_or(|c| company_of(p, &names).is_some_and(|o| o.to_lowercase() == *c))
            })
            .filter(|p| {
                query.is_empty()
                    || p.name.to_lowercase().contains(&query)
                    || p.company.to_lowercase().contains(&query)
                    || p.emails.iter().any(|e| e.contains(&query))
                    || (digits.len() >= 3
                        && p.phones.iter().any(|ph| only_digits(&ph.number).contains(&digits)))
            })
            .collect();
        let frequency = |p: &Person| p.messages + 2 * p.sent + 3 * p.meetings;
        match q.sort {
            // `Reverse`: newest first; `None` (never) goes last.
            Sort::Recent => people.sort_by_key(|p| (std::cmp::Reverse(p.last_contact), p.email.clone())),
            Sort::Frequent => people.sort_by_key(|p| (std::cmp::Reverse(frequency(p)), p.email.clone())),
            Sort::Name => people.sort_by_cached_key(|p| {
                let shown = if p.name.is_empty() { &p.email } else { &p.name };
                (shown.to_lowercase(), p.email.clone())
            }),
        }
        people.truncate(q.limit.unwrap_or(500) as usize);
        Ok(people)
    }

    /// People whose name or address contains `query` (everyone if empty),
    /// the ones you deal with most first: suggestions for recipients and
    /// guests, and the assistant's `search_people`.
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<Person>> {
        self.list(&ListQuery {
            q: query.to_owned(),
            sort: Sort::Frequent,
            filter: Filter::All,
            company: None,
            limit: Some(limit),
        })
        .await
    }

    /// The companies of the people you know (not robots), the biggest first.
    pub async fn companies(&self) -> Result<Vec<Company>> {
        let now = now();
        let everyone: Vec<Person> = self.everyone(now).await?.iter().map(|e| e.person(now)).collect();
        let names = company_names(&everyone);
        let mut counts: HashMap<String, usize> = HashMap::new();
        for p in everyone.iter().filter(|p| !p.automated) {
            if let Some(org) = company_of(p, &names) {
                *counts.entry(org).or_default() += 1;
            }
        }
        let mut list: Vec<Company> =
            counts.into_iter().map(|(name, people)| Company { name, people }).collect();
        list.sort_by(|a, b| b.people.cmp(&a.people).then_with(|| a.name.cmp(&b.name)));
        list.truncate(40);
        Ok(list)
    }

    /// One person's page, by any of their addresses or their contact
    /// (`~42`). Works for any address, even one never seen (the page is then
    /// empty).
    pub async fn person(&self, key: &str) -> Result<PersonDetail> {
        let key = Key::parse(key)?;
        let now = now();
        let found = self.everyone(now).await?.into_iter().find(|e| match &key {
            Key::Address(email) => e.emails.contains(email),
            Key::Contact(id) => e.contact.as_ref().is_some_and(|c| c.id == *id),
        });
        let entry = match (found, &key) {
            (Some(entry), _) => entry,
            (None, Key::Address(email)) => Entry { emails: vec![email.clone()], ..Entry::default() },
            (None, Key::Contact(id)) => return Err(crate::Error::NotFound(format!("person ~{id}"))),
        };
        let person = entry.person(now);

        let Related { messages: recent_messages, files } = self.related(&entry.emails, 15).await?;
        // Meetings with any of their addresses, each once.
        let (mut upcoming, mut past) = (Vec::new(), Vec::new());
        for address in &entry.emails {
            for e in self.agenda.events_with(address, Some(now), 10).await? {
                if !upcoming.iter().any(|x: &Event| x.id == e.id) {
                    upcoming.push(e);
                }
            }
            for e in self.agenda.events_with(address, None, 10).await? {
                if !past.iter().any(|x: &Event| x.id == e.id) {
                    past.push(e);
                }
            }
        }
        upcoming.sort_by_key(|e| e.start);
        upcoming.truncate(10);
        past.sort_by_key(|e| std::cmp::Reverse(e.start));
        past.truncate(10);

        let c = entry.contact.as_ref();
        Ok(PersonDetail {
            person,
            note: c.map(|c| c.note.clone()).unwrap_or_default(),
            kind: c.and_then(|c| c.kind.clone()),
            books: c.map(|c| c.books.clone()).unwrap_or_default(),
            updated_from: c.and_then(|c| c.updated_from.clone()),
            updated_from_at: c.and_then(|c| c.updated_from_at),
            last_from: entry.last_from,
            last_to: entry.last_to,
            last_meeting: entry.last_meeting,
            next_meeting: entry.next_meeting,
            recent_messages,
            upcoming,
            past,
            files,
        })
    }

    /// [`People::today`] for a date written `YYYY-MM-DD` (what the browser sends).
    pub async fn today_on(&self, date: &str) -> Result<PeopleToday> {
        let day = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d")
            .map_err(|_| crate::Error::Config(format!("“{date}” isn't a date (YYYY-MM-DD).")))?;
        self.today(day).await
    }

    /// Today's People lines, for the day `today` (the browser's date).
    pub async fn today(&self, today: NaiveDate) -> Result<PeopleToday> {
        let now = now();
        let entries = self.everyone(now).await?;
        let mut birthdays = Vec::new();
        let mut keep_in_touch = Vec::new();
        for entry in &entries {
            let person = entry.person(now);
            if let Some((date, turns)) =
                person.birthday.as_deref().and_then(|b| kinds::next_birthday(b, today))
            {
                let days = (date - today).num_days();
                if days <= 7 {
                    birthdays.push(Birthday { person: person.clone(), date: date.to_string(), days, turns });
                }
            }
            // A favourite, quiet for a month, and no meeting with them in the coming week.
            let quiet = person.last_contact.is_none_or(|at| at < now - KEEP_IN_TOUCH);
            let meeting_soon = entry.next_meeting.is_some_and(|at| at < now + 7 * 86_400);
            if person.favorite && !person.automated && quiet && !meeting_soon {
                keep_in_touch.push(person);
            }
        }
        birthdays.sort_by_key(|b| (b.days, b.person.email.clone()));
        // The longest silence first (never: first of all).
        keep_in_touch.sort_by_key(|p| (p.last_contact, p.email.clone()));
        keep_in_touch.truncate(2);
        Ok(PeopleToday { birthdays, keep_in_touch })
    }

    /// Recent mail from these addresses (any case), and the files in it.
    pub async fn related(&self, emails: &[String], limit: u32) -> Result<Related> {
        let emails: Vec<String> =
            emails.iter().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect();
        if emails.is_empty() {
            return Ok(Related { messages: Vec::new(), files: Vec::new() });
        }
        let marks = vec!["?"; emails.len()].join(", ");
        // `datetime(date)` reads the RFC 3339 date (with its offset) as UTC,
        // so messages from different time zones sort correctly.
        let sql = format!(
            "SELECT account_id, folder, uid, subject, date, unread, from_name, from_address, preview, files
             FROM messages WHERE lower(from_address) IN ({marks})
             ORDER BY datetime(date) DESC LIMIT ?"
        );
        // `sql` is built from `?` marks only (see `sql!`).
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
        for e in &emails {
            q = q.bind(e);
        }
        let rows = q.bind(i64::from(limit)).fetch_all(&self.pool).await?;
        let mut messages = Vec::new();
        let mut files = Vec::new();
        for r in &rows {
            let m = MessageRef {
                account: r.get("account_id"),
                folder: r.get("folder"),
                uid: r.get::<i64, _>("uid") as u32,
                subject: r.get("subject"),
                date: r.get("date"),
                unread: r.get("unread"),
                from: r.get::<Option<String>, _>("from_name").or(r.get("from_address")),
                preview: r.get::<Option<String>, _>("preview").filter(|p| !p.is_empty()),
            };
            let attached: Vec<crate::mail::FileInfo> = r
                .get::<Option<String>, _>("files")
                .and_then(|f| serde_json::from_str(&f).ok())
                .unwrap_or_default();
            for f in attached.into_iter().filter(|f| f.mime != "text/calendar") {
                files.push(SharedFile {
                    name: f.name,
                    mime: f.mime,
                    size: f.size,
                    account: m.account,
                    folder: m.folder.clone(),
                    uid: m.uid,
                    index: f.index,
                    date: m.date.clone(),
                });
            }
            messages.push(m);
        }
        files.truncate(8);
        Ok(Related { messages, files })
    }
}

fn only_digits(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        mail::{Folder, FolderRole, MessageSummary, Person as From},
        store::{Database, NewAccount},
    };

    const DAY: i64 = 86_400;

    /// A mailbox with an inbox and a Sent folder, for "me@example.org".
    async fn mailbox() -> (Database, crate::store::Store) {
        let db = Database::in_memory().await.unwrap();
        let account = db
            .insert_account(&NewAccount {
                name: "Me".into(),
                email: "me@example.org".into(),
                imap_host: "mail.example.org".into(),
                imap_port: 993,
                imap_user: "me".into(),
                color: "work".into(),
                password: "x".into(),
                sender_name: String::new(),
                smtp_host: None,
                smtp_port: None,
                smtp_security: None,
            })
            .await
            .unwrap();
        let store = db.store(account.id);
        let folder = |path: &str, role| Folder {
            path: path.into(),
            name: path.into(),
            depth: 0,
            role,
            declared_role: true,
        };
        store
            .replace_folders(&[folder("INBOX", FolderRole::Inbox), folder("Sent", FolderRole::Sent)])
            .await
            .unwrap();
        (db, store)
    }

    /// An email `days_ago`, from someone (or from me, to `to`).
    fn mail(uid: u32, from: (&str, &str), to: &[&str], days_ago: i64) -> MessageSummary {
        let date = chrono::DateTime::from_timestamp(now() - days_ago * DAY, 0).unwrap();
        MessageSummary {
            uid,
            from: Some(From { name: Some(from.0.into()), address: Some(from.1.into()) }),
            subject: format!("Hello {uid}"),
            date: Some(date.to_rfc3339()),
            recipients: Some(to.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        }
    }

    async fn meeting(db: &Database, with: &str, days_from_now: i64) {
        sqlx::query(
            "INSERT INTO events (calendar_id, remote_id, title, start_ts, end_ts, attendees)
             VALUES ((SELECT id FROM calendars LIMIT 1), ?, 'Review', ?, ?, ?)",
        )
        .bind(format!("m{with}{days_from_now}"))
        .bind(now() + days_from_now * DAY)
        .bind(now() + days_from_now * DAY + 3600)
        .bind(format!(r#"[{{"name":"","email":"{with}","status":"accepted"}}]"#))
        .execute(&db.pool())
        .await
        .unwrap();
    }

    fn emails(list: &[Person]) -> Vec<&str> {
        list.iter().map(|p| p.email.as_str()).collect()
    }

    #[tokio::test]
    async fn people_come_from_mail_both_ways_and_meetings() {
        let (db, store) = mailbox().await;
        let me = ("Me", "me@example.org");
        store
            .upsert_messages(
                "INBOX",
                &[
                    mail(1, ("Anna", "anna@example.org"), &["me@example.org"], 9),
                    mail(2, ("Anna Martin", "Anna@Example.org"), &["me@example.org"], 2),
                    mail(3, ("Bob", "bob@example.org"), &["me@example.org"], 1),
                    mail(4, ("GitHub", "notifications@github.com"), &["me@example.org"], 0),
                ],
            )
            .await
            .unwrap();
        // I answered Anna, and wrote to Dana (who never wrote back).
        store
            .upsert_messages(
                "Sent",
                &[mail(10, me, &["anna@example.org"], 1), mail(11, me, &["dana@acme.fr"], 3)],
            )
            .await
            .unwrap();
        meeting(&db, "carla@acme.fr", 5).await;
        meeting(&db, "anna@example.org", -20).await;

        let people = db.people();
        // An `async` closure: it owns its `ListQuery` while the future runs.
        let list =
            async |sort, filter| people.list(&ListQuery { sort, filter, ..ListQuery::default() }).await;
        // Recent: Bob (1 d), Anna (my reply 1 d, after Bob? same day: by address), Dana, Carla (never yet).
        let recent = list(Sort::Recent, Filter::Everyone).await.unwrap();
        assert_eq!(emails(&recent), ["anna@example.org", "bob@example.org", "dana@acme.fr", "carla@acme.fr"]);
        assert!(!emails(&recent).contains(&"notifications@github.com"), "robots are set apart");
        assert_eq!(
            emails(&list(Sort::Recent, Filter::Automated).await.unwrap()),
            ["notifications@github.com"]
        );

        let anna = &recent[0];
        assert_eq!((anna.name.as_str(), anna.messages, anna.sent, anna.meetings), ("Anna Martin", 2, 1, 1));
        assert_eq!(anna.waiting_since, None, "I replied after her last email");
        let bob = &recent[1];
        assert!(bob.waiting_since.is_some(), "Bob waits for an answer");
        assert_eq!(emails(&list(Sort::Recent, Filter::Waiting).await.unwrap()), ["bob@example.org"]);
        assert_eq!(recent[2].sent, 1, "Dana: someone I write to");

        // Frequent: Anna (2 + 2×1 + 3×1 = 7), Carla (3), Dana (2), Bob (1).
        let frequent = list(Sort::Frequent, Filter::Everyone).await.unwrap();
        assert_eq!(
            emails(&frequent),
            ["anna@example.org", "carla@acme.fr", "dana@acme.fr", "bob@example.org"]
        );
        let by_name = list(Sort::Name, Filter::Everyone).await.unwrap();
        assert_eq!(
            emails(&by_name),
            ["anna@example.org", "bob@example.org", "carla@acme.fr", "dana@acme.fr"]
        );

        // Companies: acme.fr from the domains (example.org too); a search on the company.
        let companies = people.companies().await.unwrap();
        assert_eq!(companies[0], Company { name: "acme.fr".into(), people: 2 });
        let at_acme =
            people.list(&ListQuery { company: Some("ACME.fr".into()), ..ListQuery::default() }).await;
        assert_eq!(emails(&at_acme.unwrap()), ["dana@acme.fr", "carla@acme.fr"]);
        // Writing "Acme" for Carla names her colleague Dana's company too.
        let update = PersonUpdate { company: Some("Acme".into()), ..PersonUpdate::default() };
        people.update("carla@acme.fr", &update).await.unwrap();
        assert_eq!(people.companies().await.unwrap()[0], Company { name: "Acme".into(), people: 2 });
        let at_acme = people.list(&ListQuery { company: Some("acme".into()), ..ListQuery::default() }).await;
        assert_eq!(emails(&at_acme.unwrap()), ["dana@acme.fr", "carla@acme.fr"]);

        // Recipient suggestions: everyone, robots included, the most frequent first.
        assert_eq!(people.search("", 2).await.unwrap()[0].email, "anna@example.org");
        assert_eq!(people.search("mart", 10).await.unwrap().len(), 1);

        let page = people.person("ANNA@example.org").await.unwrap();
        assert_eq!(page.recent_messages[0].uid, 2, "newest first");
        assert_eq!(page.past.len(), 1);
        assert!(page.last_to.is_some() && page.last_meeting.is_some());
        let carla = people.person("carla@acme.fr").await.unwrap();
        assert_eq!(carla.upcoming.len(), 1);
        assert!(carla.next_meeting.is_some());
    }

    #[tokio::test]
    async fn your_details_merging_and_robots() {
        let (db, store) = mailbox().await;
        store
            .upsert_messages(
                "INBOX",
                &[
                    mail(1, ("Claire", "claire@acme.fr"), &["me@example.org"], 3),
                    mail(2, ("Claire D.", "claire.dubois@gmail.com"), &["me@example.org"], 1),
                    mail(3, ("Shop", "noreply@shop.example"), &["me@example.org"], 1),
                ],
            )
            .await
            .unwrap();
        meeting(&db, "claire.dubois@gmail.com", 3).await;
        let people = db.people();

        let set = |name: &str, company: &str| PersonUpdate {
            name: Some(name.into()),
            company: Some(company.into()),
            role: Some("Ops lead".into()),
            phones: Some(vec![
                Phone { label: "mobile".into(), number: "+33 6 12 34 56 78".into() },
                Phone { label: String::new(), number: "  ".into() },
            ]),
            birthday: Some(Some("--03-14".into())),
            favorite: Some(true),
            ..PersonUpdate::default()
        };
        let claire = people.update("claire@acme.fr", &set("Claire Dubois", "Acme")).await.unwrap();
        assert_eq!(claire.person.name, "Claire Dubois", "your name wins");
        assert_eq!(claire.person.phones.len(), 1, "the empty row is dropped");
        assert!(claire.person.favorite);
        // Only what's sent changes; null clears the birthday.
        let claire = people
            .update("claire@acme.fr", &PersonUpdate { birthday: Some(None), ..PersonUpdate::default() })
            .await
            .unwrap();
        assert_eq!((claire.person.birthday, claire.person.company.as_str()), (None, "Acme"));
        for bad in [
            PersonUpdate { birthday: Some(Some("1990-02-30".into())), ..PersonUpdate::default() },
            PersonUpdate {
                phones: Some(vec![Phone { label: String::new(), number: "call me".into() }]),
                ..PersonUpdate::default()
            },
            PersonUpdate { kind: Some(Some("alien".into())), ..PersonUpdate::default() },
        ] {
            assert!(people.update("claire@acme.fr", &bad).await.is_err());
        }

        // Her Gmail address is the same Claire: one person, everything together.
        let merged = people.merge("claire@acme.fr", "Claire.Dubois@gmail.com").await.unwrap();
        assert_eq!(merged.person.emails, ["claire@acme.fr", "claire.dubois@gmail.com"]);
        assert_eq!((merged.person.messages, merged.upcoming.len()), (2, 1));
        let list = people.list(&ListQuery::default()).await.unwrap();
        assert_eq!(emails(&list), ["claire@acme.fr"], "one entry");
        assert_eq!(people.person("claire.dubois@gmail.com").await.unwrap().person.email, "claire@acme.fr");
        assert!(people.merge("claire@acme.fr", "claire@acme.fr").await.is_err());

        // Two contacts merged: details already set win, empty ones are filled.
        people
            .update(
                "old@acme.fr",
                &PersonUpdate {
                    note: Some("Met in Lyon".into()),
                    company: Some("Old".into()),
                    ..PersonUpdate::default()
                },
            )
            .await
            .unwrap();
        let both = people.merge("claire@acme.fr", "old@acme.fr").await.unwrap();
        assert_eq!((both.person.company.as_str(), both.note.as_str()), ("Acme", "Met in Lyon"));
        assert_eq!(people.contacts().await.unwrap().len(), 1, "the other contact is gone");

        // Unmerge: Gmail is its own person again.
        let back = people.remove_address("claire@acme.fr", "claire.dubois@gmail.com").await.unwrap();
        assert_eq!(back.person.emails, ["claire@acme.fr", "old@acme.fr"]);
        assert!(people.remove_address("claire@acme.fr", "bob@x.fr").await.is_err());
        assert_eq!(people.list(&ListQuery::default()).await.unwrap().len(), 2);

        // Robots: guessed, and as you say.
        assert!(people.person("noreply@shop.example").await.unwrap().person.automated);
        let not_robot = PersonUpdate { kind: Some(Some("person".into())), ..PersonUpdate::default() };
        assert!(!people.update("noreply@shop.example", &not_robot).await.unwrap().person.automated);
        let robot = PersonUpdate { kind: Some(Some("automated".into())), ..PersonUpdate::default() };
        assert!(people.update("claire.dubois@gmail.com", &robot).await.unwrap().person.automated);
    }

    #[tokio::test]
    async fn today_birthdays_and_keep_in_touch() {
        let (db, store) = mailbox().await;
        store
            .upsert_messages(
                "INBOX",
                &[
                    mail(1, ("Lea", "lea@x.fr"), &[], 40),
                    mail(2, ("Tom", "tom@x.fr"), &[], 2),
                    mail(3, ("Ines", "ines@x.fr"), &[], 60),
                ],
            )
            .await
            .unwrap();
        meeting(&db, "ines@x.fr", 3).await;
        let people = db.people();
        let today = chrono::Utc::now().date_naive();
        let in_days = |d: i64| (today + chrono::Duration::days(d)).format("--%m-%d").to_string();
        let fav = |birthday: Option<String>| PersonUpdate {
            favorite: Some(true),
            birthday: Some(birthday),
            ..PersonUpdate::default()
        };
        people.update("lea@x.fr", &fav(Some(in_days(0)))).await.unwrap();
        people.update("tom@x.fr", &fav(Some(in_days(9)))).await.unwrap();
        people.update("ines@x.fr", &fav(None)).await.unwrap();
        people.update("max@x.fr", &fav(Some("1990-01-01".into()))).await.unwrap();

        let t = people.today(today).await.unwrap();
        let born: Vec<(&str, i64)> = t.birthdays.iter().map(|b| (b.person.email.as_str(), b.days)).collect();
        assert_eq!(born, [("lea@x.fr", 0)], "today, not in 9 days");
        // Max never in touch, Lea quiet for 40 days; Tom wrote 2 days ago, Ines has a meeting in 3 days.
        assert_eq!(emails(&t.keep_in_touch), ["max@x.fr", "lea@x.fr"]);
    }
}
