//! `havre-cli` — try the core from a terminal.
//!
//! ```text
//! cargo run -p havre-cli -- mail folders
//! cargo run -p havre-cli -- mail list --folder INBOX --limit 20
//! cargo run -p havre-cli -- mail show 1234
//! ```

use clap::{Parser, Subcommand};
use havre_core::{
    config::ImapConfig,
    mail::{self, RenderOptions},
};

/// `clap` reads these doc comments and turns them into `--help` text.
#[derive(Parser)]
#[command(name = "havre-cli", version, about = "Havre from the command line")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Mail commands.
    Mail {
        #[command(subcommand)]
        action: MailAction,
    },
}

#[derive(Subcommand)]
enum MailAction {
    /// List the folders of the mailbox.
    Folders,
    /// List the most recent messages of a folder.
    List {
        /// Folder path, as shown by `mail folders`.
        #[arg(short, long, default_value = "INBOX")]
        folder: String,
        /// How many messages to show.
        #[arg(short, long, default_value_t = 20)]
        limit: u32,
        /// Only messages containing this text (searched by the mail server).
        #[arg(short, long)]
        search: Option<String>,
    },
    /// Print one message as text (does not mark it as read).
    Show {
        /// The message UID, as shown by `mail list`.
        uid: u32,
        #[arg(short, long, default_value = "INBOX")]
        folder: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt().with_env_filter("havre_core=info").init();

    let Command::Mail { action } = Cli::parse().command;
    let config = ImapConfig::from_env()?;

    match action {
        MailAction::Folders => {
            for f in mail::list_folders(&config).await? {
                let indent = "  ".repeat(f.depth);
                println!("{indent}{:<30} {:<8} {}", f.name, format!("{:?}", f.role).to_lowercase(), f.path);
            }
        }
        MailAction::List { folder, limit, search } => {
            let query = mail::ListQuery { limit, before_uid: None, search };
            let messages = mail::fetch_messages(&config, &folder, &query).await?.messages;
            if messages.is_empty() {
                println!("{folder} is empty.");
            }
            for m in messages {
                let marker = if m.unread { "●" } else { " " };
                let from =
                    m.from.and_then(|p| p.name.or(p.address)).unwrap_or_else(|| "(unknown sender)".into());
                let date = m.date.as_deref().map(|d| &d[..10]).unwrap_or("          ");
                println!("{marker} {:>6}  {date}  {from:<28.28}  {}", m.uid, m.subject);
            }
        }
        MailAction::Show { uid, folder } => {
            let no_urls = |_| String::new();
            let options = RenderOptions { allow_remote_images: false, attachment_url: &no_urls };
            let m = mail::fetch_message(&config, &folder, uid, false, &options).await?;
            let who = |p: &mail::Person| match (&p.name, &p.address) {
                (Some(n), Some(a)) => format!("{n} <{a}>"),
                (n, a) => n.clone().or(a.clone()).unwrap_or_default(),
            };
            println!("From:    {}", m.from.as_ref().map(who).unwrap_or_default());
            println!("To:      {}", m.to.iter().map(who).collect::<Vec<_>>().join(", "));
            println!("Date:    {}", m.date.unwrap_or_default());
            println!("Subject: {}\n", m.subject);
            println!("{}", m.text.unwrap_or_else(|| "(no text body)".into()).trim());
            for a in m.attachments.iter().filter(|a| !a.inline) {
                println!("\nAttachment: {} ({}, {} KB)", a.filename, a.mime, a.size.div_ceil(1024));
            }
        }
    }
    Ok(())
}
