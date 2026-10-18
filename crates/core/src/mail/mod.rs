//! Mail: data types, parsing, and the IMAP client.

pub mod demo;
mod folder;
pub(crate) mod imap;
mod message;
mod query;
mod sanitize;
mod summary;

pub use folder::{Folder, FolderRole, sort_folders};
pub use imap::{
    ActionOutcome, append_message, archive_messages, fetch_attachment, fetch_message, fetch_messages,
    fetch_raw_message, find_by_message_id, list_folders, move_messages, set_answered, set_flag, set_seen,
    trash_messages,
};
pub use message::{Attachment, AttachmentData, MessageDetail, RenderOptions, extract_attachment};
pub use query::{ListQuery, MessagePage};
pub use summary::{Digest, FileInfo, MessageSummary, Person};
