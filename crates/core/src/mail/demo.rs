//! Sample data, so the UI can be developed without a real mailbox.
//! Enabled with `HAVRE_DEMO=1`.
//!
//! The sample messages are real RFC 5322 emails, parsed by the same code as
//! real mail — so the demo also exercises the parser.

use std::collections::{HashMap, HashSet};

use super::{ActionOutcome, Folder, FolderRole, ListQuery, MessagePage, MessageSummary, Person};
use crate::store::FolderInfo;

fn sample_folders() -> Vec<Folder> {
    let f = |path: &str, name: &str, depth, role| Folder {
        path: path.into(),
        name: name.into(),
        depth,
        role,
        declared_role: true,
    };
    vec![
        f("INBOX", "Inbox", 0, FolderRole::Inbox),
        f("Drafts", "Drafts", 0, FolderRole::Drafts),
        f("Sent", "Sent", 0, FolderRole::Sent),
        f("Archive", "Archive", 0, FolderRole::Archive),
        f("Junk", "Junk", 0, FolderRole::Junk),
        f("Trash", "Trash", 0, FolderRole::Trash),
        f("Clients", "Clients", 0, FolderRole::Other),
        f("Clients/Lyon", "Lyon", 1, FolderRole::Other),
    ]
}

/// The demo mailbox, in memory. Archive, delete, read/unread all work,
/// until the server restarts.
pub struct DemoStore {
    /// Which folder each sample message is in.
    location: HashMap<u32, String>,
    unread: HashSet<u32>,
}

impl Default for DemoStore {
    fn default() -> Self {
        Self {
            location: SAMPLES.iter().map(|s| (s.uid, "INBOX".to_owned())).collect(),
            unread: SAMPLES.iter().filter(|s| s.unread).map(|s| s.uid).collect(),
        }
    }
}

impl DemoStore {
    /// Folders with their counters, like the real store gives them.
    pub fn folders(&self) -> Vec<FolderInfo> {
        sample_folders()
            .into_iter()
            .map(|folder| {
                let here: Vec<u32> =
                    self.location.iter().filter(|(_, f)| **f == folder.path).map(|(uid, _)| *uid).collect();
                let unread = here.iter().filter(|uid| self.unread.contains(uid)).count() as u32;
                FolderInfo { folder, unread, total: here.len() as u32 }
            })
            .collect()
    }

    pub fn page(&self, folder: &str, query: &ListQuery) -> MessagePage {
        let needle = query.search.as_deref().map(str::to_lowercase).filter(|n| !n.trim().is_empty());
        let mut matching: Vec<&Sample> = SAMPLES
            .iter()
            .filter(|s| self.location.get(&s.uid).is_some_and(|f| f == folder))
            .filter(|s| query.before_uid.is_none_or(|before| s.uid < before))
            .filter(|s| {
                needle.as_ref().is_none_or(|n| {
                    [s.from_name, s.subject, s.body].iter().any(|v| v.to_lowercase().contains(n.as_str()))
                })
            })
            .collect();
        matching.sort_by_key(|s| std::cmp::Reverse(s.uid));
        let limit = query.limit.max(1) as usize;
        let has_more = matching.len() > limit;
        let messages = matching
            .into_iter()
            .take(limit)
            .map(|s| MessageSummary {
                uid: s.uid,
                from: Some(Person { name: Some(s.from_name.into()), address: Some(s.from_address.into()) }),
                subject: s.subject.into(),
                date: Some(s.date.into()),
                unread: self.unread.contains(&s.uid),
                ..Default::default()
            })
            .collect();
        MessagePage { messages, has_more }
    }

    /// The raw email behind a sample, as a server would send it.
    pub fn raw(&self, folder: &str, uid: u32) -> Option<Vec<u8>> {
        if self.location.get(&uid)? != folder {
            return None;
        }
        let s = SAMPLES.iter().find(|s| s.uid == uid)?;
        let mut raw = format!(
            "From: {} <{}>\r\nTo: Maël Belliard <mael@example.org>\r\nSubject: {}\r\nDate: {}\r\nMIME-Version: 1.0\r\n",
            s.from_name,
            s.from_address,
            s.subject,
            rfc2822(s.date)
        );
        raw.push_str(s.body);
        Some(raw.into_bytes())
    }

    pub fn is_unread(&self, uid: u32) -> bool {
        self.unread.contains(&uid)
    }

    pub fn set_seen(&mut self, uid: u32, seen: bool) {
        if seen {
            self.unread.remove(&uid);
        } else {
            self.unread.insert(uid);
        }
    }

    pub fn move_to(&mut self, from: &str, uids: &[u32], to: &str) -> ActionOutcome {
        for uid in uids {
            if let Some(folder) = self.location.get_mut(uid).filter(|f| *f == from) {
                *folder = to.to_owned();
            }
        }
        ActionOutcome::Moved { to: to.to_owned() }
    }

    pub fn archive(&mut self, from: &str, uids: &[u32]) -> ActionOutcome {
        self.move_to(from, uids, "Archive")
    }

    pub fn trash(&mut self, from: &str, uids: &[u32]) -> ActionOutcome {
        if from == "Trash" {
            for uid in uids {
                self.location.remove(uid);
            }
            ActionOutcome::Deleted
        } else {
            self.move_to(from, uids, "Trash")
        }
    }
}

struct Sample {
    uid: u32,
    from_name: &'static str,
    from_address: &'static str,
    subject: &'static str,
    date: &'static str,
    unread: bool,
    /// Content-Type header + blank line + body.
    body: &'static str,
}

/// `2026-10-14T09:42:00+02:00` → `Wed, 14 Oct 2026 09:42:00 +0200` (good enough for samples).
fn rfc2822(iso: &str) -> String {
    let dt = mail_parser::DateTime::parse_rfc3339(iso).unwrap_or_default();
    dt.to_rfc822()
}

const TEXT: &str = "Content-Type: text/plain; charset=utf-8\r\n\r\n";

const SAMPLES: &[Sample] = &[
    Sample {
        uid: 108,
        from_name: "Claire Dubois",
        from_address: "claire@atelier-lyon.example",
        subject: "Kick-off atelier Lyon — invitation",
        date: "2026-10-14T09:42:00+02:00",
        unread: true,
        body: "Content-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n\
--b1\r\nContent-Type: multipart/alternative; boundary=\"b2\"\r\n\r\n\
--b2\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
Bonjour Maël,\r\n\r\nComme convenu au téléphone, je vous propose de lancer l'atelier jeudi 22 octobre à 10h.\r\n\r\nBien à vous,\r\nClaire\r\n\
--b2\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<div style=\"font-family:Georgia,serif;font-size:15px;line-height:1.6;color:#222\">\
<p>Bonjour Maël,</p>\
<p>Comme convenu au téléphone, je vous propose de lancer l’atelier <b>jeudi 22 octobre à 10h</b>, dans nos locaux de la rue de la Charité.</p>\
<p>L’idée est de valider ensemble le périmètre de la phase 1 et le planning des interventions. J’ai joint l’ordre du jour — n’hésitez pas à me dire s’il manque un point.</p>\
<p>Bien à vous,<br>Claire</p>\
<img src=\"https://newsletter.example/open.gif\" width=\"1\" height=\"1\">\
</div>\r\n\
--b2--\r\n\
--b1\r\nContent-Type: application/pdf; name=\"ordre-du-jour.pdf\"\r\nContent-Disposition: attachment; filename=\"ordre-du-jour.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n\
JVBERi0xLjQKJcK1wrYKMSAwIG9iago8PC9UeXBlL0NhdGFsb2c+PgplbmRvYmoKdHJhaWxlcjw8L1Jvb3QgMSAwIFI+PgolJUVPRgo=\r\n\
--b1--\r\n",
    },
    Sample {
        uid: 107,
        from_name: "Thomas Renard",
        from_address: "thomas@lmcontrol.example",
        subject: "Re: Devis maintenance Q4",
        date: "2026-10-14T08:15:00+02:00",
        unread: true,
        body: "Content-Type: text/plain; charset=utf-8\r\n\r\nBonjour Maël,\r\n\r\nJe vous renvoie la version corrigée avec la remise appliquée.\r\nOn en parle jeudi à 10h ?\r\n\r\nThomas\r\n",
    },
    Sample {
        uid: 106,
        from_name: "Forgejo",
        from_address: "noreply@git.example",
        subject: "[havre] CI passed on main",
        date: "2026-10-14T07:58:00+02:00",
        unread: false,
        body: "Content-Type: text/plain; charset=utf-8\r\n\r\nAll checks have passed on main.\r\n\r\ncore: read full messages (#15)\r\n",
    },
    Sample {
        uid: 105,
        from_name: "Julien Morel",
        from_address: "julien@example.org",
        subject: "Déjeuner jeudi ?",
        date: "2026-10-13T18:20:00+02:00",
        unread: true,
        body: "Content-Type: text/plain; charset=utf-8\r\n\r\nSalut !\r\n\r\nÇa te dit jeudi midi vers 12h30, chez Léon ?\r\n\r\nJulien\r\n",
    },
    Sample {
        uid: 104,
        from_name: "OVHcloud",
        from_address: "billing@ovh.example",
        subject: "Your invoice for October",
        date: "2026-10-13T11:02:00+02:00",
        unread: false,
        body: TEXT,
    },
    Sample {
        uid: 103,
        from_name: "Sophie Martin",
        from_address: "sophie@example.org",
        subject: "Photos du week-end",
        date: "2026-10-13T09:15:00+02:00",
        unread: false,
        body: "Content-Type: text/plain; charset=utf-8\r\n\r\nCoucou,\r\n\r\nJe t'ai mis les photos de dimanche, celle du lac est superbe.\r\n\r\nBises,\r\nSophie\r\n",
    },
    Sample {
        uid: 102,
        from_name: "Anaïs Leroy",
        from_address: "anais@lmcontrol.example",
        subject: "Compte-rendu réunion sécurité",
        date: "2026-10-12T16:40:00+02:00",
        unread: false,
        body: "Content-Type: text/plain; charset=utf-8\r\n\r\nBonjour à tous,\r\n\r\nVoici le CR de lundi. Points ouverts : rotation des clés, sauvegardes.\r\n\r\nAnaïs\r\n",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn inbox(store: &DemoStore, search: Option<&str>) -> MessagePage {
        let query = ListQuery { limit: 3, before_uid: None, search: search.map(str::to_owned) };
        store.page("INBOX", &query)
    }

    #[test]
    fn pages_and_searches() {
        let store = DemoStore::default();
        let first = inbox(&store, None);
        assert_eq!(first.messages.len(), 3);
        assert!(first.has_more);

        let older = store.page("INBOX", &ListQuery { limit: 50, before_uid: Some(106), search: None });
        assert!(older.messages.iter().all(|m| m.uid < 106));
        assert!(!older.has_more);

        let found = inbox(&store, Some("léon"));
        assert_eq!(found.messages.iter().map(|m| m.uid).collect::<Vec<_>>(), [105]);
    }

    #[test]
    fn archive_and_delete() {
        let mut store = DemoStore::default();
        store.archive("INBOX", &[108]);
        assert!(store.raw("INBOX", 108).is_none());
        assert!(store.raw("Archive", 108).is_some());

        assert_eq!(store.trash("INBOX", &[107]), ActionOutcome::Moved { to: "Trash".into() });
        assert_eq!(store.trash("Trash", &[107]), ActionOutcome::Deleted);
        assert!(store.raw("Trash", 107).is_none());
    }
}
