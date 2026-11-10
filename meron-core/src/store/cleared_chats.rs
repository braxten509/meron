//! What a conversation leaves out when it is read from one folder.
//!
//! A conversation gathers its mail from every folder, so a reply kept in Sent
//! shows beside the Inbox mail it answers. Two kinds of mail stay out:
//!
//! * Copies sitting in Trash or Junk, unless the conversation is read from
//!   inside that folder. Deleted mail must not come back because a new reply
//!   arrived in its thread.
//! * The reader's own Sent mail carrying
//!   [`LEFT_OUT_OF_CHAT_TAG`](crate::tags::LEFT_OUT_OF_CHAT_TAG). Deleting a
//!   whole conversation moves the other side's mail to Trash but leaves the
//!   reader's replies in Sent; they get this tag, so the conversation starts
//!   fresh when new mail arrives. The tag is stored and synced like any tag,
//!   so every device leaves the same replies out. They still show in Sent.
//!   Replies of a conversation that still has mail filed in another folder
//!   are not tagged: that conversation was not deleted.
//!
//! [`ChatScope`] is the one place both rules live: the reader and every tally
//! of the thread list ask it, so a card never contradicts what opening it shows.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use rusqlite::{Connection, params_from_iter};

use super::{
    change_tags, classify_folder_role, folder_for_role, get_folders, keys_with_tag,
    message_tag_keys, sender_key_addr, thread_uids_by_folder,
};
use crate::tags::LEFT_OUT_OF_CHAT_TAG;

/// What a conversation read from one folder of an account leaves out.
pub struct ChatScope {
    viewed: String,
    /// `special_use` of the account's known folders, by name.
    special_use: HashMap<String, Option<String>>,
    /// Tag keys of the messages carrying the left-out tag.
    left_out: HashSet<String>,
}

impl ChatScope {
    /// The scope of conversations read from `viewed`.
    pub fn for_folder(conn: &Connection, account: &str, viewed: &str) -> Result<Self> {
        Ok(Self {
            viewed: viewed.to_string(),
            special_use: get_folders(conn, account)?
                .into_iter()
                .map(|folder| (folder.name, folder.special_use))
                .collect(),
            left_out: keys_with_tag(conn, account, LEFT_OUT_OF_CHAT_TAG)?,
        })
    }

    /// The role of `folder`, by its saved special use or else its name.
    fn role(&self, folder: &str) -> &'static str {
        let special_use = self
            .special_use
            .get(folder)
            .and_then(|special| special.as_deref());
        classify_folder_role(folder, special_use)
    }

    /// Whether the copy in `folder` with tag key `tag_key` stays out of the
    /// conversation. The folder being read never loses its own mail.
    pub fn leaves_out(&self, folder: &str, tag_key: &str) -> bool {
        folder != self.viewed
            && (matches!(self.role(folder), "trash" | "junk") || self.left_out.contains(tag_key))
    }

    /// Whether mail in `folder` means its conversation is still kept: anything
    /// but the reader's own copies (Sent, Drafts) and thrown-away mail.
    fn keeps_conversation(&self, folder: &str) -> bool {
        !matches!(self.role(folder), "sent" | "drafts" | "trash" | "junk")
    }
}

/// Of the real thread keys in `threads`, those that still hold kept mail (see
/// [`ChatScope::keeps_conversation`]).
fn kept_threads(
    conn: &Connection,
    account: &str,
    scope: &ChatScope,
    threads: &[String],
) -> Result<HashSet<String>> {
    let mut kept = HashSet::new();
    // SQLite caps bound parameters per statement.
    for chunk in threads.chunks(100) {
        let placeholders = (2..2 + chunk.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT DISTINCT thread_key, folder FROM messages
              WHERE account = ?1 AND uid <> 0 AND thread_key IN ({placeholders})"
        ))?;
        let mut args: Vec<&str> = vec![account];
        args.extend(chunk.iter().map(String::as_str));
        let rows = stmt.query_map(params_from_iter(args), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (thread, folder) = row?;
            if scope.keeps_conversation(&folder) {
                kept.insert(thread);
            }
        }
    }
    Ok(kept)
}

/// The reader's own Sent mail that belonged to the conversation just deleted
/// from `folder`, grouped by mailbox. `thread_key` is a root thread key (with
/// `subject` naming its branch) or a sender row's key.
///
/// Only conversations with no kept mail left count: a reply whose conversation
/// still has mail filed in another folder stays part of that conversation.
fn own_replies_of_deleted(
    conn: &Connection,
    account: &str,
    folder: &str,
    thread_key: &str,
    subject: Option<&str>,
) -> Result<BTreeMap<String, Vec<u32>>> {
    let scope = ChatScope::for_folder(conn, account, folder)?;
    let mut copies: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    // Deleting a draft, a Sent copy or thrown-away mail deletes no conversation.
    if !scope.keeps_conversation(folder) {
        return Ok(copies);
    }
    let Some(addr) = sender_key_addr(thread_key) else {
        let by_folder = thread_uids_by_folder(conn, account, folder, thread_key, subject, false)?;
        if by_folder.keys().any(|other| scope.keeps_conversation(other)) {
            return Ok(copies);
        }
        copies.extend(
            by_folder
                .into_iter()
                .filter(|(other, _)| scope.role(other) == "sent"),
        );
        return Ok(copies);
    };
    // A sender row: the Sent mail to that person (see `sender_chat_headers`).
    if addr.is_empty() {
        return Ok(copies);
    }
    let Some(sent) = folder_for_role(conn, account, "sent")?.filter(|sent| sent != folder) else {
        return Ok(copies);
    };
    let replies = super::senders::addressed_to(conn, account, &sent, &addr)?;
    // A `uid:` key is a message with no thread: nothing else to keep it.
    let threads: Vec<String> = replies
        .iter()
        .map(|reply| reply.thread_key.clone())
        .filter(|thread| !thread.starts_with("uid:"))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let kept = kept_threads(conn, account, &scope, &threads)?;
    let uids: Vec<u32> = replies
        .into_iter()
        .filter(|reply| !kept.contains(&reply.thread_key))
        .map(|reply| reply.uid)
        .collect();
    if !uids.is_empty() {
        copies.insert(sent, uids);
    }
    Ok(copies)
}

/// A whole conversation was deleted from `folder` (call it after the deleted
/// rows left the store): tag the reader's own Sent mail in it, so it stays out
/// of the conversation from now on. Answers with the tag keys newly tagged,
/// which an Undo hands to [`bring_own_replies_back`]; empty when there was
/// nothing to tag. With `mirrored`, the tag waits to be pushed to the server
/// (see [`change_tags`]).
pub fn leave_own_replies_out(
    conn: &Connection,
    account: &str,
    folder: &str,
    thread_key: &str,
    subject: Option<&str>,
    mirrored: bool,
) -> Result<Vec<String>> {
    // A `uid:` key names one message of one folder: no replies elsewhere.
    if thread_key.is_empty() || thread_key.starts_with("uid:") {
        return Ok(Vec::new());
    }
    let copies = own_replies_of_deleted(conn, account, folder, thread_key, subject)?;
    if copies.is_empty() {
        return Ok(Vec::new());
    }
    let already = keys_with_tag(conn, account, LEFT_OUT_OF_CHAT_TAG)?;
    let keys: Vec<String> = message_tag_keys(conn, account, &copies)?
        .into_iter()
        .filter(|key| !already.contains(key))
        .collect();
    if !keys.is_empty() {
        let tag = [LEFT_OUT_OF_CHAT_TAG.to_string()];
        change_tags(conn, account, &keys, &tag, &[], mirrored)?;
    }
    Ok(keys)
}

/// Undo of [`leave_own_replies_out`]: take the tag off `keys` again.
pub fn bring_own_replies_back(
    conn: &Connection,
    account: &str,
    keys: &[String],
    mirrored: bool,
) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    let tag = [LEFT_OUT_OF_CHAT_TAG.to_string()];
    change_tags(conn, account, keys, &[], &tag, mirrored)
}

/// The replies one whole-conversation delete left out of their conversation
/// (see [`leave_own_replies_out`]), by the Trash copies an Undo of that
/// delete moves back.
struct LeftOutByDelete {
    account: String,
    trash: String,
    /// The trashed copies' UIDs, sorted.
    copies: Vec<u32>,
    /// Tag keys of the replies the delete tagged.
    keys: Vec<String>,
}

/// Deletes an Undo may still come for, oldest first. Kept in memory only: an
/// Undo is offered right after its delete, never after a restart.
static LEFT_OUT_BY_DELETE: std::sync::Mutex<std::collections::VecDeque<LeftOutByDelete>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());

/// How many deletes [`LEFT_OUT_BY_DELETE`] remembers: more than one bulk run.
const LEFT_OUT_BY_DELETE_KEPT: usize = 512;

fn sorted(uids: &[u32]) -> Vec<u32> {
    let mut uids = uids.to_vec();
    uids.sort_unstable();
    uids
}

/// A whole conversation was deleted from `folder`: leave the reader's own
/// replies to it out of the conversation from now on, and remember them for an
/// Undo that moves `trashed` (Trash folder and copies) back. Whether any reply
/// was tagged, for the caller to start the tag push.
pub fn leave_replies_out_after_delete(
    conn: &Connection,
    account: &str,
    folder: &str,
    thread_key: &str,
    subject: Option<&str>,
    trashed: Option<(&str, &[u32])>,
) -> bool {
    let keys = match leave_own_replies_out(conn, account, folder, thread_key, subject, true) {
        Ok(keys) => keys,
        // The delete itself went through; the replies just stay in the chat.
        Err(err) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.delete",
                "account={account}: leaving own replies out failed: {err:#}"
            );
            return false;
        }
    };
    if keys.is_empty() {
        return false;
    }
    if let Some((trash, copies)) = trashed {
        let mut remembered = LEFT_OUT_BY_DELETE.lock().unwrap();
        if remembered.len() >= LEFT_OUT_BY_DELETE_KEPT {
            remembered.pop_front();
        }
        remembered.push_back(LeftOutByDelete {
            account: account.to_string(),
            trash: trash.to_string(),
            copies: sorted(copies),
            keys,
        });
    }
    true
}

/// An Undo moved `copies` back out of `trash`: the replies their delete left
/// out belong to the conversation again. Whether any tag came off.
pub fn bring_replies_back_after_undo(
    conn: &Connection,
    account: &str,
    trash: &str,
    copies: &[u32],
) -> bool {
    let copies = sorted(copies);
    let entry = {
        let mut remembered = LEFT_OUT_BY_DELETE.lock().unwrap();
        let found = remembered.iter().position(|entry| {
            entry.account == account && entry.trash == trash && entry.copies == copies
        });
        found.and_then(|index| remembered.remove(index))
    };
    let Some(entry) = entry else {
        return false;
    };
    if let Err(err) = bring_own_replies_back(conn, account, &entry.keys, true) {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.move",
            "account={account}: bringing own replies back failed: {err:#}"
        );
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use rusqlite::{Connection, params};

    use super::*;
    use crate::store::{
        branch_compound_key, card_message_counts, get_thread_headers_from_folder, tags_by_key,
    };

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::store::db::run_migrations(&conn).unwrap();
        conn
    }

    /// One cached message of thread `thread`, Message-ID `<uid>@h`, sent at `uid`.
    fn add(conn: &Connection, folder: &str, uid: u32, thread: &str) {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, thread_key, json)
             VALUES('acct', ?1, ?2, ?3, 'topic', 'Bob', 'bob@x.com', ?3, 1, ?4, ?5)",
            params![folder, uid.to_string(), uid, thread, format!("{{\"message_id\":\"{uid}@h\"}}")],
        )
        .unwrap();
    }

    /// The conversation `thread` as read from `folder`, and what its card counts.
    fn read(conn: &Connection, folder: &str, thread: &str) -> (Vec<(String, u32)>, u32) {
        let shown = get_thread_headers_from_folder(conn, "acct", folder, thread)
            .unwrap()
            .into_iter()
            .map(|header| (header.folder, header.uid))
            .collect();
        let card = branch_compound_key(thread, "topic");
        let counts = card_message_counts(conn, "acct", folder, std::slice::from_ref(&card)).unwrap();
        (shown, counts.get(&card).copied().unwrap_or(0))
    }

    fn at(folder: &str, uid: u32) -> (String, u32) {
        (folder.to_string(), uid)
    }

    #[test]
    fn thrown_away_mail_shows_only_from_its_own_folder() {
        let conn = conn();
        add(&conn, "INBOX", 1, "k");
        add(&conn, "Trash", 2, "k");
        add(&conn, "Junk", 3, "k");
        add(&conn, "Sent", 4, "k");

        assert_eq!(read(&conn, "INBOX", "k"), (vec![at("INBOX", 1), at("Sent", 4)], 2));
        assert_eq!(
            read(&conn, "Trash", "k"),
            (vec![at("INBOX", 1), at("Trash", 2), at("Sent", 4)], 3)
        );
    }

    #[test]
    fn a_deleted_conversation_starts_fresh_until_the_delete_is_undone() {
        let conn = conn();
        add(&conn, "INBOX", 1, "k");
        add(&conn, "Sent", 2, "k");
        // A conversation also filed in a folder of its own is not deleted by
        // deleting its Inbox mail: its reply stays part of it.
        add(&conn, "Projects", 5, "kept");
        add(&conn, "Sent", 6, "kept");
        add(&conn, "INBOX", 7, "kept");

        // Delete both from the Inbox: the mail moves to Trash.
        let trash = |uid: u32, folder: &str| {
            conn.execute(
                "UPDATE messages SET folder = ?2 WHERE account = 'acct' AND uid = ?1",
                params![uid, folder],
            )
            .unwrap();
        };
        trash(1, "Trash");
        trash(7, "Trash");
        let left_out = leave_own_replies_out(&conn, "acct", "INBOX", "k", None, true).unwrap();
        assert_eq!(left_out, ["2@h"]);
        assert!(
            leave_own_replies_out(&conn, "acct", "INBOX", "kept", None, true)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            read(&conn, "Projects", "kept"),
            (vec![at("Projects", 5), at("Sent", 6)], 2)
        );

        // New mail in the deleted conversation: only that shows, the reply
        // stays in Sent, and the tag that hides it is not listed as a tag.
        add(&conn, "INBOX", 3, "k");
        assert_eq!(read(&conn, "INBOX", "k"), (vec![at("INBOX", 3)], 1));
        assert_eq!(read(&conn, "Sent", "k").0, [at("Sent", 2), at("INBOX", 3)]);
        assert!(tags_by_key(&conn, "acct", &left_out).unwrap().is_empty());

        // Undo: the mail is back and so is the reply.
        trash(1, "INBOX");
        bring_own_replies_back(&conn, "acct", &left_out, true).unwrap();
        assert_eq!(
            read(&conn, "INBOX", "k"),
            (vec![at("INBOX", 1), at("Sent", 2), at("INBOX", 3)], 3)
        );
    }
}
