//! Folder discovery, status and deletion.
//!
//! Serves the folder list and folder-management commands.
//! Distinguishes a server refusal from a lost connection during mailbox operations.

use super::*;

pub async fn list_folders(session: &mut Session) -> Result<Vec<Folder>> {
    let mut out = Vec::new();
    let mut stream = session.list(Some(""), Some("*")).await.context("LIST")?;
    while let Some(item) = stream.next().await {
        let name = item.context("LIST item")?;
        let special_use = name.attributes().iter().find_map(|attr| {
            use async_imap::imap_proto::NameAttribute;
            match attr {
                NameAttribute::Drafts => Some("drafts"),
                NameAttribute::Sent => Some("sent"),
                NameAttribute::Trash => Some("trash"),
                NameAttribute::Junk => Some("junk"),
                NameAttribute::Archive => Some("archive"),
                NameAttribute::All => Some("all"),
                NameAttribute::Flagged => Some("flagged"),
                _ => None,
            }
            .map(str::to_string)
        });
        out.push(Folder {
            display_name: crate::utf7::decode(name.name()),
            name: name.name().to_string(),
            delimiter: name.delimiter().map(|d| d.to_string()),
            unread: 0,
            special_use,
            role: String::new(),
            notify: false,
        });
    }
    Ok(out)
}

/// `(UIDVALIDITY, UIDNEXT)` of `folder` from one STATUS, without selecting
/// it: how a caller tells, cheaply, whether new mail arrived since its sync.
pub async fn folder_status(session: &mut Session, folder: &str) -> Result<(u32, u32)> {
    let mailbox = session
        .status(folder, "(UIDNEXT UIDVALIDITY)")
        .await
        .with_context(|| format!("STATUS {folder}"))?;
    Ok((
        mailbox.uid_validity.unwrap_or(0),
        mailbox.uid_next.unwrap_or(0),
    ))
}

/// The server's NO or BAD answer somewhere in `err`, if that is what it is (as
/// opposed to a dropped connection or a timeout).
fn server_refusal(err: &anyhow::Error) -> Option<&str> {
    err.chain().find_map(
        |cause| match cause.downcast_ref::<async_imap::error::Error>()? {
            async_imap::error::Error::No(text) | async_imap::error::Error::Bad(text) => {
                Some(text.as_str())
            }
            _ => None,
        },
    )
}

/// Whether the server refused a command on a mailbox because it can't be
/// opened at all: it doesn't exist (`[NONEXISTENT]`) or is a `\Noselect`
/// container. Asking again won't help, unlike a dropped connection or a
/// temporary refusal (`[UNAVAILABLE]`, `[INUSE]`).
pub fn is_unselectable_mailbox(err: &anyhow::Error) -> bool {
    let Some(text) = server_refusal(err) else {
        return false;
    };
    let text = text.to_ascii_uppercase();
    if text.contains("[UNAVAILABLE]") || text.contains("[INUSE]") || text.contains("[LIMIT]") {
        return false;
    }
    [
        "[NONEXISTENT]",
        "NOSELECT",
        "NOT SELECTABLE",
        "ISN'T SELECTABLE",
        "IS NOT SELECTABLE",
        "CANNOT BE SELECTED",
        "DOES NOT EXIST",
        "DOESN'T EXIST",
        "NOT EXIST",
        "NO SUCH MAILBOX",
        "NO SUCH FOLDER",
        "UNKNOWN MAILBOX",
        "MAILBOX NOT FOUND",
        "FOLDER NOT FOUND",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// Whether the server answered NO or BAD (any reason), rather than the
/// connection failing.
pub fn is_server_refusal(err: &anyhow::Error) -> bool {
    server_refusal(err).is_some()
}

pub async fn create_folder(session: &mut Session, name: &str) -> Result<()> {
    session.create(name).await.context("CREATE")?;
    Ok(())
}

/// Rename a folder on the server. RFC 3501 renames its inferiors with it. Run
/// [`prepare_folder_delete`] first: like DELETE, some servers refuse to rename
/// the mailbox the session has selected.
pub async fn rename_folder(session: &mut Session, from: &str, to: &str) -> Result<()> {
    session.rename(from, to).await.context("RENAME")?;
    Ok(())
}

/// A multi-folder delete that failed after removing part of its target list.
/// Callers must reconcile `removed` before surfacing the failure.
#[derive(Debug)]
pub struct PartialFolderDelete {
    pub removed: Vec<String>,
    failure: String,
}

impl std::fmt::Display for PartialFolderDelete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} folder(s) were deleted before the server rejected the operation: {}",
            self.removed.len(),
            self.failure
        )
    }
}

impl std::error::Error for PartialFolderDelete {}

impl PartialFolderDelete {
    pub fn into_parts(self) -> (Vec<String>, String) {
        let warning = self.to_string();
        (self.removed, warning)
    }
}

/// Move a reused session away from any mailbox that is about to be deleted.
/// This is a read-only preflight so the session pool may safely repeat it on a
/// fresh connection when a warm connection has gone stale.
pub async fn prepare_folder_delete(session: &mut Session) -> Result<()> {
    // Pooled sessions retain their selected mailbox. Some servers refuse to
    // delete that mailbox, so safely move the session to INBOX first. EXAMINE
    // implicitly deselects without expunging messages marked \Deleted.
    session
        .examine("INBOX")
        .await
        .context("EXAMINE INBOX before DELETE")?;
    Ok(())
}

/// Delete folders on the server, messages and all. The caller must first run
/// [`prepare_folder_delete`] on the same session. `names` is deleted in order,
/// so a subtree must arrive deepest first — RFC 3501 lets a server refuse DELETE
/// on a mailbox that still has inferiors. Returns the names actually removed;
/// if a later command fails, the error carries the successful prefix so callers
/// can reconcile irreversible changes. Callers must refuse special-use folders
/// first: IMAP happily deletes Sent or Archive, and nothing on the server brings
/// them back.
pub async fn delete_folders(session: &mut Session, names: &[String]) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for name in names {
        if let Err(err) = session.delete(name).await {
            let failure = format!("DELETE {name}: {err:#}");
            if removed.is_empty() {
                return Err(anyhow::anyhow!(failure));
            }
            return Err(PartialFolderDelete { removed, failure }.into());
        }
        removed.push(name.clone());
    }
    Ok(removed)
}

/// Whether `err` is a FETCH response our IMAP parser could not read, as opposed
/// to a network or server failure.
///
/// This matters because such a response is unrecoverable *on that connection*:
/// async-imap reports the parse failure without consuming the offending bytes
/// from its read buffer, so every later poll re-parses them and fails again.
/// The only way forward is to drop the session and re-fetch a range that
/// excludes the message — see `fetch_headers_isolating_unparseable` in
/// `engine/sync.rs`.
///
/// Detection is by message text because async-imap surfaces the failure as a
/// plain `io::Error::other`, with no variant to match on. The substring is the
/// literal from its `ImapStream::decode`; if the pinned fork moves, this needs
/// rechecking (`unparseable_response_marker_matches_async_imap` covers it).
pub fn is_unparseable_response(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.to_string().contains(UNPARSEABLE_RESPONSE_MARKER))
    })
}

const UNPARSEABLE_RESPONSE_MARKER: &str = " during parsing of ";
