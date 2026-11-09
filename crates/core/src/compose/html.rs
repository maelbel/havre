//! Rich text in messages (step 25): the HTML you write, kept safe, and the
//! plain text made from it.
//!
//! - [`clean`]: the browser (or another app's draft) sends HTML; only a short
//!   list of tags is kept ([`TAGS`]), the rest removed (their text stays).
//!   `ammonia` does it, as for reading mail. Never trust what a page sends.
//! - [`to_text`]: the plain text of a message (its `text/plain` part, search,
//!   the AI): `- ` for lists, `text <https://…>` for links, `> ` for quotes.
//!   It reads the HTML [`clean`] wrote: few tags, always written the same way,
//!   so a small hand-written reader is enough (no HTML parser to add).
//! - [`from_text`]: the other way, for plain drafts and the AI's text.
//! - [`for_sending`]: the HTML part as sent, with inline styles (mail apps
//!   ignore style sheets).

use std::collections::HashSet;

/// What a message may contain. A `div` only holds the signature
/// (`data-signature`).
pub const TAGS: [&str; 14] =
    ["p", "br", "strong", "b", "em", "i", "u", "s", "a", "ul", "ol", "li", "blockquote", "div"];

/// Keeps only [`TAGS`], `href` on links (`https`, `http`, `mailto`), and the
/// signature's marker.
pub fn clean(html: &str) -> String {
    ammonia::Builder::default()
        .tags(HashSet::from(TAGS))
        .generic_attributes(HashSet::new())
        .tag_attributes(
            [("a", HashSet::from(["href"])), ("div", HashSet::from(["data-signature"]))]
                .into_iter()
                .collect(),
        )
        .url_schemes(HashSet::from(["https", "http", "mailto"]))
        // A relative link means nothing in a mail: removed.
        .url_relative(ammonia::UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer"))
        .clean(html)
        .to_string()
}

/// Plain text as HTML paragraphs: an empty line separates paragraphs, a
/// single line break stays one, `> ` lines become a quote and `- ` / `* `
/// lines a list.
pub fn from_text(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut html = String::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with('>') {
            // The quoted lines, without one level of `>`, are a quote of their own.
            let mut inner = Vec::new();
            while i < lines.len() && lines[i].starts_with('>') {
                let l = &lines[i][1..];
                inner.push(l.strip_prefix(' ').unwrap_or(l));
                i += 1;
            }
            html.push_str(&format!("<blockquote>{}</blockquote>", from_text(&inner.join("\n"))));
        } else if is_item(line) {
            html.push_str("<ul>");
            while i < lines.len() && is_item(lines[i]) {
                html.push_str(&format!("<li><p>{}</p></li>", escape(lines[i].trim_start()[2..].trim())));
                i += 1;
            }
            html.push_str("</ul>");
        } else if line.trim().is_empty() {
            i += 1;
        } else {
            let mut para = Vec::new();
            while i < lines.len()
                && !lines[i].trim().is_empty()
                && !lines[i].starts_with('>')
                && !is_item(lines[i])
            {
                para.push(escape(lines[i]));
                i += 1;
            }
            html.push_str(&format!("<p>{}</p>", para.join("<br>")));
        }
    }
    html
}

fn is_item(line: &str) -> bool {
    let l = line.trim_start();
    l.starts_with("- ") || l.starts_with("* ")
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `&amp;` → `&`, `&#233;` → `é`… (what `ammonia` writes, and a few more).
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => {
                let code = match name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => name.strip_prefix('#').and_then(|n| n.parse().ok()),
                };
                code.and_then(char::from_u32)
            }
        };
        match ch {
            Some(c) => {
                // A no-break space is a space in plain text.
                out.push(if c == '\u{a0}' { ' ' } else { c });
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out.replace('\u{a0}', " ")
}

// ---- HTML → plain text -------------------------------------------------------------

/// One piece of the HTML: a tag or some text.
#[derive(Debug, PartialEq)]
enum Token<'a> {
    Open { name: String, href: Option<String> },
    Close(String),
    Text(&'a str),
}

/// Cuts HTML into tags and text. Fine for what [`clean`] writes (no
/// comments, no scripts, attributes in double quotes).
fn tokens(html: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        match rest.find('<') {
            Some(0) => {
                let end = rest.find('>').unwrap_or(rest.len() - 1);
                let inside = rest[1..end].trim().trim_end_matches('/').trim();
                if let Some(name) = inside.strip_prefix('/') {
                    out.push(Token::Close(name.trim().to_lowercase()));
                } else {
                    let name: String = inside
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric())
                        .collect::<String>()
                        .to_lowercase();
                    let href = inside
                        .find("href=\"")
                        .map(|at| &inside[at + 6..])
                        .and_then(|v| v.find('"').map(|end| unescape(&v[..end])));
                    out.push(Token::Open { name, href });
                }
                rest = &rest[end + 1..];
            }
            Some(at) => {
                out.push(Token::Text(&rest[..at]));
                rest = &rest[at..];
            }
            None => {
                out.push(Token::Text(rest));
                rest = "";
            }
        }
    }
    out
}

/// Writes plain text line by line, with the quote marks in front of each
/// line and blank lines between blocks.
struct Writer {
    out: String,
    /// Nothing written on the current line yet.
    at_line_start: bool,
    /// Quote depth: each line starts with that many `> `.
    quote: usize,
    /// A blank line is due before the next text, with this prefix (the quote
    /// marks of where the block ended, not of where the next one starts).
    blank: Option<String>,
}

impl Writer {
    fn prefix(&self) -> String {
        "> ".repeat(self.quote)
    }

    /// Starts the line if needed (blank line first when one is due).
    fn begin_line(&mut self) {
        if !self.at_line_start {
            return;
        }
        if let Some(prefix) = self.blank.take()
            && !self.out.is_empty()
        {
            self.out.push_str(prefix.trim_end());
            self.out.push('\n');
        }
        let prefix = self.prefix();
        self.out.push_str(&prefix);
        self.at_line_start = false;
    }

    fn text(&mut self, text: &str) {
        // HTML spacing: runs of white space are one space.
        // A space is due before the next character; written unless the line
        // has nothing yet or already ends with one ("<b>a</b> <i>b</i>": the
        // text between the tags is only that space, and it counts).
        let mut collapsed = String::new();
        let mut space = false;
        let wants_space =
            |collapsed: &str| !collapsed.is_empty() || (!self.at_line_start && !self.out.ends_with(' '));
        for c in text.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                space = true;
                continue;
            }
            if space && wants_space(&collapsed) {
                collapsed.push(' ');
            }
            space = false;
            collapsed.push(c);
        }
        if space && wants_space(&collapsed) {
            collapsed.push(' ');
        }
        if collapsed.is_empty() || (self.at_line_start && collapsed.trim().is_empty()) {
            return;
        }
        self.begin_line();
        self.out.push_str(&unescape(&collapsed));
    }

    /// Writes as is (a list marker).
    fn raw(&mut self, text: &str) {
        self.begin_line();
        self.out.push_str(text);
    }

    fn line_break(&mut self) {
        if self.at_line_start {
            // An empty line (`<br>` on its own): keep it.
            self.begin_line();
        }
        while self.out.ends_with(' ') {
            self.out.pop();
        }
        self.out.push('\n');
        self.at_line_start = true;
    }

    /// The end (or start) of a paragraph, list or quote.
    fn block(&mut self) {
        if !self.at_line_start {
            self.line_break();
        }
        // The first block boundary decides the blank line's prefix (a quote
        // opening right after a paragraph: the blank line is outside it).
        // None just after the signature's `-- ` line.
        if self.blank.is_none() && !self.on_signature_line() {
            self.blank = Some(self.prefix());
        }
    }

    /// The signature's `-- ` line: its lines follow without a blank line.
    fn on_signature_line(&self) -> bool {
        let last = self.out.trim_end_matches('\n').rsplit('\n').next().unwrap_or_default();
        last.trim_start_matches(['>', ' ']).trim() == "--"
    }
}

/// The plain text of a message's HTML (see the module's notes).
pub fn to_text(html: &str) -> String {
    let mut w = Writer { out: String::new(), at_line_start: true, quote: 0, blank: None };
    // Each open list: `None` for bullets, `Some(n)` for the next number.
    let mut lists: Vec<Option<usize>> = Vec::new();
    // Each open link: its address and where its text started.
    let mut links: Vec<(Option<String>, usize)> = Vec::new();
    for token in tokens(html) {
        match token {
            Token::Text(text) => w.text(text),
            Token::Open { name, href } => match name.as_str() {
                "br" => w.line_break(),
                "p" | "div" if !lists.is_empty() => {
                    // In a list item: its first paragraph is on the item's line
                    // (after "- "), the next ones on lines of their own, under it.
                    if w.at_line_start {
                        w.raw(&"  ".repeat(lists.len()));
                    }
                }
                "p" | "div" => w.block(),
                "blockquote" => {
                    w.block();
                    w.quote += 1;
                }
                "ul" | "ol" => {
                    if lists.is_empty() {
                        w.block();
                    } else if !w.at_line_start {
                        w.line_break();
                    }
                    lists.push(if name == "ol" { Some(1) } else { None });
                }
                "li" => {
                    if !w.at_line_start {
                        w.line_break();
                    }
                    let indent = "  ".repeat(lists.len().saturating_sub(1));
                    let marker = match lists.last_mut() {
                        Some(Some(n)) => {
                            *n += 1;
                            format!("{indent}{}. ", *n - 1)
                        }
                        _ => format!("{indent}- "),
                    };
                    w.raw(&marker);
                }
                "a" => links.push((href, w.out.len())),
                _ => {}
            },
            Token::Close(name) => match name.as_str() {
                "p" if w.on_signature_line() => w.line_break(),
                "p" | "div" if !lists.is_empty() => {
                    if !w.at_line_start {
                        w.line_break();
                    }
                }
                "p" | "div" => w.block(),
                "blockquote" => {
                    if !w.at_line_start {
                        w.line_break();
                    }
                    w.quote = w.quote.saturating_sub(1);
                    w.blank = None;
                    w.block();
                }
                "ul" | "ol" => {
                    lists.pop();
                    if lists.is_empty() {
                        w.block();
                    }
                }
                "a" => {
                    if let Some((Some(href), start)) = links.pop() {
                        let shown = w.out[start.min(w.out.len())..].trim().to_owned();
                        let address = href.strip_prefix("mailto:").unwrap_or(&href);
                        // The address after its text, unless the text already is it.
                        if !shown.is_empty() && shown != address && shown != href {
                            w.out.push_str(&format!(" <{address}>"));
                        } else if shown.is_empty() {
                            w.text(address);
                        }
                    }
                }
                _ => {}
            },
        }
    }
    // No blank lines at the end, nor spaces at the end of lines, except the
    // signature line's: "-- " (with its space) is what mail apps recognise.
    w.out
        .lines()
        .map(|l| if l.trim_end() == "--" { "-- " } else { l.trim_end() })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .trim_start_matches('\n')
        .to_owned()
}

// ---- Sending -----------------------------------------------------------------------

/// The HTML part of a message: the text in a document with a readable font,
/// quotes with a bar, the signature in grey. Inline styles: mail apps ignore
/// style sheets.
pub fn for_sending(clean_html: &str) -> String {
    let body = clean_html
        .replace(
            "<blockquote>",
            "<blockquote style=\"margin: 0 0 0 4px; padding-left: 10px; border-left: 2px solid #ccc; color: #555\">",
        )
        .replace("<div data-signature=\"\">", "<div data-signature=\"\" style=\"color: #777; font-size: 13px\">")
        // Lists: their items close together, a gap after the list.
        .replace("<ul>", "<ul style=\"margin: 0 0 10px; padding-left: 24px\">")
        .replace("<ol>", "<ol style=\"margin: 0 0 10px; padding-left: 24px\">")
        .replace("<li><p>", "<li><p style=\"margin: 0\">")
        .replace("<p>", "<p style=\"margin: 0 0 10px\">");
    format!(
        "<!doctype html><html><body><div style=\"font-family: -apple-system, 'Segoe UI', sans-serif; font-size: 14px; line-height: 1.5\">{body}</div></body></html>"
    )
}

// ---- Signatures --------------------------------------------------------------------

/// The signature block as it sits in a message: its own `div`, a `-- ` line
/// first (the separator plain-text mail apps know).
pub fn signature_block(signature_html: &str) -> String {
    format!("<div data-signature=\"\"><p>-- </p>{signature_html}</div>")
}

/// Where the signature block is in clean HTML: its start and end (after
/// its closing `</div>`).
pub fn find_signature(html: &str) -> Option<(usize, usize)> {
    let start = html.find("<div data-signature")?;
    // The matching `</div>`: count the `div`s opened inside.
    let mut depth = 0;
    let mut at = start;
    while at < html.len() {
        let next_open = html[at..].find("<div").map(|i| at + i);
        let next_close = html[at..].find("</div>").map(|i| at + i);
        match (next_open, next_close) {
            (Some(open), Some(close)) if open < close => {
                depth += 1;
                at = open + 4;
            }
            (_, Some(close)) => {
                depth -= 1;
                at = close + 6;
                if depth == 0 {
                    return Some((start, at));
                }
            }
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_what_the_browser_sends() {
        let dirty = r#"<p onclick="x()" style="color:red">Hi <strong>there</strong><script>alert(1)</script></p>
<a href="javascript:alert(1)">bad</a> <a href="https://havre.app" target="_blank">good</a>
<a href="/relative">rel</a> <img src="data:image/png;base64,AAA"> <table><tr><td>cell</td></tr></table>
<div data-signature="" class="x"><p>-- </p></div>"#;
        let c = clean(dirty);
        assert!(
            !c.contains("onclick") && !c.contains("style") && !c.contains("script") && !c.contains("alert")
        );
        assert!(
            !c.contains("javascript") && !c.contains("img") && !c.contains("table") && !c.contains("class")
        );
        assert!(c.contains("<strong>there</strong>"));
        assert!(c.contains(r#"<a href="https://havre.app" rel="noopener noreferrer">good</a>"#), "{c}");
        assert!(c.contains("cell"), "a removed tag's text stays");
        assert!(c.contains("<div data-signature=\"\">"));
    }

    #[test]
    fn plain_text_from_html() {
        let html = clean(
            "<p>Hello <strong>Anna</strong>,</p><p>Three things:</p>\
             <ul><li><p>the <a href=\"https://x.org/plan\">plan</a></p></li><li><p>a list<br>on two lines</p>\
             <ol><li><p>first</p></li><li><p>second</p></li></ol></li></ul>\
             <p>Write to <a href=\"mailto:bob@x.org\">bob@x.org</a> &amp; me.&nbsp;Thanks</p>\
             <blockquote><p>Earlier</p><blockquote><p>Even earlier</p></blockquote></blockquote>",
        );
        assert_eq!(
            to_text(&html),
            "Hello Anna,\n\nThree things:\n\n- the plan <https://x.org/plan>\n- a list\non two lines\n  1. first\n  \
             2. second\n\nWrite to bob@x.org & me. Thanks\n\n> Earlier\n>\n> > Even earlier"
        );
        // A space alone between two tags; a list item of two paragraphs.
        let html = "<ul><li><p>Pasted <strong>bold</strong> <a href=\"https://havre.app\">link</a></p>\
                    <p>Title</p></li><li><p>next</p></li></ul>";
        assert_eq!(to_text(html), "- Pasted bold link <https://havre.app>\n  Title\n- next");
    }

    #[test]
    fn html_from_text_and_back() {
        let text = "Hi Anna,\nsee below.\n\n- one\n- two\n\nOn Mon, Bob wrote:\n> Lunch?\n> > Before";
        let html = from_text(text);
        assert_eq!(
            html,
            "<p>Hi Anna,<br>see below.</p><ul><li><p>one</p></li><li><p>two</p></li></ul><p>On Mon, Bob wrote:</p>\
             <blockquote><p>Lunch?</p><blockquote><p>Before</p></blockquote></blockquote>"
        );
        assert_eq!(
            to_text(&html),
            "Hi Anna,\nsee below.\n\n- one\n- two\n\nOn Mon, Bob wrote:\n\n> Lunch?\n>\n> > Before"
        );
        assert_eq!(from_text("a < b & c"), "<p>a &lt; b &amp; c</p>");
    }

    #[test]
    fn signature_blocks() {
        let sig = signature_block("<p>Maël <strong>Belliard</strong></p>");
        let html = format!("<p>Hi</p>{sig}<blockquote><div>quoted div</div></blockquote>");
        let (start, end) = find_signature(&html).unwrap();
        assert_eq!(&html[start..end], sig);
        assert_eq!(to_text(&html), "Hi\n\n-- \nMaël Belliard\n\n> quoted div");
        assert!(for_sending(&html).contains("color: #777"));
        // Nested divs inside the signature are counted.
        let nested = "<div data-signature=\"\"><div>a</div></div><p>after</p>";
        assert_eq!(find_signature(nested), Some((0, nested.len() - "<p>after</p>".len())));
    }
}
