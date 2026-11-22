//! Builds conversation cards from stored message headers.
//! The grouping queries combine roots, counts, senders, tags and attachments.
//! Mailbox reads reuse the resulting identities when opening a complete thread.

mod headers;
pub use headers::{
    get_thread_headers_all_folders, get_thread_headers_from_folder, get_thread_reference_gaps,
};

mod identity;
pub use identity::{
    branch_compound_key, card_thread_key, should_branch_thread_by_subject,
    split_branch_compound_key, split_thread_key, thread_grouping_subject,
};
pub(super) use identity::{effective_thread_key, pick_root};
use identity::{for_each_card_row, sender_name_key, short_sender_name};

use anyhow::Result;
use rusqlite::{Connection, params, params_from_iter};
use std::collections::BTreeMap;
use std::collections::HashSet;

use crate::imap::MessageHeader;

use super::cleared_chats::ChatScope;
use super::tags::{TAG_KEY_SQL, has_tags, tags_of_cards};

#[derive(Clone)]
pub struct ThreadCard {
    pub thread_key: String,
    pub original_thread_key: Option<String>,
    pub header: MessageHeader,
    pub unread_count: u32,
    /// Every message in the card's thread, read or not — the Gmail-style "3"
    /// beside the sender. Counted per card, so a subject-branched thread counts
    /// only its own branch.
    pub message_count: u32,
    pub has_draft: bool,
}

pub fn group_thread_cards(messages: Vec<MessageHeader>, default_folder: &str) -> Vec<ThreadCard> {
    group_thread_cards_with_drafts(messages, default_folder, &HashSet::new())
}

pub fn group_thread_cards_with_drafts(
    messages: Vec<MessageHeader>,
    default_folder: &str,
    draft_thread_keys: &HashSet<String>,
) -> Vec<ThreadCard> {
    use std::collections::HashMap;

    struct RootSubject {
        /// The card's title: the oldest message's subject verbatim. Reply prefixes are stripped for
        /// grouping, never for display — when the thread's own first message is not in the mailbox the
        /// oldest one here is itself a reply, and dropping its `Re:` claims a thread start that this folder
        /// does not hold.
        display: String,
        group: String,
    }

    // What one card collects. A real thread key is the same thread wherever its
    // copies sit, but a `uid:` key names a message by (folder, uid) — the fallback
    // for one with no threading headers — so on a page that spans folders (search
    // covers the mailbox plus Sent) two unrelated messages that happen to share a
    // UID carry the same key. Qualifying those with the folder keeps them apart.
    //
    // Mail shown in the Inbox from another folder (`ai_shown`) stays in a card
    // of its own folder too, apart from the same conversation's Inbox mail:
    // each card then acts (archive, delete, move) on the mail it really holds.
    let group_key = |message: &MessageHeader, key: &str| -> String {
        if message.ai_shown {
            return format!("{}\u{2}{key}", message.folder);
        }
        if !key.starts_with("uid:") {
            return key.to_string();
        }
        let folder = if message.folder.is_empty() {
            default_folder
        } else {
            message.folder.as_str()
        };
        format!("{folder}\u{1}{key}")
    };

    let mut candidates: HashMap<String, Vec<RootCandidate>> = HashMap::new();
    for message in &messages {
        if message.uid == 0 {
            continue;
        }
        let thread_key = group_key(message, &effective_thread_key(message));
        let folder = if message.folder.is_empty() {
            default_folder
        } else {
            message.folder.as_str()
        };
        candidates
            .entry(thread_key)
            .or_default()
            .push(RootCandidate {
                folder: folder.to_string(),
                uid: message.uid,
                date: message.date,
                subject: message.subject.clone(),
            });
    }
    let roots: HashMap<String, RootSubject> = candidates
        .into_iter()
        .filter_map(|(key, candidates)| {
            let root = pick_root(candidates, default_folder)?;
            Some((
                key,
                RootSubject {
                    display: root.subject.trim().to_string(),
                    group: thread_grouping_subject(&root.subject),
                },
            ))
        })
        .collect();

    let mut groups: HashMap<String, ThreadCard> = HashMap::new();
    let mut order = Vec::new();
    // One message can reach a folder-spanning page twice: search reads the
    // mailbox plus Sent, and a self-sent message is cached in both. The cached
    // tally this page's is floored against folds those copies by Message-ID (see
    // `card_message_counts`), so fold them here too, or the card claims a message
    // more than the mailbox's own list gives it. A row with no Message-ID cannot
    // be matched to a copy and counts on its own. The value is whether a copy has
    // already been counted as unread.
    let mut folded: HashMap<(String, String), bool> = HashMap::new();
    for message in messages {
        if message.uid == 0 {
            continue;
        }
        let base_key = effective_thread_key(&message);
        let branch = should_branch_thread_by_subject(&base_key);
        let group_subject = if branch {
            thread_grouping_subject(&message.subject)
        } else {
            String::new()
        };
        let compound_key = card_thread_key(&message);
        let slot = group_key(&message, &compound_key);
        let fold_key =
            (!message.message_id.is_empty()).then(|| (slot.clone(), message.message_id.clone()));
        let first_copy = fold_key
            .as_ref()
            .is_none_or(|key| !folded.contains_key(key));
        // Copies can disagree on the read flag — the mailbox's copy unread, the
        // Sent one not — and the message is unread if any of them is, whichever
        // order the page happens to put them in.
        let counts_unread = !message.seen
            && fold_key
                .as_ref()
                .is_none_or(|key| !folded.get(key).copied().unwrap_or(false));
        if let Some(key) = fold_key {
            *folded.entry(key).or_insert(false) |= !message.seen;
        }
        let card = groups.entry(slot.clone()).or_insert_with(|| {
            order.push(slot);
            let root = roots.get(&group_key(&message, &base_key));
            let mut header = message.clone();
            if header.folder.is_empty() {
                header.folder = default_folder.to_string();
            }
            header.thread_key = compound_key.clone();
            let title = root.map(|root| root.display.as_str()).unwrap_or_default();
            if !title.is_empty() {
                header.subject = title.to_string();
            }
            let original_thread_key = if branch
                && root
                    .map(|root| root.group.as_str() != group_subject.as_str())
                    .unwrap_or(false)
            {
                root.map(|root| branch_compound_key(&base_key, &root.group))
            } else {
                None
            };
            ThreadCard {
                thread_key: compound_key.clone(),
                original_thread_key,
                header,
                unread_count: 0,
                message_count: 0,
                has_draft: draft_thread_keys.contains(&base_key),
            }
        });
        // A page can span folders — search covers the mailbox plus Sent — and the
        // card above took its folder, and so its thread id, from whichever copy
        // came first (newest). Sending a reply makes the Sent copy the newest one,
        // which would mint a second id for the same thread: the list shows the
        // card twice, and every id-keyed update (the draft badge a post-send
        // discard clears, unread reconciliation) misses the row it means. The
        // searched mailbox wins whenever it holds a copy of the thread, so the id
        // is the one that mailbox's own list would give. Not for `uid:` keys:
        // those name a single message by (folder, uid), so the folder cannot move
        // without pointing at a different message.
        if !base_key.starts_with("uid:")
            && !card.header.ai_shown
            && !message.ai_shown
            && !card.header.folder.eq_ignore_ascii_case(default_folder)
            && message.folder.eq_ignore_ascii_case(default_folder)
        {
            card.header.folder = message.folder.clone();
        }
        if first_copy {
            card.message_count += 1;
        }
        if counts_unread {
            card.unread_count += 1;
        }
        if !message.seen {
            card.header.seen = false;
        }
        if message.starred {
            card.header.starred = true;
        }
    }

    order
        .into_iter()
        .filter_map(|key| groups.remove(&key))
        .collect()
}

/// Which of `card_keys` the cache holds a copy of in `folder`, as card keys.
///
/// A folder-spanning page (search reads the mailbox plus Sent) sees only its own
/// slice, so the copy that decides a card's identity — see
/// [`group_thread_cards_with_drafts`] — may have paged out below it. The cache
/// has the whole mailbox, so it answers for the ones the page cannot.
///
/// `uid:` keys are folder-scoped by construction and never move, so they are not
/// asked about.
pub fn card_keys_in_folder(
    conn: &Connection,
    account: &str,
    folder: &str,
    card_keys: &[String],
) -> Result<HashSet<String>> {
    let mut roots: Vec<String> = card_keys
        .iter()
        .map(|key| split_thread_key(key).0)
        .filter(|root| !root.starts_with("uid:"))
        .collect();
    roots.sort();
    roots.dedup();

    let mut present = HashSet::new();
    for chunk in roots.chunks(100) {
        let placeholders = (3..3 + chunk.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT thread_key, subject FROM messages
             WHERE account = ?1 AND folder = ?2 AND uid <> 0
               AND thread_key IN ({placeholders})"
        ))?;
        let mut args: Vec<&str> = vec![account, folder];
        args.extend(chunk.iter().map(String::as_str));
        let rows = stmt.query_map(params_from_iter(args), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (root, subject) = row?;
            present.insert(if should_branch_thread_by_subject(&root) {
                branch_compound_key(&root, &thread_grouping_subject(&subject))
            } else {
                root
            });
        }
    }
    Ok(present)
}

/// Total cached messages behind each of `card_keys`, keyed by card key.
///
/// The header page a list request read is not the thread: it is filtered (unread-only, starred-only, search
/// hits) and cursor-paged by *message*, so counting the headers handed to [`group_thread_cards_with_drafts`]
/// would report "1" for a long thread with a single unread message and would split a thread that straddles a
/// page boundary.
///
/// The cache holds the whole thread, so the count is taken from there and the page tally is only a floor for
/// messages the cache has not seen yet.
///
/// The scope matches what opening the row shows, so the badge never contradicts the reader (see `thread_read`):
/// real threading keys count across every folder in the account, because a received message and the user's own
/// Sent reply belong to one thread; synthetic `uid:N` keys stay inside `folder`, because UIDs are folder-scoped
/// and spanning folders would pull in an unrelated message that happens to share the UID.
///
/// Cross-folder rows are deduplicated by Message-ID exactly as [`get_thread_headers_all_folders`] does, so a
/// self-sent message cached in both Sent and Inbox counts once — the reader renders it as one bubble.
///
/// Counting re-derives each row's card key rather than grouping on the raw thread key, so a subject-branched
/// thread counts only the branch its card stands for — the same split [`card_thread_key`] makes.
pub fn card_message_counts(
    conn: &Connection,
    account: &str,
    folder: &str,
    card_keys: &[String],
) -> Result<std::collections::HashMap<String, u32>> {
    use std::collections::HashMap;

    let mut roots: Vec<String> = card_keys
        .iter()
        .map(|key| split_thread_key(key).0)
        .collect();
    roots.sort();
    roots.dedup();
    let (uid_roots, threaded_roots): (Vec<String>, Vec<String>) =
        roots.into_iter().partition(|root| root.starts_with("uid:"));

    let scope = ChatScope::for_folder(conn, account, folder)?;
    let mut counts: HashMap<String, u32> = HashMap::new();
    count_card_rows(conn, account, Some(folder), &scope, &uid_roots, &mut counts)?;
    count_card_rows(conn, account, None, &scope, &threaded_roots, &mut counts)?;
    Ok(counts)
}

/// The title each of `card_keys` should show: the subject of the oldest cached message of its thread, verbatim.
///
/// The page a card was grouped from is a filtered, cursor-paged slice — an unread view can hold a
/// reply while the seen thread opener sits in the same folder just outside the page — so a title
/// read off that slice can call a reply the thread's start. The cache has the whole mailbox and
/// settles it, the same way [`card_message_counts`] settles the tally.
///
/// Keyed by root thread key, not card key: every subject branch of a thread carries the root
/// thread's title.
pub fn card_root_subjects(
    conn: &Connection,
    account: &str,
    folder: &str,
    card_keys: &[String],
) -> Result<std::collections::HashMap<String, String>> {
    use std::collections::HashMap;

    let mut roots: Vec<String> = card_keys
        .iter()
        .map(|key| split_thread_key(key).0)
        .collect();
    roots.sort();
    roots.dedup();
    let (uid_roots, threaded_roots): (Vec<String>, Vec<String>) =
        roots.into_iter().partition(|root| root.starts_with("uid:"));

    // `uid:` roots name a message by (folder, uid) and never move; a real thread
    // key is the same thread wherever its copies sit, so its oldest message may
    // be cached in another folder.
    let scope = ChatScope::for_folder(conn, account, folder)?;
    let mut candidates: HashMap<String, Vec<RootCandidate>> = HashMap::new();
    collect_root_candidates(
        conn,
        account,
        Some(folder),
        &scope,
        &uid_roots,
        &mut candidates,
    )?;
    collect_root_candidates(
        conn,
        account,
        None,
        &scope,
        &threaded_roots,
        &mut candidates,
    )?;

    Ok(candidates
        .into_iter()
        .filter_map(|(root, candidates)| {
            let subject = pick_root(candidates, folder)?.subject.trim().to_string();
            Some((root, subject))
        })
        .collect())
}

/// Gather the cached rows of `roots` as root candidates, keyed by root thread
/// key. `folder` scopes the query to a single mailbox; `None` spans the account.
/// Mail `scope` leaves out of the conversation cannot title it.
pub(super) fn collect_root_candidates(
    conn: &Connection,
    account: &str,
    folder: Option<&str>,
    scope: &ChatScope,
    roots: &[String],
    candidates: &mut std::collections::HashMap<String, Vec<RootCandidate>>,
) -> Result<()> {
    // SQLite caps bound parameters per statement; chunk so an unpaginated caller
    // cannot overrun it (see `count_card_rows`).
    for chunk in roots.chunks(100) {
        let first = if folder.is_some() { 3 } else { 2 };
        let placeholders = (first..first + chunk.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let folder_clause = if folder.is_some() {
            "folder = ?2 AND "
        } else {
            ""
        };
        let mut stmt = conn.prepare(&format!(
            "SELECT COALESCE(NULLIF(thread_key, ''), 'uid:' || uid), folder, uid, date, subject,
                    {TAG_KEY_SQL}
             FROM messages
             WHERE account = ?1 AND {folder_clause}uid <> 0
               AND COALESCE(NULLIF(thread_key, ''), 'uid:' || uid) IN ({placeholders})"
        ))?;
        let mut args: Vec<&str> = vec![account];
        args.extend(folder);
        args.extend(chunk.iter().map(String::as_str));
        let rows = stmt.query_map(params_from_iter(args), |row| {
            Ok((
                row.get::<_, String>(0)?,
                RootCandidate {
                    folder: row.get(1)?,
                    uid: row.get(2)?,
                    date: row.get(3)?,
                    subject: row.get(4)?,
                },
                row.get::<_, String>(5)?,
            ))
        })?;
        for row in rows {
            let (root, candidate, tag_key) = row?;
            if scope.leaves_out(&candidate.folder, &tag_key) {
                continue;
            }
            candidates.entry(root).or_default().push(candidate);
        }
    }
    Ok(())
}

/// One attachment a thread card names, as the `files` column stores it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CardFile {
    pub filename: String,
    pub mime: String,
}

/// The real attachments behind each of `card_keys`, in thread order, keyed by card key; cards with
/// none are absent. Scoped and deduplicated exactly like [`card_message_counts`], so the chips
/// speak for the same messages the reader opens. Header sync supplies names from BODYSTRUCTURE; a
/// cached body refines them using the HTML references to exclude inline images.
pub fn card_attachments(
    conn: &Connection,
    account: &str,
    folder: &str,
    card_keys: &[String],
) -> Result<std::collections::HashMap<String, Vec<CardFile>>> {
    use std::collections::HashMap;

    let mut roots: Vec<String> = card_keys
        .iter()
        .map(|key| split_thread_key(key).0)
        .collect();
    roots.sort();
    roots.dedup();
    let (uid_roots, threaded_roots): (Vec<String>, Vec<String>) =
        roots.into_iter().partition(|root| root.starts_with("uid:"));

    let mut rows: HashMap<String, Vec<(i64, Vec<CardFile>)>> = HashMap::new();
    let mut collect = |key: String, row: CardRow| {
        if !row.files.is_empty() {
            rows.entry(key).or_default().push((row.date, row.files));
        }
    };
    let scope = ChatScope::for_folder(conn, account, folder)?;
    for_each_card_row(
        conn,
        account,
        Some(folder),
        &scope,
        &uid_roots,
        &mut collect,
    )?;
    for_each_card_row(conn, account, None, &scope, &threaded_roots, &mut collect)?;

    Ok(rows
        .into_iter()
        .map(|(key, mut rows)| {
            rows.sort_by_key(|(date, _)| *date);
            (key, rows.into_iter().flat_map(|(_, files)| files).collect())
        })
        .collect())
}

/// Tally the cached rows of `roots` into `counts`, one card key at a time.
/// `folder` scopes the query to a single mailbox; `None` spans the account.
pub(super) fn count_card_rows(
    conn: &Connection,
    account: &str,
    folder: Option<&str>,
    scope: &ChatScope,
    roots: &[String],
    counts: &mut std::collections::HashMap<String, u32>,
) -> Result<()> {
    for_each_card_row(conn, account, folder, scope, roots, |key, _| {
        *counts.entry(key).or_insert(0) += 1;
    })
}

/// The tags behind each of `card_keys`, sorted, keyed by card key. Scoped and
/// deduplicated like [`card_message_counts`], so a card's tags are those of the
/// messages it counts.
pub fn card_tags(
    conn: &Connection,
    account: &str,
    folder: &str,
    card_keys: &[String],
) -> Result<std::collections::HashMap<String, Vec<String>>> {
    use std::collections::HashMap;

    if !has_tags(conn, account)? {
        return Ok(HashMap::new());
    }
    let mut roots: Vec<String> = card_keys
        .iter()
        .map(|key| split_thread_key(key).0)
        .collect();
    roots.sort();
    roots.dedup();
    let (uid_roots, threaded_roots): (Vec<String>, Vec<String>) =
        roots.into_iter().partition(|root| root.starts_with("uid:"));

    let mut keys_by_card: HashMap<String, Vec<String>> = HashMap::new();
    let mut collect =
        |card: String, row: CardRow| keys_by_card.entry(card).or_default().push(row.tag_key);
    let scope = ChatScope::for_folder(conn, account, folder)?;
    for_each_card_row(
        conn,
        account,
        Some(folder),
        &scope,
        &uid_roots,
        &mut collect,
    )?;
    for_each_card_row(conn, account, None, &scope, &threaded_roots, &mut collect)?;
    tags_of_cards(conn, account, keys_by_card)
}

/// One sender of a thread card, as the list row names them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardSender {
    /// Short label: the display name's first word, else the address local part.
    /// Empty when `me` is set — the UI says "me" in its own language.
    pub name: String,
    pub me: bool,
    /// Send time of this sender's newest message in the thread, epoch seconds.
    pub last_date: i64,
}

/// The distinct senders behind each of `card_keys`, oldest first, keyed by card key — the Gmail-style "me, Dana, Bob" in place of the newest sender alone.
///
/// Scoped and deduplicated exactly like [`card_message_counts`], so the names and the count beside them
/// describe the same messages. Senders are told apart by address, and every one of the account's own
/// addresses (or anything filed in Sent, see [`super::is_outgoing`]) collapses into a single `me`.
pub fn card_senders(
    conn: &Connection,
    account: &str,
    folder: &str,
    card_keys: &[String],
) -> Result<std::collections::HashMap<String, Vec<CardSender>>> {
    use std::collections::HashMap;

    let mut roots: Vec<String> = card_keys
        .iter()
        .map(|key| split_thread_key(key).0)
        .collect();
    roots.sort();
    roots.dedup();
    let (uid_roots, threaded_roots): (Vec<String>, Vec<String>) =
        roots.into_iter().partition(|root| root.starts_with("uid:"));

    let mine = super::self_addrs(conn, account);
    let mut rows: HashMap<String, Vec<CardRow>> = HashMap::new();
    let mut collect = |key: String, row: CardRow| rows.entry(key).or_default().push(row);
    let scope = ChatScope::for_folder(conn, account, folder)?;
    for_each_card_row(
        conn,
        account,
        Some(folder),
        &scope,
        &uid_roots,
        &mut collect,
    )?;
    for_each_card_row(conn, account, None, &scope, &threaded_roots, &mut collect)?;

    Ok(rows
        .into_iter()
        .map(|(key, mut rows)| {
            rows.sort_by_key(|row| row.date);
            let me_of = |row: &CardRow| {
                let addr = row.from_addr.trim().to_lowercase();
                let me =
                    super::account_message_is_outgoing(account, &mine, &row.folder, &addr, false);
                (addr, me)
            };
            // Address alone is not a person: GitHub, Jira and mailing lists send
            // every participant's mail from one address under that participant's
            // display name, so a named sender keys on address and name. Collect
            // each address's distinct names (with one raw spelling to show) first.
            let mut named: HashMap<String, BTreeMap<String, String>> = HashMap::new();
            for row in &rows {
                let (addr, me) = me_of(row);
                if let Some(name_key) = sender_name_key(&row.from_name, &addr)
                    && !me
                {
                    named
                        .entry(addr)
                        .or_default()
                        .entry(name_key)
                        .or_insert_with(|| row.from_name.clone());
                }
            }
            // Identity → position in `senders`, so a repeat voice only moves
            // that sender's `last_date` forward.
            let mut seen: HashMap<String, usize> = HashMap::new();
            let mut senders: Vec<CardSender> = Vec::new();
            for row in rows {
                let (addr, me) = me_of(&row);
                if !me && addr.is_empty() {
                    continue;
                }
                // A message with no usable name joins its address's named sender
                // when there is exactly one — `Alice Smith <a@x>` and `<a@x>` are
                // one person. With none it keys on the address; with several (a
                // shared notification address) it cannot be told which, so it
                // stays apart rather than being credited to the wrong one.
                let (identity, display) = if me {
                    (String::new(), String::new())
                } else {
                    let names = named.get(&addr);
                    match sender_name_key(&row.from_name, &addr) {
                        Some(name_key) => (format!("{addr}\u{1}{name_key}"), row.from_name.clone()),
                        None => match names.filter(|names| names.len() == 1) {
                            Some(names) => {
                                let (name_key, raw) = names.iter().next().unwrap();
                                (format!("{addr}\u{1}{name_key}"), raw.clone())
                            }
                            None => (addr.clone(), String::new()),
                        },
                    }
                };
                if let Some(&index) = seen.get(&identity) {
                    senders[index].last_date = row.date;
                    continue;
                }
                let name = if me {
                    String::new()
                } else {
                    short_sender_name(&display, &addr)
                };
                seen.insert(identity, senders.len());
                senders.push(CardSender {
                    name,
                    me,
                    last_date: row.date,
                });
            }
            (key, senders)
        })
        .collect())
}

/// The cached fields [`for_each_card_row`] hands its visitor.
pub(super) struct CardRow {
    folder: String,
    from_name: String,
    from_addr: String,
    date: i64,
    files: Vec<CardFile>,
    files_authoritative: bool,
    tag_key: String,
}

/// A candidate for a thread's opening message: what [`pick_root`] compares.
pub(super) struct RootCandidate {
    folder: String,
    uid: u32,
    /// Send time as Unix epoch seconds, 0 when the row carries no usable Date.
    date: i64,
    subject: String,
}
