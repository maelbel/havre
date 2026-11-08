//! A snapshot ("golden file") test of everything Havre reads from an email.
//!
//! Each sample in `testdata/*.eml` goes through every function that parses
//! mail (the reader, the list, previews, replies, the AI's text…), and the
//! results are written as text to `testdata/parsed.txt`. The test fails when
//! the output changes, and shows what changed.
//!
//! Why: updating the mail parser (mail-parser 0.11 → 1.0) must not change
//! what you see. When a change is wanted, rerun with `HAVRE_BLESS=1` to write
//! the new snapshot, and review its diff like code:
//!
//! ```sh
//! HAVRE_BLESS=1 cargo test -p havre-core parse_cases
//! ```
//!
//! What it teaches: `include_bytes!` and `concat!` run at compile time, and
//! `{:#?}` prints any `Debug` value, nested and indented.

use std::fmt::Write;

use super::{Digest, MessageDetail, MessageSummary, RenderOptions, extract_attachment};
use crate::compose::{DraftKind, prefill};

/// `include_bytes!` puts each file inside the test binary, so the test needs no paths at run time.
macro_rules! sample {
    ($name:literal) => {
        ($name, include_bytes!(concat!("testdata/", $name)).as_slice())
    };
}

const SAMPLES: &[(&str, &[u8])] = &[
    sample!("1-plain-reply.eml"),
    sample!("2-alternative.eml"),
    sample!("3-html-only.eml"),
    sample!("4-attachments.eml"),
    sample!("5-invitation.eml"),
    sample!("6-invitation-reply.eml"),
    sample!("7-groups-latin1.eml"),
    sample!("8-broken.eml"),
    sample!("9-forwarded.eml"),
];

const SNAPSHOT: &str = "src/mail/testdata/parsed.txt";

/// The headers alone: what the list gets from IMAP (`BODY.PEEK[HEADER]`).
fn headers(raw: &[u8]) -> &[u8] {
    match raw.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(end) => &raw[..end + 4],
        None => raw,
    }
}

fn describe(raw: &[u8]) -> String {
    let url = |i: usize| format!("/attachment/{i}");
    let mut out = String::new();
    // `writeln!` into a String can't fail; `.unwrap()` just says so.
    let mut line = |label: &str, value: String| writeln!(out, "--- {label}\n{value}").unwrap();

    for allow in [false, true] {
        let detail = MessageDetail::parse(
            7,
            "INBOX",
            true,
            raw,
            &RenderOptions { allow_remote_images: allow, attachment_url: &url },
        );
        line(&format!("reader (remote images allowed: {allow})"), format!("{detail:#?}"));
    }
    let files: Vec<String> = (0..5)
        .map_while(|i| extract_attachment(raw, i))
        .map(|a| {
            format!(
                "{} · {} · {} bytes · {:02x?}",
                a.filename,
                a.mime,
                a.bytes.len(),
                &a.bytes[..a.bytes.len().min(8)]
            )
        })
        .collect();
    line("downloads", format!("{files:#?}"));
    line("list row", format!("{:#?}", MessageSummary::from_headers(7, true, headers(raw))));
    line("preview", format!("{:#?}", Digest::of(raw)));
    let own = ["mael@example.net".to_owned()];
    for kind in [DraftKind::Reply, DraftKind::ReplyAll, DraftKind::Forward] {
        line(&format!("prefill {kind:?}"), format!("{:#?}", prefill(kind, raw, &own)));
    }
    line("search text", crate::ai::index::body_text(raw));
    line("assistant text", crate::ai::full_text(raw, 2000));
    out
}

#[test]
fn every_sample_reads_as_before() {
    let mut actual = String::new();
    for (name, raw) in SAMPLES {
        writeln!(actual, "===== {name}\n{}", describe(raw)).unwrap();
    }
    // Some texts keep the email's CRLF line ends; written as `\r`, the snapshot stays a plain
    // text file that Git can't change on checkout (it turns CRLF into LF here, see .gitattributes).
    let actual = actual.replace('\r', "\\r");
    // Paths in tests are relative to the crate (crates/core), where `cargo test` runs them.
    if std::env::var_os("HAVRE_BLESS").is_some() {
        std::fs::write(SNAPSHOT, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(SNAPSHOT).unwrap_or_default();
    if actual != expected {
        // Show the first lines that differ, with their name, rather than two huge strings.
        let mut section = "";
        for (n, (a, e)) in actual.lines().zip(expected.lines()).enumerate() {
            if a.starts_with("=====") || a.starts_with("---") {
                section = a;
            }
            if a != e {
                panic!(
                    "line {}, in {section}:\n  expected: {e}\n  actual:   {a}\n\
                     If the change is wanted: HAVRE_BLESS=1 cargo test -p havre-core parse_cases",
                    n + 1
                );
            }
        }
        panic!("the snapshot has {} lines, the output {}", expected.lines().count(), actual.lines().count());
    }
}
