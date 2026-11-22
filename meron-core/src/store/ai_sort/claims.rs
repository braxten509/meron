//! Tracks the daily sorting budget and claims work for pending conversations.
//! The sync engine reserves, groups or releases candidate UIDs through these queries.
//! Stored decisions and queued rule actions prevent repeated work on a message.

use super::*;

fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// How many more emails automatic sorting may send today.
pub fn ai_sort_cap_left(conn: &Connection) -> Result<i64> {
    let used: i64 = conn
        .query_row(
            "SELECT count FROM ai_sort_usage WHERE day = ?1",
            params![today()],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    Ok((AI_SORT_DAILY_CAP - used).max(0))
}

fn add_usage(conn: &Connection, count: usize) -> Result<()> {
    if count == 0 {
        return Ok(());
    }
    let day = today();
    conn.execute("DELETE FROM ai_sort_usage WHERE day <> ?1", params![day])?;
    conn.execute(
        "INSERT INTO ai_sort_usage(day, count) VALUES(?1, ?2)
         ON CONFLICT(day) DO UPDATE SET count = count + excluded.count",
        params![day, count as i64],
    )?;
    Ok(())
}

pub(super) fn is_done(conn: &Connection, account: &str, identity: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM ai_sort_done WHERE account = ?1 AND identity = ?2",
            params![account, identity],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Hold back the new Inbox mail the AI is to sort, before the sync stores it:
/// what lacks the `$MeronSorted` mark and isn't already decided on this device,
/// up to the daily cap. Writes the hidden rows and answers with the emails. Mail
/// over the cap stays in the Inbox, and the log says why.
pub fn ai_sort_hold(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    headers: &[MessageHeader],
    now: i64,
) -> Result<Vec<AiSortCandidate>> {
    if !folder.eq_ignore_ascii_case("INBOX") || headers.is_empty() {
        return Ok(Vec::new());
    }
    hold(conn, account, folder, uidvalidity, headers, now)
}

/// Hold newly arrived mail in a watched folder before storing its headers.
///
/// The first scan sets the watch at `uid_next`. Later scans consider unseen messages at or above
/// that position, skipping copies already in the Inbox or under review in another folder.
/// Messages over the cap appear in the Inbox.
///
/// The half-open UID range `read` covers messages already cached or present in `headers`.
/// Advance the watch only when this range covers its current position, so a later sync can
/// process mail outside the current window. See [`crate::store::ai_sort_watches_folder`].
#[allow(clippy::too_many_arguments)]
pub fn ai_sort_hold_folder(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    uid_next: u32,
    read: (u32, u32),
    headers: &[MessageHeader],
    now: i64,
) -> Result<Vec<AiSortCandidate>> {
    if folder.eq_ignore_ascii_case("INBOX") {
        return Ok(Vec::new());
    }
    let Some(from) =
        crate::store::ai_sort_watch_from(conn, account, folder, uidvalidity, uid_next)?
    else {
        return Ok(Vec::new());
    };
    let mut fresh = Vec::new();
    for header in crate::store::first_seen_messages(conn, account, folder, from, uid_next, headers)?
    {
        if !crate::store::ai_sort_inbox::ai_sort_seen_in_inbox(conn, account, &header)? {
            fresh.push(header);
        }
    }
    let (read_from, read_to) = read;
    if read_from <= from {
        crate::store::ai_sort_inbox::ai_sort_watch_advance(
            conn,
            account,
            folder,
            uidvalidity,
            read_to,
        )?;
    }
    if fresh.is_empty() {
        return Ok(Vec::new());
    }
    hold(conn, account, folder, uidvalidity, &fresh, now)
}

/// The Message-ID as the log and the copies compare it.
fn bare_message_id(message_id: &str) -> String {
    message_id
        .trim()
        .trim_matches(|c| c == ' ' || c == '<' || c == '>')
        .to_lowercase()
}

/// Whether a copy of this message (same Message-ID) in a folder other than
/// `folder` was decided on by the AI, or is held for it right now. Gmail shows
/// one message in INBOX and in each of its labels; whichever copy came first is
/// the one sorted (and paid for), once.
fn sorted_elsewhere(
    conn: &Connection,
    account: &str,
    folder: &str,
    message_id: &str,
) -> Result<bool> {
    let id = bare_message_id(message_id);
    if id.is_empty() {
        return Ok(false);
    }
    let decided = conn
        .prepare_cached(
            "SELECT 1 FROM ai_sort_log
              WHERE account = ?1 AND from_folder NOT IN ('', ?2)
                AND lower(trim(message_id, ' <>')) = ?3
              LIMIT 1",
        )?
        .exists(params![account, folder, id])?;
    if decided {
        return Ok(true);
    }
    Ok(conn
        .prepare_cached(
            "SELECT 1 FROM ai_sort_hidden h
               JOIN messages m ON m.account = h.account AND m.folder = h.folder AND m.uid = h.uid
              WHERE h.account = ?1 AND h.folder <> ?2
                AND h.hide_until > CAST(strftime('%s','now') AS INTEGER)
                AND lower(trim(COALESCE(json_extract(m.json, '$.message_id'), ''), ' <>')) = ?3
              LIMIT 1",
        )?
        .exists(params![account, folder, id])?)
}

/// Queue the move of a message's Inbox copy (Gmail) to `target`, under log row
/// `log_id`: the decision its copy in another folder got, reused without asking
/// the AI again. Hidden while the move runs, as the AI's own moves are. False
/// when the Inbox has no cached copy or rule work is queued for it.
#[allow(clippy::too_many_arguments)]
pub(super) fn queue_inbox_copy_move(
    conn: &Connection,
    account: &str,
    validity: u32,
    inbox_uid: u32,
    log_id: i64,
    target: &str,
    now: i64,
) -> Result<bool> {
    let folder = crate::store::EXISTING_RULE_FOLDER;
    let ruled = crate::store::pending_rule_actions(conn, account, folder)?
        .iter()
        .any(|action| action.uid == inbox_uid);
    if ruled {
        return Ok(false);
    }
    let action = RuleAction {
        folder: folder.to_string(),
        uid: inbox_uid,
        uidvalidity: validity,
        target: target.to_string(),
        rule_ids: vec![format!("{AI_RULE_ID}:{log_id}")],
        run: 0,
        ..RuleAction::default()
    };
    crate::store::rules::queue_rule_actions(
        conn,
        account,
        &BTreeMap::from([(inbox_uid, action)]),
        0,
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO ai_sort_hidden(account, folder, uid, uidvalidity, hide_until)
         VALUES(?1, ?2, ?3, ?4, ?5)",
        params![
            account,
            folder,
            inbox_uid,
            validity,
            now + AI_SORT_MOVE_HOLD_SECS
        ],
    )?;
    Ok(true)
}

/// New Inbox mail whose copy in another folder (Gmail) the AI already decided
/// to move to a server folder, which mail outside the Inbox never is: the
/// Inbox copy gets that move now, from the same log row, without a second AI
/// request. Before the sync stores `headers`. Answers with the UIDs queued.
pub fn ai_sort_reuse_for_inbox(
    conn: &Connection,
    account: &str,
    uidvalidity: u32,
    headers: &[MessageHeader],
    now: i64,
) -> Result<Vec<u32>> {
    let waiting = conn
        .prepare_cached(
            "SELECT 1 FROM ai_sort_log WHERE account = ?1 AND server_target <> '' LIMIT 1",
        )?
        .exists(params![account])?;
    if !waiting {
        return Ok(Vec::new());
    }
    let mut queued = Vec::new();
    for header in headers {
        let id = bare_message_id(&header.message_id);
        if header.ai_sorted || header.uid == 0 || id.is_empty() || ai_sort_too_old(header.date, now)
        {
            continue;
        }
        let found: Option<(i64, String)> = conn
            .query_row(
                "SELECT id, server_target FROM ai_sort_log
                  WHERE account = ?1 AND server_target <> '' AND state = 'left'
                    AND lower(trim(message_id, ' <>')) = ?2
                  ORDER BY id DESC LIMIT 1",
                params![account, id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((log_id, target)) = found else {
            continue;
        };
        if queue_inbox_copy_move(conn, account, uidvalidity, header.uid, log_id, &target, now)? {
            conn.execute(
                "UPDATE ai_sort_log SET state = 'moving', to_folder = ?2, detail = '',
                        server_target = ''
                  WHERE id = ?1",
                params![log_id, target],
            )?;
            queued.push(header.uid);
        }
    }
    Ok(queued)
}

fn hold(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    headers: &[MessageHeader],
    now: i64,
) -> Result<Vec<AiSortCandidate>> {
    conn.execute(
        "DELETE FROM ai_sort_hidden WHERE hide_until <= ?1",
        params![now],
    )?;
    let mut left = ai_sort_cap_left(conn)?;
    let mut held = Vec::new();
    let mut over_cap = Vec::new();
    for header in headers {
        if header.ai_sorted || header.uid == 0 {
            continue;
        }
        // Old mail stays where it is: not held, not sent, not shown in the
        // Inbox from another folder.
        if ai_sort_too_old(header.date, now) {
            continue;
        }
        // Already in a Meron folder: never sorted again, and not charged.
        if crate::store::meron_header_in_folder(conn, account, header)? {
            continue;
        }
        // A copy in another folder (Gmail) was or is being sorted: its decision
        // stands for this one too (see `ai_sort_reuse_for_inbox`), not charged.
        if sorted_elsewhere(conn, account, folder, &header.message_id)? {
            continue;
        }
        let candidate = AiSortCandidate::from_header(header, folder);
        if is_done(conn, account, &candidate.identity)? {
            continue;
        }
        if left <= 0 {
            over_cap.push(candidate);
            continue;
        }
        left -= 1;
        conn.execute(
            "INSERT OR REPLACE INTO ai_sort_hidden(account, folder, uid, uidvalidity, hide_until)
             VALUES(?1, ?2, ?3, ?4, ?5)",
            params![
                account,
                folder,
                candidate.uid,
                uidvalidity,
                now + AI_SORT_HOLD_SECS
            ],
        )?;
        held.push(candidate);
    }
    add_usage(conn, held.len())?;
    for candidate in &over_cap {
        let detail = format!("the daily limit of {AI_SORT_DAILY_CAP} emails was reached");
        insert_log(
            conn,
            account,
            folder,
            uidvalidity,
            candidate,
            "",
            "",
            "left",
            &detail,
        )?;
    }
    if !over_cap.is_empty() {
        // Left by the AI: mail from another folder shows in the Inbox.
        crate::store::ai_sort_show_in_inbox(conn, account, folder, uidvalidity, &over_cap)?;
        trim_log(conn)?;
    }
    Ok(held)
}

/// Leave to the user's own rules what they already have work queued for: rules
/// always win, and their queue entry must not be replaced. Their hidden rows go.
pub fn ai_sort_skip_rule_actions(
    conn: &Connection,
    account: &str,
    folder: &str,
    candidates: Vec<AiSortCandidate>,
) -> Result<Vec<AiSortCandidate>> {
    if candidates.is_empty() {
        return Ok(candidates);
    }
    let pending: BTreeSet<u32> = crate::store::pending_rule_actions(conn, account, folder)?
        .into_iter()
        .map(|action| action.uid)
        .collect();
    let (ruled, kept): (Vec<_>, Vec<_>) = candidates
        .into_iter()
        .partition(|candidate| pending.contains(&candidate.uid));
    let uids: Vec<u32> = ruled.iter().map(|candidate| candidate.uid).collect();
    ai_sort_release(conn, account, folder, &uids)?;
    Ok(kept)
}

/// Leave out what is in a Meron folder by now (sync adopted it into one): a
/// conversation there is never sorted again. Its hidden rows go and its share
/// of today's cap is given back.
pub fn ai_sort_skip_meron_foldered(
    conn: &Connection,
    account: &str,
    folder: &str,
    candidates: Vec<AiSortCandidate>,
) -> Result<Vec<AiSortCandidate>> {
    if candidates.is_empty() {
        return Ok(candidates);
    }
    let uids: Vec<u32> = candidates.iter().map(|candidate| candidate.uid).collect();
    let foldered = crate::store::meron_foldered_uids(conn, account, folder, &uids)?;
    if foldered.is_empty() {
        return Ok(candidates);
    }
    let (filed, kept): (Vec<_>, Vec<_>) = candidates
        .into_iter()
        .partition(|candidate| foldered.contains(&candidate.uid));
    let uids: Vec<u32> = filed.iter().map(|candidate| candidate.uid).collect();
    ai_sort_release(conn, account, folder, &uids)?;
    refund_usage(conn, uids.len())?;
    Ok(kept)
}

/// This device's id for its AI sorting claims (see `imap::claim_for_ai_sort`):
/// eight letters and digits, made once. Kept in `meta`, which no backup and no
/// setup copy carries: two devices must never share one.
pub fn ai_sort_claim_id(conn: &Connection) -> Result<String> {
    let known: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![CLAIM_ID_KEY],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = known.filter(|id| !id.is_empty()) {
        return Ok(id);
    }
    let id: String = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect();
    conn.execute(
        "INSERT INTO meta(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![CLAIM_ID_KEY, id],
    )?;
    Ok(id)
}

/// Held-back mail another of the reader's devices sorts (it has the claim):
/// not counted against today's cap here. `shown` shows again; `gone`, which
/// that device moved away, stays hidden until a sync drops its row.
pub fn ai_sort_left_to_other(
    conn: &Connection,
    account: &str,
    folder: &str,
    shown: &[u32],
    gone: &[u32],
) -> Result<()> {
    ai_sort_release(conn, account, folder, shown)?;
    let until = crate::store::now_unix() + AI_SORT_MOVE_HOLD_SECS;
    for uid in gone {
        conn.execute(
            "UPDATE ai_sort_hidden SET hide_until = ?4
             WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            params![account, folder, uid, until],
        )?;
    }
    refund_usage(conn, shown.len() + gone.len())
}

/// Mail taken over from a device that never finished sorting it: counted
/// against today's cap again.
pub fn ai_sort_count_taken_over(conn: &Connection, count: usize) -> Result<()> {
    add_usage(conn, count)
}

fn refund_usage(conn: &Connection, count: usize) -> Result<()> {
    if count == 0 {
        return Ok(());
    }
    conn.execute(
        "UPDATE ai_sort_usage SET count = MAX(0, count - ?2) WHERE day = ?1",
        params![today(), count as i64],
    )?;
    Ok(())
}

/// The candidates grouped by conversation, so the AI is asked once per
/// conversation (about its newest email, first in each group) and the whole
/// conversation gets one answer. A conversation with rule work queued for any
/// of its emails here is the rules': all of it is shown again and left out.
pub fn ai_sort_group_conversations(
    conn: &Connection,
    account: &str,
    folder: &str,
    candidates: Vec<AiSortCandidate>,
) -> Result<Vec<Vec<AiSortCandidate>>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let pending: BTreeSet<u32> = crate::store::pending_rule_actions(conn, account, folder)?
        .into_iter()
        .map(|action| action.uid)
        .collect();
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, Vec<(i64, AiSortCandidate)>> = BTreeMap::new();
    for candidate in candidates {
        let key = conversation_of(conn, account, folder, &candidate)?;
        let date: i64 = conn
            .query_row(
                "SELECT date FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
                params![account, folder, candidate.uid],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push((date, candidate));
    }
    let mut out = Vec::new();
    let mut ruled = Vec::new();
    for key in order {
        let mut group = groups.remove(&key).unwrap_or_default();
        group.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.uid.cmp(&a.1.uid)));
        let members: Vec<AiSortCandidate> = group.into_iter().map(|(_, it)| it).collect();
        let conversation_ruled = !pending.is_empty()
            && (members.iter().any(|it| pending.contains(&it.uid))
                || conversation_uids(conn, account, folder, &key)?
                    .iter()
                    .any(|uid| pending.contains(uid)));
        if conversation_ruled {
            ruled.extend(members.iter().map(|it| it.uid));
        } else {
            out.push(members);
        }
    }
    ai_sort_release(conn, account, folder, &ruled)?;
    Ok(out)
}

/// A candidate's conversation: its Meron-folder conversation key, or its own
/// identity when the row is gone.
pub(super) fn conversation_of(
    conn: &Connection,
    account: &str,
    folder: &str,
    candidate: &AiSortCandidate,
) -> Result<String> {
    Ok(
        crate::store::meron_conversation_key(conn, account, folder, candidate.uid)?
            .unwrap_or_else(|| format!("\u{0}{}", candidate.identity)),
    )
}

/// The UIDs in `folder` of the conversation `key` names.
fn conversation_uids(
    conn: &Connection,
    account: &str,
    folder: &str,
    key: &str,
) -> Result<Vec<u32>> {
    let mut stmt = conn.prepare_cached(
        "SELECT uid FROM messages
          WHERE account = ?1 AND lower(COALESCE(thread_key, '')) = ?2 AND folder = ?3 AND uid <> 0",
    )?;
    let rows = stmt.query_map(params![account, key, folder], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Show held-back mail again.
pub fn ai_sort_release(conn: &Connection, account: &str, folder: &str, uids: &[u32]) -> Result<()> {
    let mut stmt =
        conn.prepare("DELETE FROM ai_sort_hidden WHERE account = ?1 AND folder = ?2 AND uid = ?3")?;
    for uid in uids {
        stmt.execute(params![account, folder, uid])?;
    }
    Ok(())
}

/// Drop a folder's hidden rows when its UIDs reset.
pub fn ai_sort_clear_folder(conn: &Connection, account: &str, folder: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM ai_sort_hidden WHERE account = ?1 AND folder = ?2",
        params![account, folder],
    )?;
    Ok(())
}

/// When the last hidden row of `folder` stops hiding, if any is left.
pub fn ai_sort_hidden_until(conn: &Connection, account: &str, folder: &str) -> Result<Option<i64>> {
    Ok(conn.query_row(
        "SELECT MAX(hide_until) FROM ai_sort_hidden WHERE account = ?1 AND folder = ?2",
        params![account, folder],
        |row| row.get(0),
    )?)
}
