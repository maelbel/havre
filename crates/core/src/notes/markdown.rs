//! A small Markdown reader for notes.
//!
//! It understands the Markdown people actually type in notes: headings
//! (`#`, `##`, `###`), paragraphs, bullets (`-`, `*`, `+`), numbered lines
//! (`1.`), to-dos (`- [ ]` / `- [x]`), quotes (`>`), rules (`---`) and code
//! blocks (```` ``` ````), with **bold**, *italic*, `code` and links inside
//! a line.
//!
//! Why not a crate? Notes need to know *where* each to-do is (to tick it,
//! or turn it into a task), and the browser should get plain data rather
//! than HTML: it draws each block itself, so a note can never inject
//! markup into the page. A hand-written reader for this subset is short
//! (and a good exercise with `&str`, `strip_prefix` and `Option`).
//!
//! The reader never fails: anything it doesn't understand stays text.

use serde::Serialize;

/// One piece of a line: plain, bold, italic, code or a link.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Span {
    Text {
        text: String,
    },
    Bold {
        text: String,
    },
    Italic {
        text: String,
    },
    Code {
        text: String,
    },
    /// Only `http(s):` and `mailto:` addresses become links.
    Link {
        text: String,
        href: String,
    },
}

/// One block of a note. `line` is where it starts (0 = the first line of
/// the body), so the browser can point back into the text.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Block {
    Heading {
        line: usize,
        /// 1, 2 or 3 (`####` and more count as 3).
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph {
        line: usize,
        spans: Vec<Span>,
    },
    Bullet {
        line: usize,
        /// 0 at the margin, 1 or 2 when indented.
        depth: u8,
        spans: Vec<Span>,
    },
    Numbered {
        line: usize,
        number: u32,
        spans: Vec<Span>,
    },
    Todo {
        line: usize,
        done: bool,
        /// The text as typed (what a task made from it is called).
        text: String,
        spans: Vec<Span>,
        /// The task made from this line, if any (filled in by the store).
        task: Option<i64>,
    },
    Quote {
        line: usize,
        spans: Vec<Span>,
    },
    Code {
        line: usize,
        text: String,
    },
    Rule {
        line: usize,
    },
}

/// Reads a whole note.
pub fn parse(markdown: &str) -> Vec<Block> {
    // `lines()` drops the `\n` (and a `\r` before it), so line numbers here
    // match `split('\n')` in `set_todo`.
    let lines: Vec<&str> = markdown.lines().collect();
    let mut blocks = Vec::new();
    // Lines of the paragraph being read: consecutive lines of text join
    // into one paragraph, like in every Markdown.
    let mut paragraph: Vec<&str> = Vec::new();
    let mut paragraph_line = 0;

    // A closure can't borrow `blocks` mutably while we also push to it
    // elsewhere, so the "end the paragraph" step is a small function.
    fn end_paragraph(blocks: &mut Vec<Block>, paragraph: &mut Vec<&str>, line: usize) {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph { line, spans: inline(&paragraph.join(" ")) });
            paragraph.clear();
        }
    }

    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let line = raw.trim();

        // A code block: everything up to the closing ``` stays as typed.
        if line.starts_with("```") {
            end_paragraph(&mut blocks, &mut paragraph, paragraph_line);
            let start = i;
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            blocks.push(Block::Code { line: start, text: code.join("\n") });
            i += 1; // the closing ``` (or the end)
            continue;
        }

        if line.is_empty() {
            end_paragraph(&mut blocks, &mut paragraph, paragraph_line);
        } else if let Some((level, text)) = heading(line) {
            end_paragraph(&mut blocks, &mut paragraph, paragraph_line);
            blocks.push(Block::Heading { line: i, level, spans: inline(text) });
        } else if is_rule(line) {
            end_paragraph(&mut blocks, &mut paragraph, paragraph_line);
            blocks.push(Block::Rule { line: i });
        } else if let Some((done, text)) = todo(line) {
            end_paragraph(&mut blocks, &mut paragraph, paragraph_line);
            blocks.push(Block::Todo {
                line: i,
                done,
                text: text.to_owned(),
                spans: inline(text),
                task: None,
            });
        } else if let Some(text) = bullet(line) {
            end_paragraph(&mut blocks, &mut paragraph, paragraph_line);
            blocks.push(Block::Bullet { line: i, depth: depth(raw), spans: inline(text) });
        } else if let Some((number, text)) = numbered(line) {
            end_paragraph(&mut blocks, &mut paragraph, paragraph_line);
            blocks.push(Block::Numbered { line: i, number, spans: inline(text) });
        } else if let Some(text) = line.strip_prefix('>') {
            end_paragraph(&mut blocks, &mut paragraph, paragraph_line);
            blocks.push(Block::Quote { line: i, spans: inline(text.trim()) });
        } else {
            if paragraph.is_empty() {
                paragraph_line = i;
            }
            paragraph.push(line);
        }
        i += 1;
    }
    end_paragraph(&mut blocks, &mut paragraph, paragraph_line);
    blocks
}

/// `## Agenda` → `(2, "Agenda")`. A `#` must be followed by a space
/// (`#hashtag` is text).
fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 {
        return None;
    }
    let text = line[hashes..].strip_prefix(' ')?;
    Some((hashes.min(3) as u8, text.trim()))
}

/// `---`, `***` or `___` (three or more of the same, nothing else).
fn is_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3 && ["-", "*", "_"].iter().any(|m| compact.chars().all(|c| c.to_string() == *m))
}

/// `- text`, `* text`, `+ text` → `text`. A lone `-` (a bullet not
/// written yet, as in the meeting template) is an empty bullet.
fn bullet(line: &str) -> Option<&str> {
    if matches!(line, "-" | "*" | "+") {
        return Some("");
    }
    ["- ", "* ", "+ "].iter().find_map(|marker| line.strip_prefix(marker)).map(str::trim)
}

/// `- [ ] text` / `- [x] text` → `(done, text)`. A to-do with nothing
/// written yet (`- [ ]`, as in the meeting template) is still a to-do.
fn todo(line: &str) -> Option<(bool, &str)> {
    let rest = ["-", "*", "+"].iter().find_map(|m| line.strip_prefix(m))?.trim_start();
    // The box is the first three characters: `[ ]`, `[x]` or `[X]`.
    let (done, text) = match rest.get(..3)? {
        "[ ]" => (false, &rest[3..]),
        "[x]" | "[X]" => (true, &rest[3..]),
        _ => return None,
    };
    // `- [x]done` isn't a to-do: a space (or nothing) must follow.
    if !text.is_empty() && !text.starts_with(' ') {
        return None;
    }
    Some((done, text.trim()))
}

/// `3. text` or `3) text` → `(3, "text")`.
fn numbered(line: &str) -> Option<(u32, &str)> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 4 {
        return None;
    }
    let number = line[..digits].parse().ok()?;
    let rest = &line[digits..];
    let text = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))?;
    Some((number, text.trim()))
}

/// How far a bullet is indented: two spaces (or a tab) per level, at most 2.
fn depth(raw: &str) -> u8 {
    let spaces: usize =
        raw.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 2 } else { 1 }).sum();
    (spaces / 2).min(2) as u8
}

/// Reads the inside of a line: `**bold**`, `*italic*` / `_italic_`,
/// `` `code` ``, `[text](https://…)` and bare `https://…` addresses.
pub fn inline(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut previous: Option<char> = None;
    // `i` is a byte position. We only ever move it by whole characters
    // (`len_utf8`) or by lengths `find` gave us, so `&text[i..]` never
    // cuts a character in two (which would panic).
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if let Some((span, used)) = special(rest, previous) {
            if !plain.is_empty() {
                spans.push(Span::Text { text: std::mem::take(&mut plain) });
            }
            spans.push(span);
            i += used;
            previous = text[..i].chars().next_back();
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        plain.push(c);
        previous = Some(c);
        i += c.len_utf8();
    }
    if !plain.is_empty() {
        spans.push(Span::Text { text: plain });
    }
    spans
}

/// If `rest` starts with something special, that span and how many bytes
/// it used. `previous` is the character before (for `_` and addresses,
/// which only count at the start of a word).
fn special(rest: &str, previous: Option<char>) -> Option<(Span, usize)> {
    let in_word = previous.is_some_and(char::is_alphanumeric);

    if let Some(after) = rest.strip_prefix('`') {
        let end = after.find('`').filter(|&e| e > 0)?;
        return Some((Span::Code { text: after[..end].to_owned() }, end + 2));
    }
    if let Some(after) = rest.strip_prefix("**") {
        let end = after.find("**").filter(|&e| e > 0)?;
        return Some((Span::Bold { text: after[..end].to_owned() }, end + 4));
    }
    for marker in ['*', '_'] {
        if let Some(after) = rest.strip_prefix(marker) {
            // `snake_case_name` and `2 * 3 * 4` stay text.
            if (marker == '_' && in_word) || after.starts_with(' ') {
                return None;
            }
            let end = after.find(marker).filter(|&e| e > 0)?;
            return Some((Span::Italic { text: after[..end].to_owned() }, end + 2));
        }
    }
    if rest.starts_with('[') {
        let close = rest.find("](")?;
        let label = &rest[1..close];
        let after = &rest[close + 2..];
        let end = after.find(')')?;
        let href = after[..end].trim();
        if label.is_empty() || !safe_href(href) {
            return None;
        }
        return Some((Span::Link { text: label.to_owned(), href: href.to_owned() }, close + 2 + end + 1));
    }
    if !in_word && (rest.starts_with("https://") || rest.starts_with("http://")) {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        // "See https://example.com." — the final dot isn't part of it.
        let url = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
        if url.len() > "https://".len() {
            return Some((Span::Link { text: url.to_owned(), href: url.to_owned() }, url.len()));
        }
    }
    None
}

/// Links can't run code: `javascript:` and friends stay text.
fn safe_href(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:"))
        && !href.contains(char::is_whitespace)
}

/// Ticks (or unticks) the to-do on `line`. `None` if that line isn't a
/// to-do (the note changed meanwhile). Every other byte stays as it was.
pub fn set_todo(markdown: &str, line: usize, done: bool) -> Option<String> {
    let mut lines: Vec<String> = markdown.split('\n').map(str::to_owned).collect();
    let target = lines.get_mut(line)?;
    todo(target.trim())?;
    // The first `[ ]` / `[x]` on the line is the box (the marker comes
    // before it, the text after).
    let open = target.find('[')?;
    let mark = if done { "x" } else { " " };
    target.replace_range(open + 1..open + 2, mark);
    Some(lines.join("\n"))
}

/// The plain text of a note's start, for the list: no headings, no marks.
pub fn snippet(markdown: &str, max_chars: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in parse(markdown) {
        let text = match &block {
            Block::Paragraph { spans, .. }
            | Block::Bullet { spans, .. }
            | Block::Numbered { spans, .. }
            | Block::Quote { spans, .. }
            | Block::Todo { spans, .. } => plain(spans),
            _ => continue,
        };
        if !text.is_empty() {
            parts.push(text);
        }
        if parts.iter().map(|p| p.len()).sum::<usize>() > max_chars * 2 {
            break;
        }
    }
    let joined = parts.join(" · ");
    if joined.chars().count() <= max_chars {
        return joined;
    }
    let cut: String = joined.chars().take(max_chars).collect();
    format!("{}…", cut.trim_end())
}

/// The words of a line, without the marks.
pub fn plain(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|s| match s {
            Span::Text { text }
            | Span::Bold { text }
            | Span::Italic { text }
            | Span::Code { text }
            | Span::Link { text, .. } => text.as_str(),
        })
        .collect()
}

/// To-dos still open in a note.
pub fn open_todos(markdown: &str) -> usize {
    parse(markdown)
        .iter()
        .filter(|b| matches!(b, Block::Todo { done: false, text, .. } if !text.is_empty()))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Span {
        Span::Text { text: t.into() }
    }

    #[test]
    fn blocks() {
        let note = "Claire wants a single\nplanning.\n\n## Agenda\n- Budget\n  - 2026 envelope\n1. First\n\
                    - [ ] Send the planning\n- [x] Book the train\n- [ ]\n> Quote\n---\n```\nlet x = 1;\n```\n#tag";
        let blocks = parse(note);
        assert_eq!(
            blocks[0],
            Block::Paragraph { line: 0, spans: vec![text("Claire wants a single planning.")] }
        );
        assert_eq!(blocks[1], Block::Heading { line: 3, level: 2, spans: vec![text("Agenda")] });
        assert_eq!(blocks[2], Block::Bullet { line: 4, depth: 0, spans: vec![text("Budget")] });
        assert_eq!(blocks[3], Block::Bullet { line: 5, depth: 1, spans: vec![text("2026 envelope")] });
        assert_eq!(blocks[4], Block::Numbered { line: 6, number: 1, spans: vec![text("First")] });
        assert!(
            matches!(&blocks[5], Block::Todo { line: 7, done: false, text, .. } if text == "Send the planning")
        );
        assert!(matches!(&blocks[6], Block::Todo { line: 8, done: true, .. }));
        assert!(matches!(&blocks[7], Block::Todo { line: 9, text, .. } if text.is_empty()));
        assert_eq!(blocks[8], Block::Quote { line: 10, spans: vec![text("Quote")] });
        assert_eq!(blocks[9], Block::Rule { line: 11 });
        assert_eq!(blocks[10], Block::Code { line: 12, text: "let x = 1;".into() });
        // `#tag` (no space) is a paragraph, not a heading.
        assert_eq!(blocks[11], Block::Paragraph { line: 15, spans: vec![text("#tag")] });
        assert_eq!(blocks.len(), 12);
        // The meeting template's empty bullet is a bullet, not a "-" paragraph.
        assert_eq!(parse("## Agenda\n- ")[1], Block::Bullet { line: 1, depth: 0, spans: vec![] });
        assert_eq!(open_todos(note), 1, "the empty to-do doesn't count");
    }

    #[test]
    fn inside_a_line() {
        assert_eq!(
            inline("a **big** and *small* `x` deal"),
            vec![
                text("a "),
                Span::Bold { text: "big".into() },
                text(" and "),
                Span::Italic { text: "small".into() },
                text(" "),
                Span::Code { text: "x".into() },
                text(" deal"),
            ]
        );
        assert_eq!(
            inline("See [the doc](https://ex.com/a) or https://ex.com/b."),
            vec![
                text("See "),
                Span::Link { text: "the doc".into(), href: "https://ex.com/a".into() },
                text(" or "),
                Span::Link { text: "https://ex.com/b".into(), href: "https://ex.com/b".into() },
                text("."),
            ]
        );
        // Not marks: snake_case, a lone star, unsafe links, an unclosed bold.
        assert_eq!(inline("my_file_name"), vec![text("my_file_name")]);
        assert_eq!(inline("2 * 3 = 6"), vec![text("2 * 3 = 6")]);
        assert_eq!(inline("[x](javascript:alert(1))"), vec![text("[x](javascript:alert(1))")]);
        assert_eq!(inline("**open"), vec![text("**open")]);
        // Accents and emoji don't trip the byte positions.
        assert_eq!(inline("é **où** ✓"), vec![text("é "), Span::Bold { text: "où".into() }, text(" ✓")]);
    }

    #[test]
    fn ticking_keeps_the_rest() {
        let note = "# Call\r\n- [ ] Ask for a credit note\n  * [x] Done one\nText";
        let ticked = set_todo(note, 1, true).unwrap();
        assert_eq!(ticked, "# Call\r\n- [x] Ask for a credit note\n  * [x] Done one\nText");
        let unticked = set_todo(&ticked, 2, false).unwrap();
        assert!(unticked.contains("  * [ ] Done one"));
        assert_eq!(set_todo(note, 0, true), None, "a heading isn't a to-do");
        assert_eq!(set_todo(note, 9, true), None);
    }

    #[test]
    fn snippets() {
        let note = "## Agenda\n- Planning for **Lyon**\n- Budget\n\nA long paragraph that goes on and on.";
        assert_eq!(snippet(note, 200), "Planning for Lyon · Budget · A long paragraph that goes on and on.");
        assert_eq!(snippet(note, 12), "Planning for…");
        assert_eq!(snippet("", 10), "");
    }
}
