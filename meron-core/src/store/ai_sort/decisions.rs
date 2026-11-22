//! Records the outcome of sorting a conversation inside SQLite.
//! These transactions coordinate folder entries, queued moves and the decision log.
//! Recovery queries decide which recorded moves still need attention.

use super::*;

/// One log row. `folder` is where the email was when the AI saw it, so Move
/// back finds mail from a folder other than the Inbox.
#[allow(clippy::too_many_arguments)]
pub(super) fn insert_log(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    candidate: &AiSortCandidate,
    to_folder: &str,
    reason: &str,
    state: &str,
    detail: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO ai_sort_log
           (at, account, uid, uidvalidity, message_id, subject, sender, to_folder, reason,
            state, detail, from_folder)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            crate::store::now_unix(),
            account,
            candidate.uid,
            uidvalidity,
            candidate.message_id,
            candidate.subject,
            candidate.sender(),
            to_folder,
            reason,
            state,
            detail,
            folder
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Keep the newest [`AI_SORT_LOG_SIZE`] rows, and always those still in use: a
/// move still queued (`moving`) and an alert still owed.
pub(super) fn trim_log(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM ai_sort_log
         WHERE id <= (SELECT MAX(id) FROM ai_sort_log) - ?1
           AND state <> 'moving' AND alert = 0",
        params![AI_SORT_LOG_SIZE],
    )?;
    Ok(())
}

/// Record sorting decisions and queue accepted moves in one transaction.
///
/// Recheck each message's UIDVALIDITY, location, prior decision and queued rules.
/// A zero `run` identifies incoming mail. Other values identify an explicit Inbox sort.
///
/// Check `in_time` after acquiring the store. Return `None` when time has expired.
/// Otherwise return the accepted UIDs and those queued for a move, and show skipped mail again.
pub fn ai_sort_record_decisions(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    decisions: &[AiSortDecision],
    run: i64,
    in_time: &dyn Fn() -> bool,
) -> Result<Option<AiSortRecorded>> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if !in_time() {
        return Ok(None);
    }
    let current =
        crate::store::get_folder_state(&tx, account, folder)?.map(|(validity, _)| validity);
    let pending: BTreeSet<u32> = crate::store::pending_rule_actions(&tx, account, folder)?
        .into_iter()
        .map(|action| action.uid)
        .collect();
    let now = crate::store::now_unix();
    // New mail of a folder other than the Inbox (see `ai_sort_hold_folder`).
    let outside = !folder.eq_ignore_ascii_case("INBOX");
    let mut moving = Vec::new();
    let mut accepted = Vec::new();
    let mut filed = Vec::new();
    let mut inbox_moving = Vec::new();
    let mut shown = Vec::new();
    // One answer per conversation: rule work queued for any email of it makes
    // the whole conversation the rules'.
    let mut ruled_conversations = BTreeSet::new();
    let mut conversations = Vec::with_capacity(decisions.len());
    for decision in decisions {
        let key = conversation_of(&tx, account, folder, &decision.candidate)?;
        if pending.contains(&decision.candidate.uid) {
            ruled_conversations.insert(key.clone());
        }
        conversations.push(key);
    }
    // Conversations this call filed into a Meron folder, and the folder.
    let mut filed_conversations: BTreeMap<String, String> = BTreeMap::new();
    let mut filed_to = Vec::new();
    for (decision, conversation) in decisions.iter().zip(&conversations) {
        let candidate = &decision.candidate;
        if ruled_conversations.contains(conversation) {
            shown.push(candidate.uid);
            continue;
        }
        // Another new email of a conversation just filed: it went into the
        // Meron folder with it, so it is filed too and arrives silently.
        if let Some(id) = filed_conversations.get(conversation) {
            filed.push(candidate.uid);
            filed_to.push((candidate.uid, id.clone()));
            shown.push(candidate.uid);
            continue;
        }
        // Already in a Meron folder (moved there by hand during the hold):
        // never re-sorted.
        if crate::store::meron_uid_in_folder(&tx, account, folder, candidate.uid)? {
            shown.push(candidate.uid);
            continue;
        }
        let present = tx
            .query_row(
                "SELECT 1 FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
                params![account, folder, candidate.uid],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if current != Some(uidvalidity)
            || !present
            || pending.contains(&candidate.uid)
            || is_done(&tx, account, &candidate.identity)?
        {
            shown.push(candidate.uid);
            continue;
        }
        if let Some(crate::ai_sort::SortTarget::Meron(id)) = &decision.target {
            let Some(name) = crate::store::meron_folder_name(&tx, id)? else {
                insert_log(
                    &tx,
                    account,
                    folder,
                    uidvalidity,
                    candidate,
                    "",
                    &decision.why,
                    "failed",
                    "The Meron folder was deleted.",
                )?;
                if outside {
                    crate::store::ai_sort_show_in_inbox(
                        &tx,
                        account,
                        folder,
                        uidvalidity,
                        std::slice::from_ref(candidate),
                    )?;
                }
                shown.push(candidate.uid);
                continue;
            };
            let key = crate::store::meron_conversation_key(&tx, account, folder, candidate.uid)?
                .ok_or_else(|| anyhow::anyhow!("the email is no longer cached"))?;
            // The whole conversation moves; nothing is queued for the server
            // but the `$MeronSorted` mark.
            // Mail of the conversation older than 90 days in other folders
            // stays out (it would show there as old cards).
            crate::store::meron_move_in_in(
                &tx,
                account,
                &key,
                id,
                crate::store::MeronFiling::Ai { folder, now },
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO ai_sort_done(account, identity, at) VALUES(?1, ?2, ?3)",
                params![account, candidate.identity, now],
            )?;
            let log_id = insert_log(
                &tx,
                account,
                folder,
                uidvalidity,
                candidate,
                &name,
                &decision.why,
                "moved",
                "",
            )?;
            tx.execute(
                "UPDATE ai_sort_log SET meron_folder_id = ?2 WHERE id = ?1",
                params![log_id, id],
            )?;
            accepted.push(candidate.uid);
            filed.push(candidate.uid);
            filed_to.push((candidate.uid, id.clone()));
            filed_conversations.insert(conversation.clone(), id.clone());
            // Shown at once: the Meron hide rule hides it in the same commit.
            shown.push(candidate.uid);
            continue;
        }
        tx.execute(
            "INSERT OR IGNORE INTO ai_sort_done(account, identity, at) VALUES(?1, ?2, ?3)",
            params![account, candidate.identity, now],
        )?;
        accepted.push(candidate.uid);
        // Mail outside the Inbox is never moved on the server: a server folder
        // picked for it counts as no match, and it shows in the Inbox here.
        let server = match decision.target.as_ref() {
            Some(crate::ai_sort::SortTarget::Server(target)) if !outside => Some(target),
            _ => None,
        };
        let Some(target) = server else {
            let picked = match decision.target.as_ref() {
                Some(crate::ai_sort::SortTarget::Server(target)) => Some(target.as_str()),
                _ => None,
            };
            // A server folder for mail whose copy is in the Inbox (Gmail): the
            // Inbox copy moves there, as new Inbox mail would; this copy stays.
            if let Some(target) = picked
                && let Some(inbox_uid) = crate::store::ai_sort_find_cached(
                    &tx,
                    account,
                    crate::store::EXISTING_RULE_FOLDER,
                    &candidate.message_id,
                )?
                .filter(|_| !candidate.message_id.trim().is_empty())
                && let Some((inbox_validity, _)) = crate::store::get_folder_state(
                    &tx,
                    account,
                    crate::store::EXISTING_RULE_FOLDER,
                )?
            {
                let id = insert_log(
                    &tx,
                    account,
                    folder,
                    uidvalidity,
                    candidate,
                    target,
                    &decision.why,
                    "moving",
                    "",
                )?;
                if queue_inbox_copy_move(&tx, account, inbox_validity, inbox_uid, id, target, now)?
                {
                    inbox_moving.push(inbox_uid);
                    shown.push(candidate.uid);
                    continue;
                }
                tx.execute(
                    "UPDATE ai_sort_log SET state = 'left', to_folder = '' WHERE id = ?1",
                    params![id],
                )?;
                shown.push(candidate.uid);
                continue;
            }
            let detail = if decision.target.is_some() {
                "Mail outside the Inbox isn't moved on the server."
            } else {
                ""
            };
            let id = insert_log(
                &tx,
                account,
                folder,
                uidvalidity,
                candidate,
                "",
                &decision.why,
                "left",
                detail,
            )?;
            // Kept for its Inbox copy, should one turn up later (Gmail): that
            // copy gets this move without asking the AI again.
            if let Some(target) = picked {
                tx.execute(
                    "UPDATE ai_sort_log SET server_target = ?2 WHERE id = ?1",
                    params![id, target],
                )?;
            }
            if outside {
                crate::store::ai_sort_show_in_inbox(
                    &tx,
                    account,
                    folder,
                    uidvalidity,
                    std::slice::from_ref(candidate),
                )?;
            }
            shown.push(candidate.uid);
            continue;
        };
        let target = target.as_str();
        let id = insert_log(
            &tx,
            account,
            folder,
            uidvalidity,
            candidate,
            target,
            &decision.why,
            "moving",
            "",
        )?;
        let action = RuleAction {
            folder: folder.to_string(),
            uid: candidate.uid,
            uidvalidity,
            target: target.to_string(),
            rule_ids: vec![format!("{AI_RULE_ID}:{id}")],
            run,
            ..RuleAction::default()
        };
        crate::store::rules::queue_rule_actions(
            &tx,
            account,
            &BTreeMap::from([(candidate.uid, action)]),
            run,
        )?;
        if run == 0 {
            tx.execute(
                "UPDATE ai_sort_hidden SET hide_until = ?4
                 WHERE account = ?1 AND folder = ?2 AND uid = ?3",
                params![account, folder, candidate.uid, now + AI_SORT_MOVE_HOLD_SECS],
            )?;
        }
        moving.push(candidate.uid);
    }
    ai_sort_release(&tx, account, folder, &shown)?;
    trim_log(&tx)?;
    tx.commit()?;
    Ok(Some(AiSortRecorded {
        moving,
        accepted,
        filed,
        filed_to,
        inbox_moving,
    }))
}

/// Of `candidates`, those still open for the AI: still cached here, not held
/// back, not decided already, with no rule work queued.
pub fn ai_sort_still_open(
    conn: &Connection,
    account: &str,
    folder: &str,
    candidates: &[AiSortCandidate],
) -> Result<Vec<AiSortCandidate>> {
    let pending: BTreeSet<u32> = crate::store::pending_rule_actions(conn, account, folder)?
        .into_iter()
        .map(|action| action.uid)
        .collect();
    let mut open = Vec::new();
    for candidate in candidates {
        let present = conn
            .query_row(
                &format!(
                    "SELECT 1 FROM messages m WHERE m.account = ?1 AND m.folder = ?2 AND m.uid = ?3
                       AND NOT EXISTS (SELECT 1 FROM ai_sort_hidden h
                            WHERE h.account = m.account AND h.folder = m.folder AND h.uid = m.uid
                              AND h.hide_until > CAST(strftime('%s','now') AS INTEGER))
                       AND NOT {hidden}",
                    hidden = crate::store::meron_hidden_sql("m"),
                ),
                params![account, folder, candidate.uid],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if present
            && !pending.contains(&candidate.uid)
            && !is_done(conn, account, &candidate.identity)?
        {
            open.push(candidate.clone());
        }
    }
    Ok(open)
}

/// Of `uids`, those with an AI move of new mail queued: what a sort that ran
/// out of time still committed, so the caller doesn't alert for it.
pub fn ai_sort_queued_new_moves(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
) -> Result<Vec<u32>> {
    let wanted: BTreeSet<u32> = uids.iter().copied().collect();
    let mut out: Vec<u32> = crate::store::pending_rule_actions(conn, account, folder)?
        .into_iter()
        .filter(|action| {
            action.run == 0
                && wanted.contains(&action.uid)
                && action.rule_ids.iter().any(|id| is_ai_rule_id(id))
        })
        .map(|action| action.uid)
        .collect();
    // And what such a sort filed into a Meron folder, which arrives silently.
    for uid in crate::store::meron_foldered_uids(conn, account, folder, uids)? {
        if !out.contains(&uid) {
            out.push(uid);
        }
    }
    Ok(out)
}

/// The AI couldn't answer in time (`slow`) or at all (`failed`): show the mail
/// and say so. Nothing is marked decided, so "Sort Inbox now" can try again.
pub fn ai_sort_give_up(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    candidates: &[AiSortCandidate],
    state: &str,
    detail: &str,
) -> Result<()> {
    let mut open = Vec::new();
    for candidate in candidates {
        // An answer recorded just as time ran out stands.
        if !is_done(conn, account, &candidate.identity)? {
            open.push(candidate);
        }
    }
    let uids: Vec<u32> = open.iter().map(|candidate| candidate.uid).collect();
    ai_sort_release(conn, account, folder, &uids)?;
    for candidate in &open {
        insert_log(
            conn,
            account,
            folder,
            uidvalidity,
            candidate,
            "",
            "",
            state,
            detail,
        )?;
    }
    // Mail from another folder the AI failed on shows in the Inbox. (Not tried
    // again: "Sort Inbox now" covers the Inbox's own mail only.)
    if !folder.eq_ignore_ascii_case("INBOX") {
        let open: Vec<AiSortCandidate> = open.into_iter().cloned().collect();
        crate::store::ai_sort_show_in_inbox(conn, account, folder, uidvalidity, &open)?;
    }
    trim_log(conn)
}

/// The UID of the cached copy of `message_id` in `folder`, if there is one.
pub fn ai_sort_find_cached(
    conn: &Connection,
    account: &str,
    folder: &str,
    message_id: &str,
) -> Result<Option<u32>> {
    Ok(conn
        .query_row(
            "SELECT uid FROM messages
             WHERE account = ?1 AND folder = ?2 AND uid <> 0
               AND lower(COALESCE(json_extract(json, '$.message_id'), '')) = lower(?3)
             LIMIT 1",
            params![account, folder, message_id.trim()],
            |row| row.get(0),
        )
        .optional()?)
}
