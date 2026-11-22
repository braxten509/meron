//! Derives the stable keys and subject forms used when grouping threads.
//! Card assembly shares sender normalization and root selection across folders.
//! Reply counters and leading subject tags are normalized when branching a conversation.

use super::*;

/// The comparison key for a sender's display name, or None when the name says
/// nothing over the address (missing, or one that repeats the address).
pub(super) fn sender_name_key(name: &str, addr_lower: &str) -> Option<String> {
    let name = name.trim().trim_matches('"').trim().to_lowercase();
    (!name.is_empty() && !name.contains(addr_lower)).then_some(name)
}

/// "Dana Evans" → "Dana"; a missing or junk name (one that repeats the
/// address) falls back to the address local part.
pub(super) fn short_sender_name(name: &str, addr_lower: &str) -> String {
    let name = name.trim().trim_matches('"').trim();
    if name.is_empty() || name.to_lowercase().contains(addr_lower) {
        return addr_lower.split('@').next().unwrap_or_default().to_string();
    }
    name.split_whitespace()
        .next()
        .unwrap_or(name)
        .trim_end_matches(',')
        .to_string()
}

/// Visit every cached message behind `roots` once, with the card key it belongs to.
///
/// `folder` scopes the scan (for `uid:` roots); Message-ID copies across folders are folded, so a
/// self-sent message cached in Inbox and Sent is one visit. Mail `scope` leaves out of the
/// conversation is not visited, so every tally agrees with what opening the card shows.
///
/// The first copy supplies identity. Attachment metadata prefers a cached body over header-only
/// copies, even when its refined list is empty.
pub(super) fn for_each_card_row(
    conn: &Connection,
    account: &str,
    folder: Option<&str>,
    scope: &ChatScope,
    roots: &[String],
    mut visit: impl FnMut(String, CardRow),
) -> Result<()> {
    use std::collections::{HashMap, hash_map::Entry};

    // Message-ID duplicates are folded in Rust rather than with the correlated
    // NOT EXISTS that reading a single thread uses: with a page of keys and no
    // index on thread_key, that subquery would re-scan the account's messages
    // once per row, where one pass plus a set of seen ids is linear.
    let mut seen_ids: HashMap<(String, String), usize> = HashMap::new();
    let mut visits: Vec<(String, CardRow)> = Vec::new();
    // SQLite caps bound parameters per statement; a page of cards stays well
    // under it, but chunk anyway so an unpaginated caller cannot overrun it.
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
            "SELECT COALESCE(NULLIF(thread_key, ''), 'uid:' || uid), subject,
                    COALESCE(json_extract(json, '$.message_id'), ''),
                    folder, COALESCE(from_name, ''), COALESCE(from_addr, ''), date, files, body IS NOT NULL,
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
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                CardRow {
                    folder: row.get(3)?,
                    from_name: row.get(4)?,
                    from_addr: row.get(5)?,
                    date: row.get(6)?,
                    files_authoritative: row.get(8)?,
                    tag_key: row.get(9)?,
                    files: row
                        .get::<_, Option<String>>(7)?
                        .and_then(|files| serde_json::from_str(&files).ok())
                        .unwrap_or_default(),
                },
            ))
        })?;
        for row in rows {
            let (root, subject, message_id, card_row) = row?;
            if scope.leaves_out(&card_row.folder, &card_row.tag_key) {
                continue;
            }
            let key = if should_branch_thread_by_subject(&root) {
                branch_compound_key(&root, &thread_grouping_subject(&subject))
            } else {
                root
            };
            // A row with no Message-ID cannot be matched to a copy, so it counts
            // on its own — same call the thread reader makes.
            if !message_id.is_empty() {
                match seen_ids.entry((key.clone(), message_id)) {
                    Entry::Occupied(kept) => {
                        let kept = &mut visits[*kept.get()].1;
                        if (card_row.files_authoritative && !kept.files_authoritative)
                            || (!kept.files_authoritative && kept.files.is_empty())
                        {
                            kept.files = card_row.files;
                            kept.files_authoritative = card_row.files_authoritative;
                        }
                        continue;
                    }
                    Entry::Vacant(slot) => {
                        slot.insert(visits.len());
                    }
                }
            }
            visits.push((key, card_row));
        }
    }
    for (key, card_row) in visits {
        visit(key, card_row);
    }
    Ok(())
}

/// Choose the oldest message of one thread from `candidates`.
///
/// `read_folder` is the mailbox the card belongs to.
///
/// UIDs ascend with arrival inside a folder, so there the lowest UID is the oldest message —
/// authoritative even for a row whose Date header is missing. Across folders UIDs are unrelated: on
/// a page that merges the mailbox with Sent (search, and any thread the user replied to) a Sent
/// reply's UID 1 is not older than an Inbox root's UID 100.
///
/// So each folder's own pick is made by UID first, and only those are compared, by send time alone
/// — UID says nothing here and must not break the tie, or the Sent reply wins back whenever the two
/// share a timestamp or neither carries one.
///
/// What breaks a cross-folder tie is the mailbox being read: it is the folder the card sits in, so
/// its copy is the one that should title it. The folder name settles what is left, so the pick
/// never follows input order.
pub(in crate::store) fn pick_root(
    candidates: Vec<RootCandidate>,
    read_folder: &str,
) -> Option<RootCandidate> {
    let mut per_folder: BTreeMap<String, RootCandidate> = BTreeMap::new();
    for candidate in candidates {
        match per_folder.get(&candidate.folder) {
            Some(best) if best.uid <= candidate.uid => {}
            _ => {
                per_folder.insert(candidate.folder.clone(), candidate);
            }
        }
    }
    per_folder.into_values().min_by(|left, right| {
        fn rank<'a>(candidate: &'a RootCandidate, read_folder: &str) -> (i64, bool, &'a str) {
            (
                // An undated pick loses to a dated one.
                if candidate.date == 0 {
                    i64::MAX
                } else {
                    candidate.date
                },
                !candidate.folder.eq_ignore_ascii_case(read_folder),
                candidate.folder.as_str(),
            )
        }
        rank(left, read_folder).cmp(&rank(right, read_folder))
    })
}

pub(in crate::store) fn effective_thread_key(message: &MessageHeader) -> String {
    if message.thread_key.is_empty() {
        format!("uid:{}", message.uid)
    } else {
        message.thread_key.clone()
    }
}

/// The branch-aware thread key a list card for `message` carries: the root thread key joined with
/// the message's grouping subject for branchable threads, or the bare root for uid:/gmthrid: keys.
/// Every path that mints a clickable thread id (thread lists, starred items, new-mail
/// notifications) must use this so the id matches the card the grouping produced.
pub fn card_thread_key(message: &MessageHeader) -> String {
    let base = effective_thread_key(message);
    if should_branch_thread_by_subject(&base) {
        branch_compound_key(&base, &thread_grouping_subject(&message.subject))
    } else {
        base
    }
}

/// Split a `thread_key` request parameter into (root key, branch subject
/// filter). Card-minted keys for branchable threads always carry the
/// `#subject` suffix (see [`card_thread_key`]); uid:/gmthrid: keys never do.
pub fn split_thread_key(thread_key: &str) -> (String, Option<String>) {
    if should_branch_thread_by_subject(thread_key) {
        split_branch_compound_key(thread_key)
    } else {
        (thread_key.to_string(), None)
    }
}

pub fn should_branch_thread_by_subject(thread_key: &str) -> bool {
    !thread_key.starts_with("uid:")
        && !thread_key.starts_with("gmthrid:")
        && !thread_key.starts_with(crate::store::SENDER_KEY_PREFIX)
}

/// Join a root thread key and a grouping subject into one branch key. The root is a raw Message-ID,
/// where `#` is legal atext — escape it so [`split_branch_compound_key`] can split at the first
/// literal `#` unambiguously (the subject side stays verbatim; it is only ever compared whole
/// against other grouping subjects).
pub fn branch_compound_key(root: &str, group_subject: &str) -> String {
    let escaped = root.replace('%', "%25").replace('#', "%23");
    format!("{escaped}#{group_subject}")
}

/// Split a branch key built by [`branch_compound_key`] back into
/// (root thread key, grouping subject). Keys without a `#` separator were
/// never escaped (unbranched legacy ids) and come back verbatim with no
/// subject.
pub fn split_branch_compound_key(compound: &str) -> (String, Option<String>) {
    match compound.split_once('#') {
        Some((root, subject)) => (
            root.replace("%23", "#").replace("%25", "%"),
            Some(subject.to_string()),
        ),
        None => (compound.to_string(), None),
    }
}

pub fn thread_grouping_subject(subject: &str) -> String {
    let mut subject = subject.trim();
    loop {
        if let Some(rest) = strip_reply_prefix(subject) {
            subject = rest.trim();
            continue;
        }
        if let Some(rest) = strip_leading_bracket_tag(subject) {
            subject = rest.trim();
            continue;
        }
        break;
    }
    subject.to_string()
}

fn strip_reply_prefix(subject: &str) -> Option<&str> {
    let mut probe = subject.trim_start();
    while let Some(rest) = strip_leading_bracket_tag(probe) {
        probe = rest.trim_start();
    }

    const PREFIXES: &[&str] = &[
        "re", "fw", "fwd", "aw", "sv", "vs", "rv", "res", "tr", "antw", "wg", "答复", "回复",
        "转发",
    ];
    // Try every prefix that matches, not just the first: "fw" is a string
    // prefix of "Fwd:" but fails the colon check, and only the "fwd" entry
    // succeeds (mirrors the Go regex alternation, where the engine picks the
    // alternative that lets the trailing colon match).
    for prefix in PREFIXES {
        if !probe.is_char_boundary(prefix.len())
            || !probe[..prefix.len()].eq_ignore_ascii_case(prefix)
        {
            continue;
        }
        let mut rest = &probe[prefix.len()..];
        if let Some(after_count) = strip_reply_count(rest) {
            rest = after_count;
        }
        let mut chars = rest.chars();
        match chars.next() {
            Some(':') | Some('：') => return Some(chars.as_str()),
            _ => continue,
        }
    }
    None
}

fn strip_reply_count(rest: &str) -> Option<&str> {
    let bytes = rest.as_bytes();
    let close = match bytes.first()? {
        b'[' => b']',
        b'(' => b')',
        _ => return None,
    };
    let end = bytes.iter().position(|byte| *byte == close)?;
    if end <= 1 || !bytes[1..end].iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(&rest[end + 1..])
}

fn strip_leading_bracket_tag(subject: &str) -> Option<&str> {
    let trimmed = subject.trim_start();
    if !trimmed.starts_with('[') {
        return None;
    }
    let end = trimmed.find(']')?;
    Some(&trimmed[end + 1..])
}
