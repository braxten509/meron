//! Discovery of standard mail folders.

use super::*;

/// Best-effort resolver for an account's Sent folder. Prefers IMAP SPECIAL-USE
/// (`\Sent` attribute) when the server advertises it; falls back to common name
/// patterns (Gmail's `[Gmail]/Sent Mail`, Dovecot's `Sent`, Courier's `INBOX.Sent`).
/// Returns `None` when the server has no LIST entries matching either path.
pub async fn find_sent_folder(session: &mut Session) -> Result<Option<String>> {
    let mut stream = session.list(Some(""), Some("*")).await.context("LIST")?;
    let mut by_attr: Option<String> = None;
    let mut by_name: Option<String> = None;
    while let Some(item) = stream.next().await {
        let entry = item.context("LIST item")?;
        let name = entry.name().to_string();
        for attr in entry.attributes() {
            if matches!(attr, async_imap::imap_proto::NameAttribute::Sent) {
                by_attr = Some(name.clone());
            }
        }
        if by_name.is_none() && looks_like_sent(&name) {
            by_name = Some(name);
        }
    }
    drop(stream);
    Ok(by_attr.or(by_name))
}

pub async fn find_trash_folder(session: &mut Session) -> Result<Option<String>> {
    let mut stream = session.list(Some(""), Some("*")).await.context("LIST")?;
    let mut by_attr: Option<String> = None;
    let mut by_name: Option<String> = None;
    while let Some(item) = stream.next().await {
        let entry = item.context("LIST item")?;
        let name = entry.name().to_string();
        for attr in entry.attributes() {
            if matches!(attr, async_imap::imap_proto::NameAttribute::Trash) {
                by_attr = Some(name.clone());
            }
        }
        if by_name.is_none() && looks_like_trash(&name) {
            by_name = Some(name);
        }
    }
    drop(stream);
    Ok(by_attr.or(by_name))
}

pub async fn find_archive_folder(session: &mut Session) -> Result<Option<String>> {
    let mut stream = session.list(Some(""), Some("*")).await.context("LIST")?;
    let mut by_attr: Option<String> = None;
    let mut by_name: Option<String> = None;
    while let Some(item) = stream.next().await {
        let entry = item.context("LIST item")?;
        let name = entry.name().to_string();
        for attr in entry.attributes() {
            if matches!(
                attr,
                async_imap::imap_proto::NameAttribute::Archive
                    | async_imap::imap_proto::NameAttribute::All
            ) {
                by_attr = Some(name.clone());
            }
        }
        if by_name.is_none() && looks_like_archive(&name) {
            by_name = Some(name);
        }
    }
    drop(stream);
    Ok(by_attr.or(by_name))
}

pub async fn find_drafts_folder(session: &mut Session) -> Result<Option<String>> {
    let mut stream = session.list(Some(""), Some("*")).await.context("LIST")?;
    let mut by_attr: Option<String> = None;
    let mut by_name: Option<String> = None;
    while let Some(item) = stream.next().await {
        let entry = item.context("LIST item")?;
        let name = entry.name().to_string();
        for attr in entry.attributes() {
            if matches!(attr, async_imap::imap_proto::NameAttribute::Drafts) {
                by_attr = Some(name.clone());
            }
        }
        if by_name.is_none() && looks_like_drafts(&name) {
            by_name = Some(name);
        }
    }
    drop(stream);
    Ok(by_attr.or(by_name))
}

pub fn looks_like_archive(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "archive"
            | "archives"
            | "all mail"
            | "inbox.archive"
            | "inbox.archives"
            | "[gmail]/all mail"
            | "[google mail]/all mail"
    )
}

pub fn looks_like_sent(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    // Gmail's localized account uses "[Gmail]/Sent Mail"; Dovecot ships "Sent";
    // Courier/Cyrus often namespace under INBOX (`INBOX.Sent`).
    matches!(
        n.as_str(),
        "sent" | "sent mail" | "sent messages" | "sent items" | "inbox.sent" | "[gmail]/sent mail"
    )
}

pub fn looks_like_trash(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "trash"
            | "deleted"
            | "deleted items"
            | "deleted messages"
            | "bin"
            | "inbox.trash"
            | "inbox.deleted"
            | "inbox.deleted items"
            | "[gmail]/trash"
            | "[gmail]/bin"
    )
}

pub fn looks_like_junk(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "junk" | "spam" | "bulk mail" | "inbox.junk" | "inbox.spam" | "[gmail]/spam"
    )
}

pub fn looks_like_drafts(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "drafts" | "draft" | "inbox.drafts" | "inbox.draft" | "[gmail]/drafts" | "[gmail]/draft"
    )
}
