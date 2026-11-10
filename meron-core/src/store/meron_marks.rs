//! Meron-folder marks: what this device knows of the `$MeronF_<id>` keywords
//! on the server's Inbox and archive-mailbox copies (see
//! [`crate::meron_marks`]).
//!
//! `meron_marks` holds, per copy, the mark last read from the server or
//! written by this device (`folder_id` empty: none). A sync compares what it
//! read against it: only a mark that changed since then is acted on, so mail
//! that always carried its mark isn't filed again after the user took it out
//! here, and a read that started before this device's own write (which may
//! not show it yet) is ignored.
//!
//! `written_at` is a [`meron_mark_tick`], not a wall-clock second: a sync's
//! read and this device's writes must be told apart even within one second.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use super::MeronFiling;

/// The account's archive mailbox (Archive, or Gmail's All Mail), from the
/// cached folder list; never the Inbox. `None` for local and feed accounts,
/// or when it has none.
pub fn meron_archive_mailbox(conn: &Connection, account: &str) -> Option<String> {
    if super::is_local_account(account) {
        return None;
    }
    let folders: Vec<crate::imap::Folder> = conn
        .prepare_cached("SELECT name, special_use FROM folders WHERE account = ?1 ORDER BY name")
        .ok()?
        .query_map(params![account], |row| {
            Ok(crate::imap::Folder {
                name: row.get(0)?,
                special_use: row.get(1)?,
                ..Default::default()
            })
        })
        .ok()?
        .collect::<rusqlite::Result<_>>()
        .ok()?;
    crate::engine::cached_archive_folder_from_folders(folders, "INBOX")
        .filter(|name| !name.eq_ignore_ascii_case("INBOX"))
}

/// Whether a sync of `folder` reads Meron-folder marks: the Inbox and the
/// account's archive mailbox.
pub fn meron_reads_marks(conn: &Connection, account: &str, folder: &str) -> bool {
    if super::is_local_account(account) {
        return false;
    }
    folder.eq_ignore_ascii_case("INBOX")
        || meron_archive_mailbox(conn, account).as_deref() == Some(folder)
}

/// One mark-folder copy's mark as a sync read it from the server.
#[derive(Clone, Debug, PartialEq)]
pub struct MeronMarkSeen {
    pub uid: u32,
    /// The Meron folder id the copy's mark names, if it carries one.
    pub mark: Option<String>,
}

/// What reading the marks changed here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeronMarksApplied {
    /// UIDs filed into a Meron folder because of a mark, with its id.
    pub filed: Vec<(u32, String)>,
    /// Conversations taken out of a Meron folder because its mark went.
    pub released: usize,
    /// Some marks were left unread because this device wrote them after the
    /// read began: the sync must not move its CONDSTORE position past them,
    /// so the next one reads them again.
    pub skipped_stale: bool,
    /// Archive-mailbox UIDs whose mark names a folder here but which aren't
    /// cached: the sync fetches and stores them, then observes them again.
    pub uncached_marked: Vec<u32>,
}

/// A point in time for ordering mark reads and writes: microseconds, always
/// later than the last one this process handed out.
pub fn meron_mark_tick() -> i64 {
    static LAST: AtomicI64 = AtomicI64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_micros() as i64);
    let mut last = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(last + 1);
        match LAST.compare_exchange_weak(last, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(seen) => last = seen,
        }
    }
}

/// `(folder_id, written_at)` remembered for one copy.
fn remembered(conn: &Connection, account: &str, folder: &str, uid: u32) -> Result<Option<(String, i64)>> {
    Ok(conn
        .prepare_cached(
            "SELECT folder_id, written_at FROM meron_marks
              WHERE account = ?1 AND folder = ?2 AND uid = ?3",
        )?
        .query_row(params![account, folder, uid], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?)
}

fn remember(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
    folder_id: &str,
    written_at: i64,
) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO meron_marks(account, folder, uid, folder_id, written_at)
         VALUES(?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(account, folder, uid)
         DO UPDATE SET folder_id = excluded.folder_id, written_at = excluded.written_at",
    )?
    .execute(params![account, folder, uid, folder_id, written_at])?;
    Ok(())
}

/// This device wrote `target` (or took every mark away, `None`) on `uids` of
/// `folder` at `now` (a [`meron_mark_tick`]).
pub fn meron_marks_written(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
    target: Option<&str>,
    now: i64,
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for uid in uids {
        remember(&tx, account, folder, *uid, target.unwrap_or_default(), now)?;
    }
    tx.commit()?;
    Ok(())
}

/// Of `uids` in `folder`, those this device knows to carry a mark.
pub fn meron_marked_uids(conn: &Connection, account: &str, folder: &str, uids: &[u32]) -> Result<Vec<u32>> {
    let mut out = Vec::new();
    for uid in uids {
        if remembered(conn, account, folder, *uid)?.is_some_and(|(id, _)| !id.is_empty()) {
            out.push(*uid);
        }
    }
    Ok(out)
}

/// Forget the marks of copies the server no longer holds in `folder`.
pub fn meron_marks_prune(
    conn: &Connection,
    account: &str,
    folder: &str,
    server_uids: &HashSet<u32>,
) -> Result<()> {
    let known: Vec<u32> = conn
        .prepare_cached("SELECT uid FROM meron_marks WHERE account = ?1 AND folder = ?2")?
        .query_map(params![account, folder], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let gone: Vec<u32> = known.into_iter().filter(|uid| !server_uids.contains(uid)).collect();
    if gone.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    for uid in gone {
        tx.execute(
            "DELETE FROM meron_marks WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            params![account, folder, uid],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Act on the marks a sync of `folder` (the Inbox or the archive mailbox,
/// [`meron_reads_marks`]) read, its read having started at `read_at` (a
/// [`meron_mark_tick`]):
///
/// - a mark that appeared or changed files the copy's conversation into that
///   Meron folder, when the folder exists here, the mail is within
///   [`super::AI_SORT_MAX_AGE_DAYS`] and the conversation isn't there already.
///   Always filed as AI sorting files (`source = 'ai'`, old mail of other
///   folders left out): which device's AI or user did it can't be told;
/// - a mark that went away takes the conversation out of the Meron folder that
///   mark named, if that is where it is here (wherever it was filed from);
/// - a copy whose mark this device wrote after `read_at` is left alone: the
///   read may predate the write ([`MeronMarksApplied::skipped_stale`]);
/// - a mark naming a folder this device doesn't have is remembered as waiting
///   ([`WAITING_FOR_FOLDER`]); every Inbox read checks the waiting ones and
///   files them once the folder is here (a setup copy, a restore), so they
///   needn't be read from the server again;
/// - a mark gone from one copy while another mark-folder copy of the
///   conversation still carries it leaves the conversation where it is.
///
/// In the Inbox a mark naming the built-in Archive counts as no mark when
/// the account has an archive mailbox (one already known here stays, keeping
/// an earlier Meron-only filing); without one it files like any mark, unless
/// a newer Inbox copy of the conversation lacks it (see [`file_copy`]). In the
/// archive mailbox there is no age limit, filing counts as the user's, and an
/// Archive mark of a conversation that still has cached Inbox mail waits
/// ([`WAITING_FOR_INBOX`]) until it has none.
///
/// Nothing is written back to the server. Rows must be stored already.
pub fn meron_marks_observe(
    conn: &Connection,
    account: &str,
    folder: &str,
    seen: &[MeronMarkSeen],
    read_at: i64,
    now: i64,
) -> Result<MeronMarksApplied> {
    let mut out = MeronMarksApplied::default();
    if super::is_local_account(account) {
        return Ok(out);
    }
    let inbox = folder.eq_ignore_ascii_case("INBOX");
    let archive_mailbox = meron_archive_mailbox(conn, account);
    let in_archive = !inbox && archive_mailbox.as_deref() == Some(folder);
    if !inbox && !in_archive {
        return Ok(out);
    }
    // Archived mail, and the built-in Archive's own marks (on the Inbox of an
    // account with no archive mailbox, where archiving is Meron-only), are
    // the user's filing and have no age limit.
    let filing = |mark: &str| {
        if in_archive || super::meron_is_builtin(mark) {
            MeronFiling::User
        } else {
            MeronFiling::Ai { folder, now }
        }
    };
    let too_old = |date: i64, mark: &str| {
        !in_archive && !super::meron_is_builtin(mark) && super::ai_sort_too_old(date, now)
    };
    let mut filed_keys: BTreeSet<String> = BTreeSet::new();
    // The marks this read brought, for telling an Inbox copy's mark before
    // the loop below has remembered it (see [`file_copy`]).
    let seen_marks: BTreeMap<u32, &str> =
        seen.iter().map(|item| (item.uid, item.mark.as_deref().unwrap_or_default())).collect();
    // What the server says now comes first: a fresh mark replaces a waiting
    // one for the same copy, and its filing wins over the waiting ones below.
    for item in seen {
        let mut mark = item.mark.as_deref().unwrap_or_default();
        let (before, written_at) = remembered(conn, account, folder, item.uid)?.unwrap_or_default();
        // The built-in Archive's mark means nothing on an Inbox copy, unless
        // the account has no archive mailbox: archiving there is Meron-only
        // and the Inbox copies carry it. One this device already knew, from
        // before the account had an archive mailbox, stays as it is: that
        // Meron-only filing is kept, only new ones aren't made from it.
        if inbox && archive_mailbox.is_some() && super::meron_is_builtin(mark) {
            if super::meron_is_builtin(&before) {
                continue;
            }
            mark = "";
        }
        if before == mark {
            continue;
        }
        // This device wrote that copy's mark after the read began.
        if written_at > 0 && written_at >= read_at {
            out.skipped_stale = true;
            continue;
        }
        // Not cached yet: the full read that caches it brings the mark again.
        // The archive mailbox's sync caches little, so it fetches those.
        let Some(date) = cached_date(conn, account, folder, item.uid)? else {
            if in_archive && !mark.is_empty() && super::meron_folder_exists(conn, mark)? {
                out.uncached_marked.push(item.uid);
            }
            continue;
        };
        // A folder this device doesn't have (yet): waits for it (see above).
        if !mark.is_empty() && !super::meron_folder_exists(conn, mark)? {
            remember(conn, account, folder, item.uid, mark, WAITING_FOR_FOLDER)?;
            continue;
        }
        remember(conn, account, folder, item.uid, mark, 0)?;
        if too_old(date, if mark.is_empty() { &before } else { mark }) {
            continue;
        }
        if mark.is_empty() {
            // Gone: out of the Meron folder it named, if it is there and no
            // other Inbox copy of the conversation still carries that mark.
            if let Some(card) = super::meron_card_release(conn, account, folder, "", Some(item.uid))?
                && card.folder_id == before
            {
                let mut still_marked = false;
                for (other_folder, other_uid) in &card.rows {
                    let other_marks = other_folder.eq_ignore_ascii_case("INBOX")
                        || archive_mailbox.as_deref() == Some(other_folder.as_str());
                    if !other_marks || (other_folder == folder && *other_uid == item.uid) {
                        continue;
                    }
                    if remembered(conn, account, other_folder, *other_uid)?.is_some_and(|(id, _)| id == before) {
                        still_marked = true;
                        break;
                    }
                }
                if !still_marked {
                    out.released += super::meron_release_card(conn, &card)?.released;
                }
            }
            continue;
        }
        if in_archive && super::meron_is_builtin(mark) && has_inbox_copy(conn, account, folder, item.uid)? {
            remember(conn, account, folder, item.uid, mark, WAITING_FOR_INBOX)?;
            continue;
        }
        file_copy(conn, account, folder, item.uid, mark, filing(mark), &seen_marks, &mut filed_keys, &mut out)?;
    }
    // Marks that waited for their folder (still the newest this device knows
    // for their copy): filed once the folder is here, unless this read filed
    // the conversation already. Forgotten when the mail is no longer cached
    // (gone from the Inbox; a new read brings its mark again) or too old.
    let waiting: Vec<(u32, String)> = conn
        .prepare_cached(
            "SELECT uid, folder_id FROM meron_marks
              WHERE account = ?1 AND folder = ?2 AND written_at = ?3",
        )?
        .query_map(params![account, folder, WAITING_FOR_FOLDER], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (uid, mark) in waiting {
        let current = cached_date(conn, account, folder, uid)?.filter(|date| !too_old(*date, &mark));
        if current.is_none() {
            conn.prepare_cached("DELETE FROM meron_marks WHERE account = ?1 AND folder = ?2 AND uid = ?3")?
                .execute(params![account, folder, uid])?;
            continue;
        }
        if !super::meron_folder_exists(conn, &mark)? {
            continue;
        }
        if in_archive && super::meron_is_builtin(&mark) && has_inbox_copy(conn, account, folder, uid)? {
            remember(conn, account, folder, uid, &mark, WAITING_FOR_INBOX)?;
            continue;
        }
        remember(conn, account, folder, uid, &mark, 0)?;
        let key = super::meron_conversation_key(conn, account, folder, uid)?;
        if key.as_ref().is_some_and(|key| filed_keys.contains(key)) {
            continue;
        }
        file_copy(conn, account, folder, uid, &mark, filing(&mark), &seen_marks, &mut filed_keys, &mut out)?;
    }
    // Archive marks that waited for their conversation's Inbox mail to go:
    // filed once none of it is cached. Forgotten when the copy is gone.
    if in_archive {
        let waiting: Vec<(u32, String)> = conn
            .prepare_cached(
                "SELECT uid, folder_id FROM meron_marks
                  WHERE account = ?1 AND folder = ?2 AND written_at = ?3",
            )?
            .query_map(params![account, folder, WAITING_FOR_INBOX], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (uid, mark) in waiting {
            if cached_date(conn, account, folder, uid)?.is_none() {
                conn.prepare_cached("DELETE FROM meron_marks WHERE account = ?1 AND folder = ?2 AND uid = ?3")?
                    .execute(params![account, folder, uid])?;
                continue;
            }
            if has_inbox_copy(conn, account, folder, uid)? {
                continue;
            }
            remember(conn, account, folder, uid, &mark, 0)?;
            let key = super::meron_conversation_key(conn, account, folder, uid)?;
            if key.as_ref().is_some_and(|key| filed_keys.contains(key)) {
                continue;
            }
            file_copy(conn, account, folder, uid, &mark, filing(&mark), &seen_marks, &mut filed_keys, &mut out)?;
        }
    }
    Ok(out)
}

/// `written_at` of an archive-mailbox copy's built-in Archive mark whose
/// conversation still has cached Inbox mail: filed into Archive once it has
/// none (a conversation in Archive never has Inbox mail).
pub const WAITING_FOR_INBOX: i64 = -3;

/// Whether the conversation of cached copy `(folder, uid)` has a cached Inbox
/// copy.
fn has_inbox_copy(conn: &Connection, account: &str, folder: &str, uid: u32) -> Result<bool> {
    match super::meron_conversation_key(conn, account, folder, uid)? {
        Some(key) => super::meron_conversation_has_inbox_copy(conn, account, &key),
        None => Ok(false),
    }
}

/// `written_at` of a mark read for a Meron folder this device doesn't have:
/// remembered (so the same mark isn't handled twice) and filed when the
/// folder arrives.
pub const WAITING_FOR_FOLDER: i64 = -1;

/// The date of a cached message, `None` when it isn't cached.
fn cached_date(conn: &Connection, account: &str, folder: &str, uid: u32) -> Result<Option<i64>> {
    Ok(conn
        .prepare_cached("SELECT COALESCE(date, 0) FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3")?
        .query_row(params![account, folder, uid], |row| row.get(0))
        .optional()?)
}

/// File the conversation of mark-folder copy `uid` into Meron folder `mark`
/// (which exists), unless it is there already.
#[allow(clippy::too_many_arguments)]
fn file_copy(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
    mark: &str,
    filing: MeronFiling<'_>,
    seen_marks: &BTreeMap<u32, &str>,
    filed_keys: &mut BTreeSet<String>,
    out: &mut MeronMarksApplied,
) -> Result<()> {
    let Some(key) = super::meron_conversation_key(conn, account, folder, uid)? else {
        return Ok(());
    };
    // Another copy of a conversation this pass filed: announced with it.
    if filed_keys.contains(&key) {
        out.filed.push((uid, mark.to_string()));
        return Ok(());
    }
    let already = super::meron_entry_of_key(conn, account, &key)?
        .or(super::meron_entry_of_row(conn, account, folder, uid)?)
        .is_some_and(|(_, id)| id == mark);
    if already {
        return Ok(());
    }
    // A Meron-only Archive mark on Inbox copies, read together with a newer
    // Inbox reply that lacks it (a device catching up): the reply brought the
    // conversation back to the Inbox. It stays there, and the old marks go so
    // every device agrees.
    if folder.eq_ignore_ascii_case("INBOX") && super::meron_is_builtin(mark) {
        let mut marked: Vec<(u32, i64)> = Vec::new();
        let mut unmarked_newest = i64::MIN;
        for (row_uid, date) in super::meron_conversation_inbox_rows(conn, account, &key)? {
            let row_mark = match seen_marks.get(&row_uid) {
                Some(seen) => seen.to_string(),
                None => remembered(conn, account, folder, row_uid)?.map(|(id, _)| id).unwrap_or_default(),
            };
            if row_mark == mark {
                marked.push((row_uid, date));
            } else {
                unmarked_newest = unmarked_newest.max(date);
            }
        }
        let marked_newest = marked.iter().map(|(_, date)| *date).max().unwrap_or(i64::MIN);
        if unmarked_newest > marked_newest {
            let mut uids: Vec<u32> = marked.into_iter().map(|(uid, _)| uid).collect();
            uids.sort_unstable();
            crate::meron_marks::queue_archive_unmark(crate::meron_marks::MarkJob {
                account: account.to_string(),
                folder: folder.to_string(),
                uids,
                target: None,
            });
            return Ok(());
        }
    }
    super::meron_move_in(conn, account, &key, mark, filing)?;
    filed_keys.insert(key);
    out.filed.push((uid, mark.to_string()));
    Ok(())
}
