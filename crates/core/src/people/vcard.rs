//! vCard (`.vcf`): the address-book format of CardDAV servers, phones,
//! Outlook and Apple Contacts. RFC 2426 (3.0) and RFC 6350 (4.0), with what
//! older files (2.1) and Apple add.
//!
//! A vCard is lines of `NAME;PARAM=VALUE:value` between `BEGIN:VCARD` and
//! `END:VCARD`:
//!
//! ```text
//! BEGIN:VCARD
//! VERSION:3.0
//! FN:Claire Martin
//! N:Martin;Claire;;;
//! TEL;TYPE=CELL:+33 6 12 34 56 78
//! NOTE:Met at the kick-off\, prefers calls
//! END:VCARD
//! ```
//!
//! - Long lines are **folded**: cut, and continued on the next line after a
//!   space. Reading unfolds them first.
//! - In values, `\,` `\;` `\\` and `\n` are **escaped**; structured values
//!   (`N`, `ORG`) are parts separated by an unescaped `;`.
//!
//! [`Card`] keeps every line it read, in order. Writing back only replaces the
//! lines of the details that changed, so whatever Havre doesn't know (postal
//! addresses, websites, Apple's labels…) stays as it was.

use base64::Engine;

use super::{Details, Phone, kinds};

/// One line of a vCard.
#[derive(Debug, Clone, PartialEq)]
pub struct Prop {
    /// Apple's `item1.TEL` groups (kept, never used).
    pub group: Option<String>,
    /// Upper case: `TEL`, `EMAIL`…
    pub name: String,
    /// Names in upper case; values as written (quotes removed).
    pub params: Vec<(String, String)>,
    /// Still escaped: use [`unescape`] or [`parts`].
    pub value: String,
}

impl Prop {
    fn new(name: &str, params: &[(&str, &str)], value: String) -> Self {
        Prop {
            group: None,
            name: name.to_owned(),
            params: params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            value,
        }
    }

    /// Every value of a parameter (`TYPE=CELL,VOICE` and `TYPE=CELL;TYPE=VOICE`
    /// both give `["CELL", "VOICE"]`), upper case.
    pub fn param(&self, name: &str) -> Vec<String> {
        self.params
            .iter()
            .filter(|(k, _)| k == name)
            .flat_map(|(_, v)| v.split(','))
            .map(|v| v.trim().to_uppercase())
            .filter(|v| !v.is_empty())
            .collect()
    }

    pub fn text(&self) -> String {
        unescape(&self.value)
    }
}

/// A contact card.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Card {
    /// Every line between `BEGIN:VCARD` and `END:VCARD`, in order.
    pub props: Vec<Prop>,
}

/// A photo: in the card, or somewhere to fetch.
#[derive(Debug, Clone, PartialEq)]
pub enum PhotoRef {
    Inline { mime: String, data: Vec<u8> },
    Url(String),
}

// ---- Reading -----------------------------------------------------------------------------

/// Every card in a `.vcf` text (a file can hold many). Unreadable lines are
/// skipped rather than failing the whole file.
pub fn parse(text: &str) -> Vec<Card> {
    let mut cards = Vec::new();
    let mut current: Option<Card> = None;
    for line in unfold(text) {
        let Some(prop) = parse_line(&line) else { continue };
        match (prop.name.as_str(), prop.value.trim().to_uppercase().as_str()) {
            ("BEGIN", "VCARD") => current = Some(Card::default()),
            ("END", "VCARD") => cards.extend(current.take()),
            _ => {
                if let Some(card) = current.as_mut() {
                    card.props.push(prop);
                }
            }
        }
    }
    cards
}

/// Joins folded lines: a line starting with a space or a tab continues the
/// previous one. vCard 2.1's quoted-printable values end a line with `=`
/// to continue instead.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut soft_break = false;
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = raw.strip_prefix([' ', '\t'])
            && let Some(last) = lines.last_mut()
        {
            last.push_str(rest);
        } else if soft_break && let Some(last) = lines.last_mut() {
            last.push_str(raw);
        } else {
            lines.push(raw.to_owned());
        }
        let last = lines.last().map(String::as_str).unwrap_or_default();
        soft_break = last.to_uppercase().contains("QUOTED-PRINTABLE") && last.ends_with('=');
        if soft_break && let Some(last) = lines.last_mut() {
            last.pop(); // the `=`
        }
    }
    lines.retain(|l| !l.trim().is_empty());
    lines
}

/// `item1.TEL;TYPE="CELL,VOICE":+33…` → a [`Prop`]. `None` for a line
/// without a `:`.
fn parse_line(line: &str) -> Option<Prop> {
    // The `:` that ends the name and parameters: the first one outside quotes.
    let mut in_quotes = false;
    let colon = line.char_indices().find_map(|(i, c)| {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => return Some(i),
            _ => {}
        }
        None
    })?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);

    let mut pieces = split_unquoted(head, ';').into_iter();
    let name = pieces.next()?.trim().to_owned();
    let (group, name) = match name.split_once('.') {
        Some((group, name)) => (Some(group.to_owned()), name.to_uppercase()),
        None => (None, name.to_uppercase()),
    };
    if name.is_empty() {
        return None;
    }
    let params = pieces
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (k.trim().to_uppercase(), v.trim().trim_matches('"').to_owned()),
            // vCard 2.1: `TEL;CELL:` means `TYPE=CELL`.
            None => ("TYPE".to_owned(), p.trim().to_owned()),
        })
        .collect::<Vec<_>>();
    let mut prop = Prop { group, name, params, value: value.to_owned() };
    if prop.param("ENCODING").iter().any(|e| e == "QUOTED-PRINTABLE") {
        // Decoded as is (its `;` still separate parts); only line breaks get escaped.
        prop.value = quoted_printable(&prop.value).replace("\r\n", "\\n").replace('\n', "\\n");
        prop.params.retain(|(k, _)| k != "ENCODING" && k != "CHARSET");
    }
    Some(prop)
}

fn split_unquoted(text: &str, at: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut in_quotes) = (0, false);
    for (i, c) in text.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c == at && !in_quotes {
            out.push(&text[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&text[start..]);
    out
}

/// `=C3=A9` → `é` (vCard 2.1).
fn quoted_printable(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            let hex = text.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(byte) = hex {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `a\, b\nc` → `a, b` + new line + `c`.
pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The reverse of [`unescape`].
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// The parts of a structured value (`N:Martin;Claire;;;`), unescaped.
pub fn parts(value: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    let last = out.last_mut().expect("never empty");
                    match next {
                        'n' | 'N' => last.push('\n'),
                        other => last.push(other),
                    }
                }
            }
            ';' => out.push(String::new()),
            c => out.last_mut().expect("never empty").push(c),
        }
    }
    out
}

impl Card {
    fn first(&self, name: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.name == name)
    }

    fn first_text(&self, name: &str) -> String {
        self.first(name).map(|p| p.text().trim().to_owned()).unwrap_or_default()
    }

    pub fn version(&self) -> &str {
        self.first("VERSION").map(|p| p.value.trim()).unwrap_or("3.0")
    }

    pub fn uid(&self) -> Option<String> {
        Some(self.first_text("UID")).filter(|u| !u.is_empty())
    }

    /// A group card (Apple's `X-ADDRESSBOOKSERVER-KIND:group`, vCard 4's
    /// `KIND:group`) isn't a person.
    pub fn is_group(&self) -> bool {
        ["KIND", "X-ADDRESSBOOKSERVER-KIND"]
            .iter()
            .any(|k| self.first(k).is_some_and(|p| p.value.trim().eq_ignore_ascii_case("group")))
    }

    /// The addresses, lowercase, each once.
    pub fn emails(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in self.props.iter().filter(|p| p.name == "EMAIL") {
            let email = p.text().trim().trim_start_matches("mailto:").to_lowercase();
            if email.contains('@') && !out.contains(&email) {
                out.push(email);
            }
        }
        out
    }

    /// The details Havre shows and edits.
    pub fn details(&self) -> Details {
        let mut name = self.first_text("FN");
        if name.is_empty() {
            // No `FN` (vCard 2.1): "Given Family" from `N` (Family;Given;Middle;Prefix;Suffix).
            let n = self.first("N").map(|p| parts(&p.value)).unwrap_or_default();
            let pick = |i: usize| n.get(i).map(|s| s.trim().to_owned()).unwrap_or_default();
            name = [pick(3), pick(1), pick(2), pick(0), pick(4)]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
        }
        let company = self
            .first("ORG")
            .and_then(|p| parts(&p.value).into_iter().next())
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();
        let phones = self
            .props
            .iter()
            .filter(|p| p.name == "TEL")
            .filter_map(|p| {
                let number = p.text().trim().trim_start_matches("tel:").trim().to_owned();
                (!number.is_empty()).then(|| Phone { label: phone_label(&p.param("TYPE")), number })
            })
            .collect();
        let birthday = self.first("BDAY").and_then(read_birthday);
        Details {
            name,
            company,
            role: self.first_text("TITLE"),
            phones,
            birthday,
            note: self.first_text("NOTE"),
        }
    }

    /// The photo, if there's one.
    pub fn photo(&self) -> Option<PhotoRef> {
        let p = self.first("PHOTO")?;
        let value = p.value.trim();
        // vCard 4: `data:image/jpeg;base64,…`, or a link.
        if let Some(rest) = value.strip_prefix("data:") {
            let (meta, data) = rest.split_once(',')?;
            let mime = meta.split(';').next().unwrap_or("image/jpeg").to_owned();
            let data = base64::engine::general_purpose::STANDARD.decode(strip_spaces(data)).ok()?;
            return Some(PhotoRef::Inline { mime, data });
        }
        if value.starts_with("http://") || value.starts_with("https://") {
            return Some(PhotoRef::Url(value.to_owned()));
        }
        // vCard 3: `PHOTO;ENCODING=b;TYPE=JPEG:…` (2.1: `ENCODING=BASE64`).
        let encoding = p.param("ENCODING");
        if encoding.iter().any(|e| e == "B" || e == "BASE64") {
            let kind = p.param("TYPE").into_iter().next().unwrap_or_else(|| "JPEG".into()).to_lowercase();
            let data = base64::engine::general_purpose::STANDARD.decode(strip_spaces(value)).ok()?;
            let mime = if kind.contains('/') { kind } else { format!("image/{kind}") };
            return Some(PhotoRef::Inline { mime, data });
        }
        None
    }
}

fn strip_spaces(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `TYPE=CELL,VOICE` → `mobile`; Havre's labels are mobile, work, home,
/// other (a type it doesn't know is kept, in lower case).
fn phone_label(types: &[String]) -> String {
    let meaningful: Vec<&str> =
        types.iter().map(String::as_str).filter(|t| !matches!(*t, "VOICE" | "PREF" | "INTERNET")).collect();
    if meaningful.contains(&"CELL") || meaningful.contains(&"IPHONE") {
        return "mobile".into();
    }
    match meaningful.first() {
        Some(&"WORK") => "work".into(),
        Some(&"HOME") => "home".into(),
        Some(other) => other.trim_start_matches("X-").to_lowercase(),
        None => String::new(),
    }
}

/// `mobile` → `CELL` (the reverse of [`phone_label`]).
fn phone_type(label: &str) -> String {
    match label.trim().to_lowercase().as_str() {
        "mobile" | "cell" => "CELL".into(),
        "" => "VOICE".into(),
        other => {
            // Only letters and digits in a parameter value without quotes.
            let clean: String = other.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            if clean.is_empty() { "VOICE".into() } else { clean.to_uppercase() }
        }
    }
}

/// `BDAY` → `YYYY-MM-DD`, or `--MM-DD` without the year. Accepts `19881112`,
/// `1988-11-12`, `--1112`, `--11-12`, a time after the date, and Apple's
/// "no year" (`X-APPLE-OMIT-YEAR=1604` with `1604-11-12`).
fn read_birthday(p: &Prop) -> Option<String> {
    let value = p.text();
    let date = value.trim().split('T').next().unwrap_or_default();
    let (year, rest) = match date.strip_prefix("--") {
        Some(rest) => (None, rest.to_owned()),
        None => {
            let digits: String = date.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() != 8 {
                return None;
            }
            (Some(digits[..4].to_owned()), digits[4..].to_owned())
        }
    };
    let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() != 4 {
        return None;
    }
    let omitted = p.param("X-APPLE-OMIT-YEAR").into_iter().next();
    let year = year.filter(|y| Some(y) != omitted.as_ref() && y != "1604");
    let text = match year {
        Some(y) => format!("{y}-{}-{}", &digits[..2], &digits[2..]),
        None => format!("--{}-{}", &digits[..2], &digits[2..]),
    };
    kinds::parse_birthday(&text).map(|_| text)
}

// ---- Writing -----------------------------------------------------------------------------

impl Card {
    /// A new card (vCard 3.0: what every address book reads).
    pub fn new(uid: &str, details: &Details, emails: &[String]) -> Card {
        let mut card = Card {
            props: vec![
                Prop::new("VERSION", &[], "3.0".into()),
                Prop::new("PRODID", &[], "-//Havre//People//EN".into()),
                Prop::new("UID", &[], escape(uid)),
            ],
        };
        for email in emails {
            card.props.push(Prop::new("EMAIL", &[("TYPE", "INTERNET")], escape(email)));
        }
        card.apply(details);
        if card.first("FN").is_none() {
            // `FN` is required: an address, else nothing.
            let fallback = emails.first().cloned().unwrap_or_default();
            card.props.push(Prop::new("FN", &[], escape(&fallback)));
            card.props.push(Prop::new("N", &[], ";;;;".into()));
        }
        card
    }

    /// Sets the details that differ from what the card says; every other
    /// line is kept as it was (and where it was).
    pub fn apply(&mut self, details: &Details) {
        let current = self.details();
        if current.name != details.name {
            let name = details.name.trim();
            let (given, family) = split_name(name);
            self.replace("FN", vec![Prop::new("FN", &[], escape(name))]);
            let n = format!("{};{};;;", escape(family), escape(given));
            self.replace("N", vec![Prop::new("N", &[], n)]);
        }
        if current.company != details.company {
            // Keep the departments after the company (`ORG:Acme;Sales`).
            let mut org = self.first("ORG").map(|p| parts(&p.value)).unwrap_or_default();
            if org.is_empty() {
                org.push(String::new());
            }
            org[0] = details.company.trim().to_owned();
            let value: Vec<String> = org.iter().map(|s| escape(s)).collect();
            let props = if org.iter().all(|s| s.is_empty()) {
                Vec::new()
            } else {
                vec![Prop::new("ORG", &[], value.join(";"))]
            };
            self.replace("ORG", props);
        }
        if current.role != details.role {
            self.replace_text("TITLE", &details.role);
        }
        if current.note != details.note {
            self.replace_text("NOTE", &details.note);
        }
        if current.phones != details.phones {
            let props = details
                .phones
                .iter()
                .map(|p| Prop::new("TEL", &[("TYPE", &phone_type(&p.label))], escape(p.number.trim())))
                .collect();
            self.replace("TEL", props);
        }
        if current.birthday != details.birthday {
            let props = match details.birthday.as_deref().and_then(kinds::parse_birthday) {
                None => Vec::new(),
                Some((m, d, Some(y))) => vec![Prop::new("BDAY", &[], format!("{y:04}-{m:02}-{d:02}"))],
                // No year: vCard 4 has `--MMDD`; for 3.0, Apple's way (read by Nextcloud and DAVx⁵ too).
                Some((m, d, None)) if self.version().starts_with('4') => {
                    vec![Prop::new("BDAY", &[], format!("--{m:02}{d:02}"))]
                }
                Some((m, d, None)) => {
                    vec![Prop::new("BDAY", &[("X-APPLE-OMIT-YEAR", "1604")], format!("1604-{m:02}-{d:02}"))]
                }
            };
            self.replace("BDAY", props);
        }
    }

    fn replace_text(&mut self, name: &str, text: &str) {
        let text = text.trim();
        let props = if text.is_empty() { Vec::new() } else { vec![Prop::new(name, &[], escape(text))] };
        self.replace(name, props);
    }

    /// Replaces every `name` line with `props`, where the first one was (at
    /// the end when there was none). Apple's labels for removed lines go too.
    fn replace(&mut self, name: &str, props: Vec<Prop>) {
        let at = self.props.iter().position(|p| p.name == name);
        let groups: Vec<String> =
            self.props.iter().filter(|p| p.name == name).filter_map(|p| p.group.clone()).collect();
        self.props.retain(|p| p.name != name && !p.group.as_ref().is_some_and(|g| groups.contains(g)));
        let at = at.unwrap_or(self.props.len()).min(self.props.len());
        self.props.splice(at..at, props);
    }

    /// The card as text: CRLF line ends, lines folded at 75 bytes.
    pub fn to_vcf(&self) -> String {
        let mut out = String::from("BEGIN:VCARD\r\n");
        // VERSION must come first.
        let (version, rest): (Vec<&Prop>, Vec<&Prop>) = self.props.iter().partition(|p| p.name == "VERSION");
        for p in version.into_iter().take(1).chain(rest) {
            let mut line = String::new();
            if let Some(group) = &p.group {
                line.push_str(group);
                line.push('.');
            }
            line.push_str(&p.name);
            for (k, v) in &p.params {
                let quote = v.contains([':', ';', ',']);
                line.push_str(&if quote { format!(";{k}=\"{v}\"") } else { format!(";{k}={v}") });
            }
            line.push(':');
            line.push_str(&p.value);
            fold(&line, &mut out);
        }
        out.push_str("END:VCARD\r\n");
        out
    }
}

/// "Claire Martin" → ("Claire", "Martin"): the last word is the family
/// name. Good enough for most names; the full name (`FN`) is what's shown.
fn split_name(name: &str) -> (&str, &str) {
    match name.rsplit_once(' ') {
        Some((given, family)) => (given.trim(), family.trim()),
        None => (name, ""),
    }
}

/// Writes `line` cut into pieces of at most 75 bytes, never inside a
/// character (UTF-8 letters take several bytes).
fn fold(line: &str, out: &mut String) {
    let mut start = 0;
    let mut first = true;
    while start < line.len() {
        // The continuation's leading space counts in the 75.
        let room = if first { 75 } else { 74 };
        let mut end = (start + room).min(line.len());
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        if !first {
            out.push(' ');
        }
        out.push_str(&line[start..end]);
        out.push_str("\r\n");
        start = end;
        first = false;
    }
}

/// Several cards in one file.
pub fn to_file(cards: &[Card]) -> String {
    cards.iter().map(Card::to_vcf).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPLE: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nPRODID:-//Apple Inc.//iPhone OS 17.0//EN\r\n\
N:Martin;Claire;;;\r\nFN:Claire Martin\r\nORG:Acme;Sales;\r\nTITLE:Project lead\r\n\
EMAIL;type=INTERNET;type=WORK;type=pref:Claire.Martin@acme.fr\r\n\
EMAIL;type=INTERNET;type=HOME:claire@gmail.com\r\n\
item1.TEL;type=pref:+33 1 23 45 67 89\r\nitem1.X-ABLabel:_$!<Main>!$_\r\n\
TEL;type=CELL;type=VOICE:+33 6 12 34 56 78\r\n\
ADR;type=WORK:;;3 rue de la Paix;Lyon;;69001;France\r\n\
NOTE:Met at the Lyon kick-off\\, prefers calls.\\nLikes trains.\r\n\
BDAY;X-APPLE-OMIT-YEAR=1604:1604-11-12\r\n\
PHOTO;ENCODING=b;TYPE=JPEG:/9j/4A\r\n AQSkZJRg==\r\n\
UID:abc-123\r\nEND:VCARD\r\n";

    #[test]
    fn reads_an_apple_card() {
        let cards = parse(APPLE);
        assert_eq!(cards.len(), 1);
        let card = &cards[0];
        assert_eq!(card.uid().as_deref(), Some("abc-123"));
        assert_eq!(card.emails(), ["claire.martin@acme.fr", "claire@gmail.com"]);
        let d = card.details();
        assert_eq!(
            (d.name.as_str(), d.company.as_str(), d.role.as_str()),
            ("Claire Martin", "Acme", "Project lead")
        );
        assert_eq!(d.note, "Met at the Lyon kick-off, prefers calls.\nLikes trains.");
        assert_eq!(d.birthday.as_deref(), Some("--11-12"), "Apple's 1604 means no year");
        assert_eq!(
            d.phones,
            [
                Phone { label: String::new(), number: "+33 1 23 45 67 89".into() },
                Phone { label: "mobile".into(), number: "+33 6 12 34 56 78".into() },
            ]
        );
        // The folded photo is joined back.
        let Some(PhotoRef::Inline { mime, data }) = card.photo() else { panic!("a photo") };
        assert_eq!(mime, "image/jpeg");
        assert_eq!(&data[..2], [0xff, 0xd8]);
    }

    #[test]
    fn writes_back_only_what_changed() {
        let mut card = parse(APPLE).remove(0);
        let mut d = card.details();
        d.role = "Director".into();
        d.phones.push(Phone { label: "work".into(), number: "+33 4 00 00 00 00".into() });
        d.birthday = Some("1988-11-12".into());
        card.apply(&d);
        let text = card.to_vcf();
        // Unchanged lines stay as they were; the address and the photo too.
        assert!(text.contains("N:Martin;Claire;;;\r\n") && text.contains("ORG:Acme;Sales;\r\n"));
        assert!(text.contains("ADR;TYPE=WORK:;;3 rue de la Paix;Lyon;;69001;France\r\n"));
        assert!(text.contains("PHOTO;ENCODING=b;TYPE=JPEG:/9j/4AAQSkZJRg==\r\n"));
        assert!(text.contains("TITLE:Director\r\n") && text.contains("BDAY:1988-11-12\r\n"));
        // The phones are rewritten; Apple's label for the old one goes with it.
        assert!(!text.contains("X-ABLabel") && text.contains("TEL;TYPE=WORK:+33 4 00 00 00 00"));
        // And it reads back the same.
        let back = parse(&text).remove(0).details();
        assert_eq!(back, d);

        // A company change keeps the department; clearing the note removes the line.
        d.company = "Acme; Inc".into();
        d.note.clear();
        d.birthday = Some("--02-29".into());
        card.apply(&d);
        let text = card.to_vcf();
        assert!(text.contains("ORG:Acme\\; Inc;Sales;\r\n") && !text.contains("NOTE"));
        assert!(text.contains("BDAY;X-APPLE-OMIT-YEAR=1604:1604-02-29"));
        assert_eq!(parse(&text).remove(0).details(), d);
    }

    #[test]
    fn reads_other_flavours() {
        // vCard 4: data URI photo, `--MMDD`, TEL as a URI, no ORG.
        let v4 = "BEGIN:VCARD\nVERSION:4.0\nFN:Paul Bernard\nTEL;VALUE=uri;TYPE=home:tel:+33-1-11\n\
BDAY:--0229\nPHOTO:data:image/png;base64,iVBORw0K\nEND:VCARD\n";
        let card = parse(v4).remove(0);
        let d = card.details();
        assert_eq!(d.birthday.as_deref(), Some("--02-29"));
        assert_eq!(d.phones[0], Phone { label: "home".into(), number: "+33-1-11".into() });
        assert!(matches!(card.photo(), Some(PhotoRef::Inline { mime, .. }) if mime == "image/png"));

        // vCard 2.1 (old Outlook exports): no FN, quoted-printable, bare types.
        let v21 = "BEGIN:VCARD\r\nVERSION:2.1\r\nN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Dur=C3=A9;=\r\nSophie\r\n\
TEL;CELL:06 12\r\nEMAIL;PREF;INTERNET:Sophie@Studio.fr\r\nBDAY:19900102\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Team\r\nX-ADDRESSBOOKSERVER-KIND:group\r\nEND:VCARD\r\n";
        let cards = parse(v21);
        assert_eq!(cards.len(), 2);
        let d = cards[0].details();
        assert_eq!(d.name, "Sophie Duré");
        assert_eq!(d.birthday.as_deref(), Some("1990-01-02"));
        assert_eq!(d.phones[0].label, "mobile");
        assert_eq!(cards[0].emails(), ["sophie@studio.fr"]);
        assert!(cards[1].is_group() && !cards[0].is_group());
    }

    #[test]
    fn new_cards_fold_and_escape() {
        let d = Details {
            name: "Élodie Rousseau-Lefèvre".into(),
            note:
                "Très longue note, avec des virgules; et des points-virgules, pour voir comment c'est plié."
                    .into(),
            birthday: Some("--05-01".into()),
            ..Details::default()
        };
        let card = Card::new("u1@havre", &d, &["elodie@example.org".into()]);
        let text = card.to_vcf();
        assert!(text.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n"));
        assert!(text.lines().all(|l| l.len() <= 75), "{text}");
        assert!(text.contains("N:Rousseau-Lefèvre;Élodie;;;"));
        let back = parse(&text).remove(0);
        assert_eq!(back.details(), d);
        assert_eq!(back.emails(), ["elodie@example.org"]);
        assert_eq!(back.uid().as_deref(), Some("u1@havre"));
        // No name: the address stands in for the required FN.
        let bare = Card::new("u2", &Details::default(), &["x@y.org".into()]).to_vcf();
        assert!(bare.contains("FN:x@y.org"));
    }
}
