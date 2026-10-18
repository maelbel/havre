//! A lightweight view of an email, for message lists.

use mail_parser::MessageParser;
use serde::Serialize;

/// Someone who sends or receives mail.
///
/// Later this becomes a link to the shared `People` entity, so the same
/// person shows up in Mail, Agenda and every other app.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Person {
    pub name: Option<String>,
    pub address: Option<String>,
}

/// What the message list needs to show one email — no body, no attachments.
/// `Serialize` lets the server send it as JSON to the web UI.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MessageSummary {
    /// IMAP UID: stable id of the message inside its folder.
    pub uid: u32,
    pub from: Option<Person>,
    pub subject: String,
    /// RFC 3339 date, e.g. `2026-10-14T09:42:00+02:00`.
    pub date: Option<String>,
    pub unread: bool,
    /// Starred (the IMAP `\Flagged` flag).
    pub flagged: bool,
    /// The `Message-ID` header, without `<>`: the same in every folder.
    /// Kept on the server side (labels, undo); the list doesn't need it.
    #[serde(skip)]
    pub message_id: Option<String>,
    /// Havre labels on it (their ids).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<i64>,
    /// Snoozed: hidden until then (unix seconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<i64>,
    /// The first words of the message (once its body was downloaded).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// Has attachments (files, not embedded images).
    pub attachments: bool,
    /// Carries a meeting invitation.
    pub invite: bool,
    /// When that meeting starts (unix seconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_at: Option<i64>,
    /// The account it belongs to (set in the unified inbox).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<i64>,
    /// Its folder (set in the unified inbox).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

impl MessageSummary {
    /// Builds a summary from raw RFC 5322 headers.
    ///
    /// `mail-parser` decodes encoded words for us, so a subject like
    /// `=?UTF-8?Q?D=C3=A9jeuner?=` becomes `Déjeuner`.
    pub fn from_headers(uid: u32, unread: bool, raw_headers: &[u8]) -> Self {
        let parsed = MessageParser::default().parse_headers(raw_headers);

        // `parsed` is an Option: headers that can't be parsed at all still
        // give us a row in the list, just with an empty subject.
        let Some(message) = parsed else {
            return Self { uid, unread, ..Default::default() };
        };

        let from = message.from().and_then(|list| list.first()).map(|addr| Person {
            name: addr.name().map(str::to_owned),
            address: addr.address().map(str::to_owned),
        });

        Self {
            uid,
            from,
            subject: message.subject().unwrap_or_default().to_owned(),
            date: message.date().map(|d| d.to_rfc3339()),
            unread,
            // `Some("")` = read, and there is none (so we don't look again).
            message_id: Some(message.message_id().unwrap_or_default().to_owned()),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_encoded_subject_and_sender() {
        let raw = b"From: =?UTF-8?Q?Ana=C3=AFs_Leroy?= <anais@example.org>\r\n\
Subject: =?UTF-8?Q?D=C3=A9jeuner_jeudi_=3F?=\r\n\
Date: Wed, 14 Oct 2026 09:42:00 +0200\r\n\r\n";

        let summary = MessageSummary::from_headers(42, true, raw);

        assert_eq!(summary.uid, 42);
        assert_eq!(summary.subject, "Déjeuner jeudi ?");
        assert!(summary.unread);
        let from = summary.from.expect("sender should be parsed");
        assert_eq!(from.name.as_deref(), Some("Anaïs Leroy"));
        assert_eq!(from.address.as_deref(), Some("anais@example.org"));
        assert_eq!(summary.date.as_deref(), Some("2026-10-14T09:42:00+02:00"));
    }

    #[test]
    fn survives_missing_headers() {
        let summary = MessageSummary::from_headers(1, false, b"X-Nothing: here\r\n\r\n");
        assert_eq!(summary.subject, "");
        assert!(summary.from.is_none());
    }

    #[test]
    fn reads_the_message_id() {
        let raw = b"Message-ID: <abc.123@mail.example.com>\r\nSubject: Hi\r\n\r\n";
        let summary = MessageSummary::from_headers(3, true, raw);
        assert_eq!(summary.message_id.as_deref(), Some("abc.123@mail.example.com"));
        // None at all: "" (read, and there is none), so the sync doesn't ask again.
        let none = MessageSummary::from_headers(4, true, b"Subject: Hi\r\n\r\n");
        assert_eq!(none.message_id.as_deref(), Some(""));
    }

    /// A message with a quoted reply, an attachment and an invitation.
    const INVITE: &str = "From: Claire <claire@example.com>\r\n\
Subject: Kick-off\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b\"\r\n\
\r\n\
--b\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Bonjour Mael,\r\n\
Je vous propose jeudi.\r\n\
\r\n\
Le mar. 13 oct. 2026, Mael a écrit :\r\n\
> une vieille ligne\r\n\
--b\r\n\
Content-Type: application/pdf; name=\"agenda.pdf\"\r\n\
Content-Disposition: attachment; filename=\"agenda.pdf\"\r\n\
\r\n\
%PDF-1.4\r\n\
--b\r\n\
Content-Type: text/calendar; method=REQUEST; charset=utf-8\r\n\
\r\n\
BEGIN:VCALENDAR\r\n\
METHOD:REQUEST\r\n\
BEGIN:VEVENT\r\n\
UID:kick-off@example.com\r\n\
DTSTART:20261022T080000Z\r\n\
DTEND:20261022T093000Z\r\n\
SUMMARY:Kick-off\r\n\
ORGANIZER:mailto:claire@example.com\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n\
--b--\r\n";

    #[test]
    fn digest_reads_preview_files_and_invitation() {
        let d = Digest::of(INVITE.as_bytes());
        // The quoted reply is left out of the preview.
        assert_eq!(d.preview, "Bonjour Mael, Je vous propose jeudi.");
        let names: Vec<&str> = d.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["agenda.pdf", "invitation.ics"]);
        assert!(d.invite);
        // 2026-10-22 08:00 UTC.
        assert_eq!(d.invite_at, Some(1_792_656_000));
    }

    #[test]
    fn an_answer_is_not_an_invitation() {
        let reply = INVITE.replace("METHOD:REQUEST", "METHOD:REPLY");
        let d = Digest::of(reply.as_bytes());
        assert!(!d.invite);
        assert_eq!(d.invite_at, None);
    }
}

/// What the list shows about a message once its body is known.
#[derive(Debug, Clone, Default, PartialEq, Serialize, serde::Deserialize)]
pub struct Digest {
    /// About two lines of the message's own words (quotes left out).
    pub preview: String,
    /// Real attachments (not embedded images).
    pub files: Vec<FileInfo>,
    /// Carries a meeting invitation (`text/calendar`), not an answer to one.
    pub invite: bool,
    /// When the invited meeting starts (unix seconds), if the invitation reads.
    pub invite_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct FileInfo {
    /// Its number among the message's attachments (to download it).
    #[serde(default)]
    pub index: usize,
    pub name: String,
    pub mime: String,
    pub size: usize,
}

impl Digest {
    /// Reads a raw email.
    pub fn of(raw: &[u8]) -> Self {
        use mail_parser::MimeHeaders;
        let Some(m) = MessageParser::default().parse(raw) else { return Self::default() };
        let text = m
            .body_text(0)
            .map(|t| t.into_owned())
            .or_else(|| m.body_html(0).map(|h| strip_tags(&h)))
            .unwrap_or_default();
        let own: Vec<&str> = text
            .lines()
            .map(str::trim)
            .take_while(|l| {
                !(l.starts_with("On ") && l.ends_with("wrote:"))
                    && !(l.starts_with("Le ") && l.ends_with("a écrit :"))
                    && !l.starts_with("-----Original Message")
                    && !l.starts_with("---------- Forwarded")
                    && *l != "--"
            })
            .filter(|l| !l.starts_with('>') && !l.is_empty())
            .collect();
        let preview: String =
            own.join(" ").split_whitespace().collect::<Vec<_>>().join(" ").chars().take(220).collect();
        let mut files = Vec::new();
        let mut invite = false;
        let mut invite_at = None;
        for (index, part) in m.attachments().enumerate() {
            let mime = part
                .content_type()
                .map(|ct| match ct.subtype() {
                    Some(sub) => format!("{}/{}", ct.ctype(), sub).to_lowercase(),
                    None => ct.ctype().to_lowercase(),
                })
                .unwrap_or_default();
            let name = part.attachment_name().map(str::to_owned);
            if mime == "text/calendar" || name.as_deref().is_some_and(|n| n.to_lowercase().ends_with(".ics"))
            {
                // An answer (METHOD:REPLY, "Claire accepted") is not an invitation.
                let text = String::from_utf8_lossy(part.contents());
                match crate::agenda::ics::read_invitation(&text) {
                    Ok(inv) if inv.method == "REPLY" || inv.method == "COUNTER" => {}
                    Ok(inv) => {
                        invite = true;
                        invite_at = invite_at.or(inv.events.first().map(|e| e.start));
                    }
                    Err(_) => invite = true,
                }
            }
            // Embedded images (logos, signatures) aren't files.
            if part.content_id().is_some() && mime.starts_with("image/") {
                continue;
            }
            files.push(FileInfo {
                index,
                name: name.unwrap_or_else(|| {
                    if mime == "text/calendar" { "invitation.ics".into() } else { "attachment".into() }
                }),
                mime,
                size: part.contents().len(),
            });
        }
        Self { preview, files, invite, invite_at }
    }
}

/// HTML → rough text, for a preview only.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let lower = html.to_lowercase();
    // Skip <style> and <script> contents.
    let mut skip_until: Option<&str> = None;
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if let Some(end) = skip_until {
            if lower[i..].starts_with(end) {
                skip_until = None;
                i += end.len();
            } else {
                i += 1;
            }
            continue;
        }
        if lower[i..].starts_with("<style") {
            skip_until = Some("</style>");
            continue;
        }
        if lower[i..].starts_with("<script") {
            skip_until = Some("</script>");
            continue;
        }
        let c = bytes[i] as char;
        match c {
            '<' => {
                in_tag = true;
                if lower[i..].starts_with("<br")
                    || lower[i..].starts_with("<p")
                    || lower[i..].starts_with("<div")
                {
                    out.push('\n');
                }
            }
            '>' => in_tag = false,
            _ if !in_tag => {
                // Copy the whole UTF-8 character.
                let ch = html[i..].chars().next().unwrap_or(' ');
                out.push(ch);
                i += ch.len_utf8();
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
}
