//! A full email: headers, body and attachments.

use std::collections::HashMap;

use mail_parser::{Address, MessageParser, MimeHeaders, PartType};
use serde::Serialize;

use super::{Person, sanitize::sanitize_html};

/// One attachment, without its content (downloaded separately).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Attachment {
    /// Position among the message's attachments; used to download it.
    pub index: usize,
    pub filename: String,
    /// MIME type, e.g. `application/pdf`.
    pub mime: String,
    /// Size in bytes (decoded).
    pub size: usize,
    /// Shown inside the HTML body (an embedded image) rather than as a file.
    pub inline: bool,
}

/// Everything the reader view needs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MessageDetail {
    pub uid: u32,
    pub folder: String,
    pub from: Option<Person>,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub subject: String,
    pub date: Option<String>,
    /// Plain-text body (always present when the email has any text).
    pub text: Option<String>,
    /// Sanitized HTML body, when the email has one.
    pub html: Option<String>,
    /// Remote images were removed from `html` (privacy). The UI can offer
    /// "Load images", which asks again with `allow_remote_images`.
    pub remote_images_blocked: bool,
    pub attachments: Vec<Attachment>,
    pub unread: bool,
}

/// Options for [`MessageDetail::parse`].
pub struct RenderOptions<'a> {
    pub allow_remote_images: bool,
    /// Builds the URL of attachment number `index`, for embedded images.
    /// The core does not know about HTTP, so the caller decides.
    pub attachment_url: &'a (dyn Fn(usize) -> String + Sync),
}

impl MessageDetail {
    /// Parses a raw RFC 5322 message (the bytes of `BODY[]`).
    pub fn parse(uid: u32, folder: &str, unread: bool, raw: &[u8], options: &RenderOptions<'_>) -> Self {
        let Some(message) = MessageParser::default().parse(raw) else {
            return Self {
                uid,
                folder: folder.to_owned(),
                from: None,
                to: vec![],
                cc: vec![],
                subject: String::new(),
                date: None,
                text: Some(String::from_utf8_lossy(raw).into_owned()),
                html: None,
                remote_images_blocked: false,
                attachments: vec![],
                unread,
            };
        };

        // Attachments, and a map Content-ID → URL for embedded images.
        let mut inline_images = HashMap::new();
        let attachments: Vec<Attachment> = message
            .attachments()
            .enumerate()
            .map(|(index, part)| {
                let mime = part
                    .content_type()
                    .map(|ct| match ct.subtype() {
                        Some(sub) => format!("{}/{}", ct.ctype(), sub),
                        None => ct.ctype().to_owned(),
                    })
                    .unwrap_or_else(|| "application/octet-stream".into());
                let content_id = part.content_id().map(|id| id.trim_matches(['<', '>']).to_owned());
                let inline = content_id.is_some() && mime.starts_with("image/");
                if let (true, Some(id)) = (inline, content_id) {
                    inline_images.insert(id, (options.attachment_url)(index));
                }
                Attachment {
                    index,
                    filename: part
                        .attachment_name()
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("attachment-{}", index + 1)),
                    mime,
                    size: part.contents().len(),
                    inline,
                }
            })
            .collect();

        // `html_part(0)` can be a text part converted to HTML by mail-parser;
        // we only want real HTML here, the text is sent separately.
        let real_html = message.html_part(0).and_then(|part| match &part.body {
            PartType::Html(html) => Some(html.as_ref()),
            _ => None,
        });
        let sanitized = real_html.map(|html| sanitize_html(html, inline_images, options.allow_remote_images));

        Self {
            uid,
            folder: folder.to_owned(),
            from: message.from().and_then(|a| people(a).into_iter().next()),
            to: message.to().map(people).unwrap_or_default(),
            cc: message.cc().map(people).unwrap_or_default(),
            subject: message.subject().unwrap_or_default().to_owned(),
            date: message.date().map(|d| d.to_rfc3339()),
            text: message.body_text(0).map(|t| t.into_owned()),
            remote_images_blocked: sanitized.as_ref().is_some_and(|s| s.blocked_remote_images),
            html: sanitized.map(|s| s.html),
            attachments,
            unread,
        }
    }
}

/// Decoded content of one attachment, ready to download.
pub struct AttachmentData {
    pub filename: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// Extracts attachment number `index` from a raw message.
pub fn extract_attachment(raw: &[u8], index: usize) -> Option<AttachmentData> {
    let message = MessageParser::default().parse(raw)?;
    let part = message.attachments().nth(index)?;
    Some(AttachmentData {
        filename: part
            .attachment_name()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("attachment-{}", index + 1)),
        mime: part
            .content_type()
            .map(|ct| match ct.subtype() {
                Some(sub) => format!("{}/{}", ct.ctype(), sub),
                None => ct.ctype().to_owned(),
            })
            .unwrap_or_else(|| "application/octet-stream".into()),
        bytes: part.contents().to_vec(),
    })
}

/// Turns a `From:`/`To:` header (a list, or groups of lists) into people.
fn people(address: &Address<'_>) -> Vec<Person> {
    address
        .iter()
        .map(|a| Person { name: a.name().map(str::to_owned), address: a.address().map(str::to_owned) })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"From: Claire Dubois <claire@example.org>\r
To: Mael <mael@example.org>, Thomas <thomas@example.org>\r
Subject: =?UTF-8?Q?Kick-off_=E2=80=94_invitation?=\r
Date: Wed, 14 Oct 2026 09:42:00 +0200\r
MIME-Version: 1.0\r
Content-Type: multipart/mixed; boundary=\"outer\"\r
\r
--outer\r
Content-Type: multipart/related; boundary=\"rel\"\r
\r
--rel\r
Content-Type: multipart/alternative; boundary=\"alt\"\r
\r
--alt\r
Content-Type: text/plain; charset=utf-8\r
\r
Bonjour Ma\xc3\xabl,\r
--alt\r
Content-Type: text/html; charset=utf-8\r
\r
<p>Bonjour <b>Ma\xc3\xabl</b></p><img src=\"cid:logo@x\"><img src=\"https://t.example/p.gif\"><script>x()</script>\r
--alt--\r
--rel\r
Content-Type: image/png\r
Content-ID: <logo@x>\r
Content-Transfer-Encoding: base64\r
\r
iVBORw0KGgo=\r
--rel--\r
--outer\r
Content-Type: application/pdf; name=\"ordre-du-jour.pdf\"\r
Content-Disposition: attachment; filename=\"ordre-du-jour.pdf\"\r
Content-Transfer-Encoding: base64\r
\r
JVBERi0xLjQK\r
--outer--\r
";

    fn url(i: usize) -> String {
        format!("/att/{i}")
    }

    #[test]
    fn parses_bodies_attachments_and_people() {
        let options = RenderOptions { allow_remote_images: false, attachment_url: &url };
        let m = MessageDetail::parse(7, "INBOX", true, SAMPLE, &options);

        assert_eq!(m.subject, "Kick-off — invitation");
        assert_eq!(m.to.len(), 2);
        assert_eq!(m.text.as_deref().map(str::trim), Some("Bonjour Maël,"));

        let html = m.html.expect("html body");
        assert!(html.contains("<b>Maël</b>"));
        assert!(!html.contains("script"));
        assert!(html.contains(r#"src="/att/0""#), "inline image rewritten: {html}");
        assert!(m.remote_images_blocked);

        assert_eq!(m.attachments.len(), 2);
        assert!(m.attachments[0].inline);
        let pdf = &m.attachments[1];
        assert_eq!(
            (pdf.filename.as_str(), pdf.mime.as_str(), pdf.inline),
            ("ordre-du-jour.pdf", "application/pdf", false)
        );
    }

    #[test]
    fn extracts_attachment_bytes() {
        let pdf = extract_attachment(SAMPLE, 1).expect("attachment");
        assert_eq!(pdf.bytes, b"%PDF-1.4\n");
        assert_eq!(pdf.mime, "application/pdf");
        assert!(extract_attachment(SAMPLE, 9).is_none());
    }
}
