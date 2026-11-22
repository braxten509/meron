//! Builds thread and sender cards for the desktop and mobile hosts.
//! Ordinary and starred folder views share message identity and ranking rules.
//! The resulting JSON retains the source folder needed to open each conversation.

use super::*;

/// Cards for a sender-grouped page (see [`store::get_sender_page`]): one per correspondent, keyed
/// `sender:<address>`, in the ordinary card shape plus `group: "sender"` and `automated` (a
/// no-reply style address). Star and tag state stay with the single messages inside the chat, so
/// the card carries none of either beyond `starred` (any of their messages starred).
pub fn sender_cards_json(
    conn: &Connection,
    account_id: &str,
    folder_id: &str,
    groups: Vec<store::SenderGroup>,
) -> anyhow::Result<Vec<Value>> {
    let folder_role = store::folder_role(conn, account_id, folder_id)?;
    let whos: Vec<String> = groups.iter().map(|group| group.who.clone()).collect();
    let drafts = store::sender_draft_addrs(conn, account_id, &whos)?;
    let mut newest: Vec<MessageHeader> = groups.iter().map(|group| group.newest.clone()).collect();
    store::apply_card_identity(conn, account_id, folder_id, &mut newest);
    Ok(groups
        .into_iter()
        .zip(newest)
        .map(|(group, header)| {
            let thread_id =
                format_thread_id(account_id, folder_id, &store::sender_thread_key(&group.who));
            json!({
                "id": thread_id,
                "account_id": account_id,
                "folder_id": canon_folder(folder_id),
                "folder_role": folder_role,
                "thread_id": thread_id,
                "original_thread_id": Value::Null,
                "from_name": header.from_name,
                "from_addr": if header.from_addr.trim().is_empty() { group.who.clone() } else { header.from_addr.clone() },
                "to": "",
                "subject": header.subject,
                "preview": "",
                "body": "",
                "date": group.newest.date,
                "unread": group.unread_count > 0,
                "unread_count": group.unread_count,
                "message_count": group.message_count,
                "starred": group.starred,
                "tags": [],
                "has_draft": drafts.contains(&group.who),
                "has_attachments": group.has_attachments,
                "files": [],
                "recipient_overflow": 0,
                "senders": [],
                "senders_truncated": false,
                "group": "sender",
                "automated": store::is_automated_address(&group.who),
            })
        })
        .collect())
}

pub fn thread_cards_json(
    conn: &Connection,
    account_id: &str,
    folder_id: &str,
    messages: Vec<MessageHeader>,
    draft_thread_keys: &HashSet<String>,
) -> anyhow::Result<Vec<Value>> {
    Ok(
        thread_cards_json_keyed(conn, account_id, folder_id, messages, draft_thread_keys)?
            .into_iter()
            .map(|(_, card)| card)
            .collect(),
    )
}

/// [`thread_cards_json`] with each card paired to its thread key. The key is
/// folder-independent, so callers that span folders (the starred view) can tell
/// two copies of one thread apart from two different threads.
pub(super) fn thread_cards_json_keyed(
    conn: &Connection,
    account_id: &str,
    folder_id: &str,
    messages: Vec<MessageHeader>,
    draft_thread_keys: &HashSet<String>,
) -> anyhow::Result<Vec<(String, Value)>> {
    let mut cards = store::group_thread_cards_with_drafts(messages, folder_id, draft_thread_keys);

    // Grouping pins a card to the mailbox being read when the page holds a copy
    // there, so a Sent reply landing on top does not mint a second id for a
    // thread the list already shows (see `group_thread_cards_with_drafts`). The
    // page is a slice, though: the mailbox's own copy of an old thread can sit
    // below it, leaving the card on the folder its newest hit came from. The
    // cache has the whole mailbox, so it settles those.
    // Cards of mail shown in the Inbox from another folder stay in that
    // folder: they are kept apart from the Inbox's own card on purpose.
    let strays: Vec<String> = cards
        .iter()
        .filter(|card| !card.header.ai_shown)
        .filter(|card| !card.header.folder.eq_ignore_ascii_case(folder_id))
        .map(|card| card.thread_key.clone())
        .collect();
    if !strays.is_empty() {
        let here = store::card_keys_in_folder(conn, account_id, folder_id, &strays)?;
        for card in &mut cards {
            if !card.header.ai_shown && here.contains(&card.thread_key) {
                card.header.folder = folder_id.to_string();
            }
        }
    }

    // The page these cards were grouped from is a filtered, cursor-paged slice
    // of messages, so its per-card tally is not the thread size. Re-count from
    // the cached folder, bucketed by the folder each card actually sits in
    // (starred and search pages span several). The page tally stays as a floor:
    // a live IMAP page can hold messages the cache has not stored yet.
    //
    // Both lookups are keyed by (folder, key): a `uid:` key names a message by
    // (folder, uid), so on a page that spans folders `INBOX/uid:7` and
    // `Sent/uid:7` are unrelated cards under one key, and a flat map would let
    // whichever folder ran last answer for both.
    let mut counts: HashMap<(String, String), u32> = HashMap::new();
    let mut keys_by_folder: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for card in &cards {
        keys_by_folder
            .entry(card.header.folder.as_str())
            .or_default()
            .push(card.thread_key.clone());
    }
    // Same slice problem for the card's title: an unread view can hold a reply
    // while the seen message that opened the thread sits just outside the page,
    // and a title read off the slice alone would call that reply the thread's
    // start. The cache answers for the whole mailbox.
    let mut root_subjects: HashMap<(String, String), String> = HashMap::new();
    // And for who is in it: the page's newest header names one sender, but a
    // thread several people replied to should list them all.
    let mut senders: HashMap<(String, String), Vec<store::CardSender>> = HashMap::new();
    // And which files it carries, which the page's headers cannot say.
    let mut files: HashMap<(String, String), Vec<store::CardFile>> = HashMap::new();
    // Same for its tags, which any message of the thread may carry.
    let mut tags: HashMap<(String, String), Vec<String>> = HashMap::new();
    for (folder, keys) in keys_by_folder {
        tags.extend(
            store::card_tags(conn, account_id, folder, &keys)?
                .into_iter()
                .map(|(key, list)| ((folder.to_string(), key), list)),
        );
        files.extend(
            store::card_attachments(conn, account_id, folder, &keys)?
                .into_iter()
                .map(|(key, list)| ((folder.to_string(), key), list)),
        );
        senders.extend(
            store::card_senders(conn, account_id, folder, &keys)?
                .into_iter()
                .map(|(key, list)| ((folder.to_string(), key), list)),
        );
        counts.extend(
            store::card_message_counts(conn, account_id, folder, &keys)?
                .into_iter()
                .map(|(key, count)| ((folder.to_string(), key), count)),
        );
        root_subjects.extend(
            store::card_root_subjects(conn, account_id, folder, &keys)?
                .into_iter()
                .map(|(key, subject)| ((folder.to_string(), key), subject)),
        );
    }

    // Which Meron folder each card's conversation is in (search shows them
    // with an "In ‹folder›" label; the Meron view knows its own).
    let card_location = |header: &MessageHeader| {
        let folder = if header.folder.is_empty() {
            folder_id
        } else {
            header.folder.as_str()
        };
        (folder.to_string(), header.uid)
    };
    let meron_rows: Vec<(String, u32)> = cards
        .iter()
        .map(|card| card_location(&card.header))
        .collect();
    let meron_of = store::meron_folder_of_rows(conn, account_id, &meron_rows)?;

    cards
        .into_iter()
        .map(|card| {
            let folder = card.header.folder.as_str();
            let meron_folder_id = meron_of.get(&card_location(&card.header)).cloned();
            let message_count = counts
                .get(&(folder.to_string(), card.thread_key.clone()))
                .copied()
                .unwrap_or(0)
                .max(card.message_count);
            // Cached rows win when the thread has any: every listed message is
            // cached, so the cache is the fuller view, and the page's own title
            // only stands in for a thread the cache has not stored yet.
            let subject = root_subjects
                .get(&(
                    folder.to_string(),
                    store::split_thread_key(&card.thread_key).0,
                ))
                .cloned()
                .unwrap_or_else(|| card.header.subject.clone());
            // Only a thread with more than one sender gets the list; a single
            // correspondent keeps the card identity (which names the recipient
            // on an outbound-only thread rather than "me").
            //
            // Past three, Gmail-style: the opener, then a gap, then the two who
            // wrote most recently (by their newest message, not their first, so
            // an early voice who sent the latest reply is not the one elided),
            // in the order of those messages. `senders_truncated` marks the gap
            // after the first entry.
            let list = senders
                .get(&(folder.to_string(), card.thread_key.clone()))
                .filter(|list| list.len() > 1)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let senders_truncated = list.len() > 3;
            let shown: Vec<&store::CardSender> = if senders_truncated {
                let mut rest: Vec<&store::CardSender> = list[1..].iter().collect();
                // Stable, so senders tied on date keep their first-seen order.
                rest.sort_by_key(|sender| sender.last_date);
                let mut shown = vec![&list[0]];
                shown.extend_from_slice(&rest[rest.len() - 2..]);
                shown
            } else {
                list.iter().collect()
            };
            let senders: Vec<Value> = shown
                .into_iter()
                .map(|sender| json!({ "name": sender.name, "me": sender.me }))
                .collect();
            let files = files
                .get(&(folder.to_string(), card.thread_key.clone()))
                .map(Vec::as_slice)
                .unwrap_or_default();
            let tags = tags
                .get(&(folder.to_string(), card.thread_key.clone()))
                .map(Vec::as_slice)
                .unwrap_or_default();
            let folder_role = store::folder_role(conn, account_id, folder)?;
            let thread_id = format_thread_id(account_id, folder, &card.thread_key);
            let original_thread_id = card
                .original_thread_key
                .as_deref()
                .map(|key| format_thread_id(account_id, folder, key));
            Ok((
                card.thread_key.clone(),
                json!({
                "id": thread_id,
                "account_id": account_id,
                "folder_id": folder,
                "folder_role": folder_role,
                "thread_id": thread_id,
                "original_thread_id": original_thread_id,
                "from_name": card.header.from_name,
                "from_addr": card.header.from_addr,
                "to": "",
                "subject": subject,
                "preview": "",
                "body": "",
                "date": card.header.date,
                "unread": card.unread_count > 0,
                "unread_count": card.unread_count,
                "message_count": message_count,
                "starred": card.header.starred,
                "tags": tags,
                "has_draft": card.has_draft,
                "has_attachments": !files.is_empty(),
                "files": files,
                "recipient_overflow": card.header.recipient_overflow,
                "senders": senders,
                "senders_truncated": senders_truncated,
                "meron_folder_id": meron_folder_id,
                "shown_in_inbox": card.header.ai_shown,
                }),
            ))
        })
        .collect()
}

/// Every starred mail message across all accounts, as ordinary thread cards.
///
/// The starred view used to emit one row per starred *message* (`id` was `{thread_id}#{uid}`),
/// which made it the only list in the app whose rows were not threads — hence its own row menu, its
/// own bulk-selection surface and a `selectedStarredItem` selection model running alongside the
/// normal one.
///
/// Building the same cards every other mailbox builds collapses all of that: star is just another
/// folder whose contents happen to span accounts.
///
/// Cards are grouped per (account, folder) rather than in one pass, because
/// [`store::group_thread_cards_with_drafts`] keys on the thread key alone. A single pass would
/// merge a thread starred in both Inbox and Archive into one card and pin it to whichever folder
/// happened to sort first.
///
/// The per-folder pass is then collapsed back to one card per thread by [`starred_folder_rank`], so
/// a thread the server files under several folders is listed once, in its most specific one.
///
/// `max_threads` bounds the conversations read, not the rows: the cards are what [`starred_page`]
/// then searches, sorts and pages through, so a budget spent on duplicate folder copies would cut
/// the list short with no cursor left to reach what it dropped.
pub fn starred_thread_cards(conn: &Connection, max_threads: u32) -> anyhow::Result<Vec<Value>> {
    let mut by_location: BTreeMap<(String, String), Vec<MessageHeader>> = BTreeMap::new();
    // The conversations the budget let through. Expanding a starred hit below
    // re-reads its whole root thread, which can carry subject branches the
    // budget stopped short of; those are a later page's rows, not this set's.
    let mut admitted: HashSet<(String, String)> = HashSet::new();
    for (account, header) in store::get_starred_all_accounts(conn, max_threads)? {
        let folder = if header.folder.is_empty() {
            "INBOX".to_string()
        } else {
            header.folder.clone()
        };
        admitted.insert((
            account.clone(),
            store::starred_thread_identity(&folder, &store::card_thread_key(&header)),
        ));
        by_location
            .entry((account, folder))
            .or_default()
            .push(header);
    }

    let mut draft_keys_by_account: HashMap<String, HashSet<String>> = HashMap::new();
    let mut cards: Vec<Value> = Vec::new();
    // Where each (account, thread key) already landed, so a second folder
    // holding the same thread replaces that card instead of adding a row.
    let mut card_slots: HashMap<(String, String), usize> = HashMap::new();
    for ((account, folder), starred_headers) in by_location {
        // A starred hit admits its whole thread into the view. Build the card
        // from every cached message in that thread so its date, unread count and
        // sender match the ordinary mailbox card rather than the particular
        // starred message that happened to admit it.
        let mut headers = Vec::new();
        let mut seen_thread_keys = HashSet::new();
        for header in starred_headers {
            // Already the `uid:<uid>` fallback when the message carried no
            // threading headers — the cache read fills that in.
            let thread_key = header.thread_key;
            if seen_thread_keys.insert(thread_key.clone()) {
                headers.extend(store::get_thread_headers(
                    conn,
                    &account,
                    &folder,
                    &thread_key,
                )?);
            }
        }
        for header in &mut headers {
            header.folder = folder.clone();
        }
        headers.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.uid.cmp(&a.uid)));
        let draft_thread_keys = match draft_keys_by_account.get(&account) {
            Some(keys) => keys,
            None => {
                let keys = store::draft_thread_keys(conn, &account)?;
                draft_keys_by_account.entry(account.clone()).or_insert(keys)
            }
        };
        for (thread_key, card) in
            thread_cards_json_keyed(conn, &account, &folder, headers, draft_thread_keys)?
        {
            // Branching can split one root thread by subject. Only branches that
            // actually contain a starred message belong in the starred view.
            if !card["starred"].as_bool().unwrap_or(false) {
                continue;
            }
            let identity = (
                account.clone(),
                store::starred_thread_identity(&folder, &thread_key),
            );
            if !admitted.contains(&identity) {
                continue;
            }
            match card_slots.entry(identity) {
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
    Ok(cards)
}

pub(super) fn card_rank(card: &Value) -> u8 {
    starred_folder_rank(card["folder_role"].as_str().unwrap_or("folder"))
}

/// Choose the folder shown when several mailboxes cache the same starred conversation.
///
/// The lowest rank wins. Prefer the Inbox and ordinary mailboxes over catch-all copies such as
/// Gmail's All Mail, so the starred list has a single row that opens in the expected folder.
fn starred_folder_rank(role: &str) -> u8 {
    match role {
        "inbox" => 0,
        "folder" => 1,
        "sent" => 2,
        "drafts" => 3,
        "archive" => 4,
        "junk" => 5,
        "trash" => 6,
        _ => 7,
    }
}
