//! Maps stored mail into the card and message shapes consumed by both hosts.
//! Folder actions and notifications reuse the same conversation identifiers.
//! Paging helpers encode the position needed to open subsequent results.

mod thread_cards;
use thread_cards::{card_rank, thread_cards_json_keyed};
pub use thread_cards::{sender_cards_json, starred_thread_cards, thread_cards_json};

use crate::imap::MessageHeader;
use crate::store;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};

pub fn canon_folder(folder: &str) -> String {
    if folder.eq_ignore_ascii_case("inbox") {
        "INBOX".to_string()
    } else {
        folder.to_string()
    }
}

/// How many of a batch's arrivals are described individually in a
/// `mail.newMessages` detail. Clients post one notification per entry, and a
/// notification shade can't usefully show more than a handful; `count` still
/// reports the true size of the batch.
pub const NEW_MESSAGES_DETAIL_MAX: usize = 8;

/// The `mail.newMessages` payload for a batch of arrivals, newest first.
///
/// `from`/`from_addr`/`subject`/`threadKey` describe the newest message and are what a
/// single-notification client (desktop) reads; `messages` carries the same fields per arrival plus
/// a body snippet, for clients that post one notification per mail.
///
/// Snippets come from cached bodies, so a message whose body hasn't been fetched yet contributes an
/// empty `preview` rather than holding up the event.
pub fn new_messages_detail(
    conn: &Connection,
    account_id: &str,
    account_name: &str,
    folder: &str,
    muted: bool,
    headers: &[MessageHeader],
) -> Option<Value> {
    let latest = headers.first()?;
    let is_inbox = folder.eq_ignore_ascii_case("INBOX");
    let messages: Vec<Value> = headers
        .iter()
        .take(NEW_MESSAGES_DETAIL_MAX)
        .map(|header| {
            json!({
                "uid": header.uid,
                "from": display_from(header),
                // The sender's exact address, lowercased: what a notification
                // tap opens in group-by-sender mode (`sender:<from_addr>`).
                "from_addr": header.from_addr.trim().to_lowercase(),
                "subject": header.subject,
                "preview": store::cached_body_preview(conn, account_id, folder, header.uid)
                    .unwrap_or_default(),
                "threadKey": store::card_thread_key(header),
                "date": header.date,
            })
        })
        .collect();
    Some(json!({
        "account": account_id,
        "accountName": account_name,
        // The mailbox the arrivals landed in: "inbox", or the wire name of a
        // folder the user opted in to notifications. `folderName` is its
        // readable label, empty for the inbox (clients name that themselves).
        "folder": if is_inbox { "inbox" } else { folder },
        "folderName": if is_inbox { String::new() } else { crate::utf7::decode(folder) },
        "count": headers.len(),
        "muted": muted,
        "from": display_from(latest),
        "from_addr": latest.from_addr.trim().to_lowercase(),
        "subject": latest.subject,
        "preview": messages
            .first()
            .and_then(|message| message.get("preview"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
        // Branch-aware card key so a notification tap opens the exact list card
        // the grouping produced.
        "threadKey": store::card_thread_key(latest),
        "messages": messages,
    }))
}

/// The `mail.newMessages` payload for new mail AI sorting filed into a Meron
/// folder with notifications on: [`new_messages_detail`] for the mailbox the
/// mail is in (so a tap and the still-unread check find it there), named after
/// the Meron folder (`folderName`), with its id in `meronFolderId`.
pub fn meron_new_messages_detail(
    conn: &Connection,
    account_id: &str,
    account_name: &str,
    muted: bool,
    arrivals: &crate::engine::ai_sort::MeronArrivals,
) -> Option<Value> {
    let mut detail = new_messages_detail(
        conn,
        account_id,
        account_name,
        &arrivals.folder,
        muted,
        &arrivals.headers,
    )?;
    detail["folderName"] = json!(arrivals.meron_name);
    detail["meronFolderId"] = json!(arrivals.meron_id);
    Some(detail)
}

/// Sender name for notifications, falling back to the address when the envelope
/// carries no display name.
pub fn display_from(header: &MessageHeader) -> String {
    if !header.from_name.trim().is_empty() {
        header.from_name.trim().to_string()
    } else {
        header.from_addr.trim().to_string()
    }
}

/// Gate for deleting a folder on the server. Both clients call this so the rule lives in one place:
/// special-use mailboxes are off limits (the app routes mail through
/// Inbox/Sent/Drafts/Trash/Junk/Archive, and the server keeps no copy once they are gone), and that
/// applies to anything nested under the folder too, since deleting it takes the whole subtree.
pub fn check_folder_deletable(
    conn: &Connection,
    account: &str,
    folder: &str,
) -> Result<(), String> {
    check_ordinary_folder_tree(conn, account, folder, "deleted")
}

/// Gate for renaming a folder on the server: the same rule as deleting, since a
/// rename moves the whole subtree and the app would lose track of a special-use
/// mailbox the server no longer reports under the name it had.
pub fn check_folder_renamable(
    conn: &Connection,
    account: &str,
    folder: &str,
) -> Result<(), String> {
    check_ordinary_folder_tree(conn, account, folder, "renamed")
}

fn check_ordinary_folder_tree(
    conn: &Connection,
    account: &str,
    folder: &str,
    verb: &str,
) -> Result<(), String> {
    if !store::folder_exists(conn, account, folder).map_err(|error| error.to_string())? {
        return Err(format!("{folder} is not in the synchronized folder list"));
    }
    let role = store::folder_role(conn, account, folder).map_err(|error| error.to_string())?;
    if role != "folder" {
        return Err(format!("{folder} is a special folder and cannot be {verb}"));
    }
    for child in store::child_folders(conn, account, folder).map_err(|error| error.to_string())? {
        let role = store::folder_role(conn, account, &child).map_err(|error| error.to_string())?;
        if role != "folder" {
            return Err(format!("{child} is a special folder and cannot be {verb}"));
        }
    }
    Ok(())
}

/// The folders a delete of `folder` removes, deepest first: IMAP DELETE fails on
/// a mailbox that still has inferiors, so the subtree has to come off leaf-end
/// first, with `folder` itself last. Sorting by descending name length puts every
/// descendant ahead of its own parent, whatever the server's delimiter is.
pub fn folder_delete_targets(
    conn: &Connection,
    account: &str,
    folder: &str,
) -> Result<Vec<String>, String> {
    let mut targets =
        store::child_folders(conn, account, folder).map_err(|error| error.to_string())?;
    targets.sort_by_key(|name| std::cmp::Reverse(name.len()));
    targets.push(folder.to_string());
    Ok(targets)
}

pub fn format_thread_id(account_id: &str, folder: &str, thread_key: &str) -> String {
    if let Some(uid) = thread_key.strip_prefix("uid:") {
        return format!("{account_id}#{}#{uid}", canon_folder(folder));
    }
    let encoded = URL_SAFE_NO_PAD.encode(thread_key.as_bytes());
    format!("{account_id}#{}#t.{encoded}", canon_folder(folder))
}

pub fn allocate_message_id(account_id: &str, draft: bool) -> String {
    let domain = account_id
        .split_once('@')
        .map(|(_, domain)| domain)
        .filter(|domain| !domain.is_empty())
        .unwrap_or("meron");
    let prefix = if draft { "meron-draft-" } else { "meron-" };
    format!("{prefix}{}@{domain}", uuid::Uuid::new_v4().simple())
}

/// Narrow cards the way the starred view does: `filter` "unread" (or, for the
/// Meron view, "starred"), the attachments toggle, and a plain text `query`
/// over the card's visible fields.
fn narrow_cards(items: &mut Vec<Value>, query: &str, filter: &str, attachments_only: bool) {
    let flag = |item: &Value, key: &str| item.get(key).and_then(Value::as_bool).unwrap_or(false);
    if filter.eq_ignore_ascii_case("unread") {
        items.retain(|item| flag(item, "unread"));
    }
    if filter.eq_ignore_ascii_case("starred") {
        items.retain(|item| flag(item, "starred"));
    }
    if attachments_only {
        items.retain(|item| flag(item, "has_attachments"));
    }
    let query = query.trim().to_lowercase();
    if !query.is_empty() {
        items.retain(|item| {
            [
                "subject",
                "preview",
                "from_name",
                "from_addr",
                "account_id",
                "folder_id",
            ]
            .iter()
            .filter_map(|field| item.get(field).and_then(Value::as_str))
            .any(|value| value.to_lowercase().contains(&query))
        });
    }
}

pub fn starred_page(
    mut items: Vec<Value>,
    query: &str,
    filter: &str,
    attachments_only: bool,
    limit: usize,
    before_cursor: Option<&str>,
) -> Value {
    // Starred cards are all starred already; only "unread" narrows them.
    let filter = if filter.eq_ignore_ascii_case("unread") {
        "unread"
    } else {
        ""
    };
    narrow_cards(&mut items, query, filter, attachments_only);
    items.sort_by(|a, b| {
        b.get("date")
            .and_then(Value::as_i64)
            .cmp(&a.get("date").and_then(Value::as_i64))
            .then_with(|| {
                b.get("id")
                    .and_then(Value::as_str)
                    .cmp(&a.get("id").and_then(Value::as_str))
            })
    });
    let start = before_cursor
        .and_then(decode_starred_cursor)
        .and_then(|(date, id)| {
            items.iter().position(|item| {
                item.get("date").and_then(Value::as_i64) == Some(date)
                    && item.get("id").and_then(Value::as_str) == Some(id.as_str())
            })
        })
        .map(|index| index + 1)
        .unwrap_or_default();
    let end = (start + limit).min(items.len());
    let page = items[start..end].to_vec();
    let next_cursor = if end < items.len() {
        page.last().and_then(|item| {
            Some(encode_starred_cursor(
                item.get("date")?.as_i64()?,
                item.get("id")?.as_str()?,
            ))
        })
    } else {
        None
    };
    let mut out = json!({ "items": page });
    if let Some(cursor) = next_cursor {
        out["next_cursor"] = Value::String(cursor);
    }
    out
}

pub fn mutation_result(
    mut result: Value,
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    source_folder: &str,
    target_folder: Option<&str>,
    unread: Option<bool>,
    starred: Option<bool>,
    removed: bool,
) -> anyhow::Result<Value> {
    let mut folders = serde_json::Map::new();
    let mut folder_counts = Vec::new();
    let mut seen_folders = std::collections::HashSet::new();
    for folder in std::iter::once(source_folder)
        .chain(target_folder)
        .filter(|folder| seen_folders.insert(*folder))
    {
        let unread = store::get_folder_unread(conn, account_id, folder)?;
        folders.insert(folder.to_string(), json!(unread));
        folder_counts
            .push(json!({ "account_id": account_id, "folder_id": folder, "unread": unread }));
    }
    result["change"] = json!({
        "thread_id": thread_id,
        "account_id": account_id,
        "source_folder": source_folder,
        "source_role": store::folder_role(conn, account_id, source_folder)?,
        "target_folder": target_folder,
        "target_role": target_folder.map(|folder| store::folder_role(conn, account_id, folder)).transpose()?,
        "unread": unread,
        "starred": starred,
        "removed": removed,
    });
    let mut folder_unreads = serde_json::Map::new();
    folder_unreads.insert(account_id.to_string(), Value::Object(folders));
    result["folder_unreads"] = Value::Object(folder_unreads);
    result["folder_counts"] = Value::Array(folder_counts);
    Ok(result)
}

/// Add the unread counts of further mailboxes a mutation touched — a
/// whole-thread flag change reaches every folder the thread spans — to a
/// [`mutation_result`], so clients refresh those badges too.
pub fn add_folder_unreads(
    result: &mut Value,
    conn: &Connection,
    account_id: &str,
    folders: impl IntoIterator<Item = impl AsRef<str>>,
) -> anyhow::Result<()> {
    for folder in folders {
        let folder = folder.as_ref();
        if result["folder_unreads"][account_id].get(folder).is_some() {
            continue;
        }
        let unread = store::get_folder_unread(conn, account_id, folder)?;
        result["folder_unreads"][account_id][folder] = json!(unread);
        if let Some(counts) = result["folder_counts"].as_array_mut() {
            counts.push(json!({ "account_id": account_id, "folder_id": folder, "unread": unread }));
        }
    }
    Ok(())
}

/// How many Meron-folder conversations one page reads at a time while a
/// filter or search may drop some of them.
const MERON_PAGE_STEP: usize = 50;

/// `meron:<date>:<entry id>` keyset cursor of the Meron folder view.
fn encode_meron_cursor(date: i64, entry: &str) -> String {
    format!("meron:{date}:{entry}")
}

fn decode_meron_cursor(cursor: &str) -> Option<(i64, String)> {
    let (date, entry) = cursor.strip_prefix("meron:")?.split_once(':')?;
    Some((date.parse().ok()?, entry.to_string()))
}

/// One page of a Meron folder: thread cards of its conversations, newest conversation first, keyset paged by (newest message date, entry).
///
/// Each conversation's card sits in its best-ranked copy (Inbox, a folder, Sent, Archive), so its
/// thread id is a real `account#FOLDER#t.…` id; Drafts, Trash and Junk copies stay out (the draft
/// badge still comes from the drafts). `filter` is "all", "unread" or "starred"; `query` a plain
/// text search.
pub fn meron_folder_page(
    conn: &Connection,
    folder_id: &str,
    query: &str,
    filter: &str,
    attachments_only: bool,
    limit: usize,
    before_cursor: Option<&str>,
) -> anyhow::Result<Value> {
    let limit = limit.max(1);
    let entries = store::meron_folder_entries(conn, folder_id)?;
    let start = match before_cursor.and_then(decode_meron_cursor) {
        Some((date, entry)) => entries
            .iter()
            .position(|it| (it.newest, it.entry_id.as_str()) < (date, entry.as_str()))
            .unwrap_or(entries.len()),
        None => 0,
    };
    let mut draft_keys_by_account: HashMap<String, HashSet<String>> = HashMap::new();
    let mut cards: Vec<Value> = Vec::new();
    let mut next = start;
    while next < entries.len() && cards.len() < limit {
        let step = if query.trim().is_empty() && filter.is_empty() && !attachments_only {
            limit - cards.len()
        } else {
            MERON_PAGE_STEP
        };
        let chunk = &entries[next..(next + step).min(entries.len())];
        next += chunk.len();
        let mut chunk_cards = Vec::new();
        for entry in chunk {
            chunk_cards.extend(meron_entry_cards(
                conn,
                folder_id,
                entry,
                &mut draft_keys_by_account,
            )?);
        }
        let filter = if filter.eq_ignore_ascii_case("all") {
            ""
        } else {
            filter
        };
        narrow_cards(&mut chunk_cards, query, filter, attachments_only);
        cards.extend(chunk_cards);
    }
    let mut out = json!({
        "threads": cards,
        "folder_unread": store::meron_unread_by_folder(conn)?
            .get(folder_id)
            .copied()
            .unwrap_or(0),
    });
    if next < entries.len()
        && let Some(last) = entries.get(next - 1)
    {
        out["next_cursor"] = Value::String(encode_meron_cursor(last.newest, &last.entry_id));
    }
    Ok(out)
}

/// The cards of one Meron-folder conversation: grouped per mailbox like the
/// starred view, then one card per thread (or subject branch) in its
/// best-ranked mailbox, newest first.
fn meron_entry_cards(
    conn: &Connection,
    folder_id: &str,
    entry: &store::MeronEntryRows,
    draft_keys_by_account: &mut HashMap<String, HashSet<String>>,
) -> anyhow::Result<Vec<Value>> {
    let account = entry.account.as_str();
    let mut by_folder: BTreeMap<String, Vec<MessageHeader>> = BTreeMap::new();
    for (folder, uid) in &entry.rows {
        if let Some(header) = store::meron_row_header(conn, account, folder, *uid)? {
            by_folder.entry(folder.clone()).or_default().push(header);
        }
    }
    let draft_thread_keys = match draft_keys_by_account.get(account) {
        Some(keys) => keys,
        None => {
            let keys = store::draft_thread_keys(conn, account)?;
            draft_keys_by_account
                .entry(account.to_string())
                .or_insert(keys)
        }
    };
    let mut cards: Vec<Value> = Vec::new();
    let mut slots: HashMap<String, usize> = HashMap::new();
    for (folder, mut headers) in by_folder {
        headers.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.uid.cmp(&a.uid)));
        for (thread_key, mut card) in
            thread_cards_json_keyed(conn, account, &folder, headers, draft_thread_keys)?
        {
            card["meron_folder_id"] = json!(folder_id);
            let identity = store::starred_thread_identity(&folder, &thread_key);
            match slots.entry(identity) {
                Entry::Occupied(slot) => {
                    let kept = &mut cards[*slot.get()];
                    if card_rank(&card) < card_rank(kept) {
                        *kept = card;
                    }
                }
                Entry::Vacant(slot) => {
                    slot.insert(cards.len());
                    cards.push(card);
                }
            }
        }
    }
    cards.sort_by(|a, b| {
        b.get("date")
            .and_then(Value::as_i64)
            .cmp(&a.get("date").and_then(Value::as_i64))
    });
    Ok(cards)
}

fn encode_starred_cursor(date: i64, id: &str) -> String {
    let value = json!({ "date": date, "id": id });
    format!(
        "starred:{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap_or_default())
    )
}

fn decode_starred_cursor(cursor: &str) -> Option<(i64, String)> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor.strip_prefix("starred:")?.as_bytes())
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    Some((
        value.get("date")?.as_i64()?,
        value.get("id")?.as_str()?.to_string(),
    ))
}

#[cfg(test)]
mod tests;
