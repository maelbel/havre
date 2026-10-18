//! Small pure helpers that build IMAP command arguments.
//! Kept apart from the network code so they are easy to test.

use serde::Serialize;

use super::MessageSummary;

/// What to list in a folder.
#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    /// Page size.
    pub limit: u32,
    /// Only messages older than this UID ("load more").
    pub before_uid: Option<u32>,
    /// Full-text search (sender, subject, body), done by the mail server.
    pub search: Option<String>,
}

/// One page of a message list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MessagePage {
    /// Newest first.
    pub messages: Vec<MessageSummary>,
    /// More (older) messages exist: ask again with `before_uid`.
    pub has_more: bool,
}

/// Builds the arguments of `UID SEARCH`, or `None` when no search is needed
/// (then we can list by sequence numbers, which is faster).
///
/// `before_uid = Some(42)` and `text = "devis"` give `UID 1:41 TEXT "devis"`.
pub fn search_criteria(before_uid: Option<u32>, text: Option<&str>) -> Option<String> {
    let mut parts = Vec::new();
    let text = text.map(str::trim).filter(|t| !t.is_empty());

    // Non-ASCII search terms need the charset announced first.
    if text.is_some_and(|t| !t.is_ascii()) {
        parts.push("CHARSET UTF-8".to_owned());
    }
    match before_uid {
        Some(0 | 1) => parts.push("UID 0".to_owned()), // nothing is older than the first message
        Some(uid) => parts.push(format!("UID 1:{}", uid - 1)),
        None => {}
    }
    if let Some(t) = text {
        parts.push(format!("TEXT {}", quote(t)));
    }

    // "CHARSET UTF-8" alone is not a search.
    let has_key = before_uid.is_some() || text.is_some();
    has_key.then(|| parts.join(" "))
}

/// IMAP quoted string: wrap in `"`, escape `\` and `"`, drop line breaks
/// (they are not allowed inside a quoted string).
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\r' | '\n' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `[3, 7, 9]` → `"3,7,9"`, the "sequence set" syntax of IMAP.
pub fn uid_set(uids: &[u32]) -> String {
    uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_listing_needs_no_search() {
        assert_eq!(search_criteria(None, None), None);
        assert_eq!(search_criteria(None, Some("   ")), None);
    }

    #[test]
    fn builds_paging_and_text_criteria() {
        assert_eq!(search_criteria(Some(42), None).as_deref(), Some("UID 1:41"));
        assert_eq!(search_criteria(Some(1), None).as_deref(), Some("UID 0"));
        assert_eq!(
            search_criteria(Some(42), Some("devis Q4")).as_deref(),
            Some(r#"UID 1:41 TEXT "devis Q4""#)
        );
        assert_eq!(
            search_criteria(None, Some("réunion")).as_deref(),
            Some(r#"CHARSET UTF-8 TEXT "réunion""#)
        );
    }

    #[test]
    fn quotes_safely() {
        assert_eq!(quote(r#"say "hi"\now"#), r#""say \"hi\"\\now""#);
        assert_eq!(quote("a\r\nb"), r#""a  b""#);
    }

    #[test]
    fn joins_uids() {
        assert_eq!(uid_set(&[3, 7, 9]), "3,7,9");
    }
}
