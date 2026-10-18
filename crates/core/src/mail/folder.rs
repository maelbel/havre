//! Mail folders ("mailboxes" in IMAP).

use async_imap::types::NameAttribute;
use serde::Serialize;

/// What a folder is for. Lets the UI show the right icon and order,
/// whatever the folder is called on the server ("Sent", "Envoyés"…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderRole {
    // The order here is the order in the sidebar.
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    All,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Folder {
    /// Exact name on the server, used to select it. Can look odd
    /// (`&AMk-l&AOk-ments envoy&AOk-s`): never show it, use `name`.
    pub path: String,
    /// Human name of the last level, decoded: `Éléments envoyés`.
    pub name: String,
    /// Nesting level: 0 for top-level folders.
    pub depth: usize,
    pub role: FolderRole,
    /// The server itself declared the role (SPECIAL-USE), rather than us
    /// guessing from the name. Preferred when two folders share a role.
    #[serde(skip)]
    pub declared_role: bool,
}

impl Folder {
    /// Builds a folder from one line of the IMAP `LIST` answer.
    /// Returns `None` for entries that cannot be opened (`\Noselect`),
    /// like Gmail's `[Gmail]` parent.
    pub fn from_list(path: &str, delimiter: Option<&str>, attributes: &[NameAttribute<'_>]) -> Option<Self> {
        if attributes.iter().any(|a| matches!(a, NameAttribute::NoSelect)) {
            return None;
        }

        // IMAP folder names use "modified UTF-7": `&AOk-` means `é`.
        let decoded = utf7_imap::decode_utf7_imap(path.to_owned());
        let (depth, name) = match delimiter {
            Some(d) if !d.is_empty() => {
                let parts: Vec<&str> = decoded.split(d).collect();
                (parts.len() - 1, parts.last().copied().unwrap_or_default().to_owned())
            }
            _ => (0, decoded.clone()),
        };

        let declared = role_from_attributes(attributes);
        let role = declared.unwrap_or_else(|| role_from_name(path, &name));
        // Gmail-style servers nest special folders (`[Gmail]/Sent Mail`);
        // show them at the top level like any other special folder.
        let depth = if role == FolderRole::Other { depth } else { 0 };

        Some(Self { path: path.to_owned(), name, depth, role, declared_role: declared.is_some() })
    }
}

/// Sorts folders for the sidebar: special folders first, then the rest
/// alphabetically (keeping children right after their parent).
pub fn sort_folders(folders: &mut [Folder]) {
    folders
        .sort_by(|a, b| a.role.cmp(&b.role).then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase())));
}

/// Servers that support SPECIAL-USE (RFC 6154) tell us directly.
fn role_from_attributes(attributes: &[NameAttribute<'_>]) -> Option<FolderRole> {
    attributes.iter().find_map(|a| match a {
        NameAttribute::Sent => Some(FolderRole::Sent),
        NameAttribute::Drafts => Some(FolderRole::Drafts),
        NameAttribute::Trash => Some(FolderRole::Trash),
        NameAttribute::Junk => Some(FolderRole::Junk),
        NameAttribute::Archive => Some(FolderRole::Archive),
        NameAttribute::All => Some(FolderRole::All),
        _ => None,
    })
}

/// Older servers don't, so we guess from common names (English and French).
fn role_from_name(path: &str, name: &str) -> FolderRole {
    if path.eq_ignore_ascii_case("INBOX") {
        return FolderRole::Inbox;
    }
    let n = name.to_lowercase();
    let is = |names: &[&str]| names.contains(&n.as_str());
    if is(&[
        "sent",
        "sent items",
        "sent messages",
        "sent mail",
        "envoyés",
        "messages envoyés",
        "éléments envoyés",
    ]) {
        FolderRole::Sent
    } else if is(&["drafts", "draft", "brouillons"]) {
        FolderRole::Drafts
    } else if is(&["trash", "deleted items", "deleted messages", "bin", "corbeille", "éléments supprimés"])
    {
        FolderRole::Trash
    } else if is(&[
        "junk",
        "spam",
        "junk e-mail",
        "junk email",
        "courrier indésirable",
        "pourriel",
        "indésirables",
    ]) {
        FolderRole::Junk
    } else if is(&["archive", "archives"]) {
        FolderRole::Archive
    } else {
        FolderRole::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf7_and_guesses_role() {
        let f = Folder::from_list("&AMk-l&AOk-ments envoy&AOk-s", Some("/"), &[]).unwrap();
        assert_eq!(f.name, "Éléments envoyés");
        assert_eq!(f.role, FolderRole::Sent);
        assert_eq!(f.path, "&AMk-l&AOk-ments envoy&AOk-s");
    }

    #[test]
    fn uses_special_use_attributes_and_skips_noselect() {
        let f = Folder::from_list("[Gmail]/Corbeille", Some("/"), &[NameAttribute::Trash]).unwrap();
        assert_eq!((f.role, f.depth, f.name.as_str()), (FolderRole::Trash, 0, "Corbeille"));
        assert!(Folder::from_list("[Gmail]", Some("/"), &[NameAttribute::NoSelect]).is_none());
    }

    #[test]
    fn nested_folders_keep_depth_and_sort_after_specials() {
        let mut folders = vec![
            Folder::from_list("Clients/Lyon", Some("/"), &[]).unwrap(),
            Folder::from_list("Trash", Some("/"), &[]).unwrap(),
            Folder::from_list("INBOX", Some("/"), &[]).unwrap(),
            Folder::from_list("Clients", Some("/"), &[]).unwrap(),
        ];
        sort_folders(&mut folders);
        let order: Vec<_> = folders.iter().map(|f| (f.name.as_str(), f.depth)).collect();
        assert_eq!(order, [("INBOX", 0), ("Trash", 0), ("Clients", 0), ("Lyon", 1)]);
    }
}
