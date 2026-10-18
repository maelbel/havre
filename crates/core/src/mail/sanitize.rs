//! Making email HTML safe to display.
//!
//! An email is untrusted content: it can contain scripts, forms, or
//! "tracking pixels" (tiny remote images that tell the sender you opened it).
//! We use `ammonia` to keep only harmless HTML, and we drop remote images
//! unless the user asks for them.
//!
//! The web UI adds a second layer: the result is shown in a sandboxed
//! iframe with a strict Content-Security-Policy.

use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// The result of sanitizing one HTML body.
pub struct Sanitized {
    pub html: String,
    /// `true` if at least one remote image was removed.
    pub blocked_remote_images: bool,
}

/// Cleans `html`.
///
/// - `inline_images` maps a Content-ID (`logo@example`) to the URL where the
///   attached image can be loaded; `<img src="cid:logo@example">` is rewritten.
/// - `allow_remote_images`: keep `http(s)` images (the user clicked "Load images").
pub fn sanitize_html(
    html: &str,
    inline_images: HashMap<String, String>,
    allow_remote_images: bool,
) -> Sanitized {
    // Shared between this function and the filter closure below.
    // `Arc` = shared ownership across threads, `AtomicBool` = a bool that
    // can be changed through a shared reference.
    let blocked = Arc::new(AtomicBool::new(false));
    let blocked_in_filter = Arc::clone(&blocked);

    let mut builder = ammonia::Builder::default();
    builder
        // Emails are laid out with old-school HTML attributes and inline styles.
        .add_generic_attributes(&[
            "style",
            "align",
            "valign",
            "bgcolor",
            "width",
            "height",
            "border",
            "cellpadding",
            "cellspacing",
            "color",
            "dir",
            "class",
        ])
        .add_tags(&["font", "center", "style"])
        .add_tag_attributes("font", &["face", "size"])
        // `<style>` blocks are kept (newsletters need them); ammonia would
        // otherwise delete them together with their content.
        .rm_clean_content_tags(&["style"])
        // `cid:` = an image attached to the email itself (rewritten below).
        .add_url_schemes(&["cid"])
        .link_rel(Some("noopener noreferrer"))
        .attribute_filter(move |element, attribute, value| {
            if element == "img" && attribute == "src" {
                if let Some(id) = value.strip_prefix("cid:") {
                    return inline_images.get(id).map(|url| Cow::Owned(url.clone()));
                }
                let remote =
                    value.starts_with("http://") || value.starts_with("https://") || value.starts_with("//");
                if remote && !allow_remote_images {
                    blocked_in_filter.store(true, Ordering::Relaxed);
                    return None;
                }
            }
            Some(Cow::Borrowed(value))
        });

    let html = builder.clean(html).to_string();
    Sanitized { html, blocked_remote_images: blocked.load(Ordering::Relaxed) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_scripts_and_event_handlers() {
        let out = sanitize_html(
            r#"<p onclick="steal()">Hi</p><script>alert(1)</script><a href="javascript:x()">x</a>"#,
            HashMap::new(),
            false,
        );
        assert!(!out.html.contains("script"));
        assert!(!out.html.contains("onclick"));
        assert!(!out.html.contains("javascript:"));
        assert!(out.html.contains("<p>Hi</p>"));
    }

    #[test]
    fn blocks_remote_images_unless_allowed() {
        let html = r#"<img src="https://tracker.example/p.gif"><p style="color:red">ok</p>"#;
        let blocked = sanitize_html(html, HashMap::new(), false);
        assert!(blocked.blocked_remote_images);
        assert!(!blocked.html.contains("tracker.example"));
        assert!(blocked.html.contains(r#"style="color:red""#));

        let allowed = sanitize_html(html, HashMap::new(), true);
        assert!(!allowed.blocked_remote_images);
        assert!(allowed.html.contains("tracker.example"));
    }

    #[test]
    fn rewrites_inline_images() {
        let map = HashMap::from([("logo@x".to_owned(), "/api/img/0".to_owned())]);
        let out = sanitize_html(r#"<img src="cid:logo@x" alt="logo">"#, map, false);
        assert!(out.html.contains(r#"src="/api/img/0""#));
        assert!(!out.blocked_remote_images);
    }
}
