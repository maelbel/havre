//! Small rules about people: robots, companies, birthdays. Plain functions,
//! tested on their own.

use chrono::{Datelike, NaiveDate};

/// Does this address belong to a program rather than a person? Read from
/// the part before the `@` (`no-reply@`, `notifications@`, `newsletter@`…).
/// You can say otherwise on the person's page.
pub fn looks_automated(email: &str) -> bool {
    let local = email.split('@').next().unwrap_or_default().to_lowercase();
    // `bounces+123@…`: what comes before a `+` is the mailbox (`news.anchor@` stays a person).
    let head = local.split('+').next().unwrap_or_default();
    const EXACT: &[&str] = &[
        "notification",
        "notifications",
        "notify",
        "newsletter",
        "newsletters",
        "news",
        "mailer-daemon",
        "postmaster",
        "bounce",
        "bounces",
        "alert",
        "alerts",
        "digest",
        "automated",
        "robot",
        "calendar-notification",
    ];
    const ANYWHERE: &[&str] =
        &["noreply", "no-reply", "no_reply", "donotreply", "do-not-reply", "do_not_reply"];
    EXACT.contains(&head) || EXACT.contains(&local.as_str()) || ANYWHERE.iter().any(|w| local.contains(w))
}

/// Mail services anyone can sign up to: their domain says nothing about
/// where someone works.
const PUBLIC_DOMAINS: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "outlook.fr",
    "hotmail.com",
    "hotmail.fr",
    "live.com",
    "live.fr",
    "msn.com",
    "yahoo.com",
    "yahoo.fr",
    "icloud.com",
    "me.com",
    "mac.com",
    "aol.com",
    "gmx.com",
    "gmx.fr",
    "gmx.de",
    "proton.me",
    "protonmail.com",
    "pm.me",
    "laposte.net",
    "orange.fr",
    "wanadoo.fr",
    "free.fr",
    "sfr.fr",
    "neuf.fr",
    "bbox.fr",
    "hey.com",
    "fastmail.com",
    "mail.com",
    "yandex.com",
    "zoho.com",
    "tutanota.com",
    "tuta.io",
];

/// Where someone works: the company you wrote, else their address's domain
/// (`claire@acme.fr` → `acme.fr`), unless it's a public mail service.
pub fn organization(company: &str, email: &str) -> Option<String> {
    let company = company.trim();
    if !company.is_empty() {
        return Some(company.to_owned());
    }
    let domain = email.rsplit_once('@')?.1.trim().to_lowercase();
    (!domain.is_empty() && !PUBLIC_DOMAINS.contains(&domain.as_str())).then_some(domain)
}

/// A birthday as written: `YYYY-MM-DD`, or `--MM-DD` when you don't know
/// the year (vCard's way). Returns (month, day, year).
pub fn parse_birthday(text: &str) -> Option<(u32, u32, Option<i32>)> {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("--") {
        let (m, d) = rest.split_once('-')?;
        let (m, d): (u32, u32) = (m.parse().ok()?, d.parse().ok()?);
        // Checked against a leap year, so 29 February is fine.
        NaiveDate::from_ymd_opt(2000, m, d)?;
        return Some((m, d, None));
    }
    let date = NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
    Some((date.month(), date.day(), Some(date.year())))
}

/// The next birthday on or after `today`, and the age then (if the year is
/// known). Born on 29 February: the 28th in other years.
pub fn next_birthday(text: &str, today: NaiveDate) -> Option<(NaiveDate, Option<i32>)> {
    let (month, day, born) = parse_birthday(text)?;
    let on = |year: i32| {
        NaiveDate::from_ymd_opt(year, month, day).or_else(|| NaiveDate::from_ymd_opt(year, 2, 28))
    };
    let this_year = on(today.year())?;
    let next = if this_year >= today { this_year } else { on(today.year() + 1)? };
    Some((next, born.map(|b| next.year() - b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_robots_not_people() {
        for robot in [
            "noreply@github.com",
            "no-reply@accounts.google.com",
            "notifications@github.com",
            "newsletter@lemonde.fr",
            "MAILER-DAEMON@mail.example.org",
            "bounces+12345@sendgrid.net",
            "do-not-reply@sncf.fr",
            "calendar-notification@google.com",
        ] {
            assert!(looks_automated(robot), "{robot}");
        }
        for person in ["claire@acme.fr", "noemie@example.org", "news.anchor@tv.fr", "thomas.alert@x.fr"] {
            assert!(!looks_automated(person), "{person}");
        }
    }

    #[test]
    fn companies_from_the_field_or_the_domain() {
        assert_eq!(organization("Acme", "claire@gmail.com").as_deref(), Some("Acme"));
        assert_eq!(organization("", "claire@Acme.fr").as_deref(), Some("acme.fr"));
        assert_eq!(organization("  ", "claire@gmail.com"), None);
        assert_eq!(organization("", "mael@laposte.net"), None);
    }

    #[test]
    fn birthdays_with_or_without_a_year() {
        assert_eq!(parse_birthday("1990-05-17"), Some((5, 17, Some(1990))));
        assert_eq!(parse_birthday("--12-03"), Some((12, 3, None)));
        assert_eq!(parse_birthday("--02-29"), Some((2, 29, None)));
        for bad in ["1990-02-30", "--13-01", "17/05/1990", "", "--5"] {
            assert_eq!(parse_birthday(bad), None, "{bad}");
        }
        let today = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(next_birthday("1990-11-09", today), Some((day(2026, 11, 9), Some(36))), "today counts");
        assert_eq!(next_birthday("1990-05-17", today), Some((day(2027, 5, 17), Some(37))), "next year");
        assert_eq!(next_birthday("--12-03", today), Some((day(2026, 12, 3), None)));
        // 29 February: the 28th in 2027, the 29th in 2028.
        assert_eq!(next_birthday("2000-02-29", today), Some((day(2027, 2, 28), Some(27))));
        assert_eq!(next_birthday("2000-02-29", day(2028, 1, 1)), Some((day(2028, 2, 29), Some(28))));
    }
}
