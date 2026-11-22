//! Reads and maintains the local sorting history.
//! Log updates and alerts expose the status of a decision to the mail hosts.
//! Cleanup handles deleted accounts, forgotten folders and restored folder identities.

use super::*;

/// The log ids behind queued AI actions' `rule_ids`.
fn ai_log_ids(rule_ids: &[String]) -> Vec<i64> {
    rule_ids
        .iter()
        .filter_map(|id| id.strip_prefix(AI_RULE_ID)?.strip_prefix(':')?.parse().ok())
        .collect()
}

/// Whether a queued action's rule id is the AI's.
pub fn is_ai_rule_id(id: &str) -> bool {
    id == AI_RULE_ID || id.starts_with("ai-sort:")
}

/// Mark the moves the server just made for the AI. Called before the queue
/// entries are cleared, as they say which log rows these were.
pub fn ai_sort_mark_moved(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
    run: i64,
) -> Result<()> {
    if uids.is_empty() {
        return Ok(());
    }
    let mut stmt = conn.prepare(
        "SELECT rule_ids FROM rule_actions
         WHERE account = ?1 AND folder = ?2 AND uid = ?3 AND run = ?4",
    )?;
    for uid in uids {
        let ids: Option<String> = stmt
            .query_row(params![account, folder, uid, run], |row| row.get(0))
            .optional()?;
        let ids: Vec<String> = ids
            .and_then(|ids| serde_json::from_str(&ids).ok())
            .unwrap_or_default();
        for id in ai_log_ids(&ids) {
            conn.execute(
                "UPDATE ai_sort_log SET state = 'moved', uid = 0
                 WHERE id = ?1 AND state = 'moving'",
                params![id],
            )?;
        }
    }
    ai_sort_release(conn, account, folder, uids)
}

/// A queued AI move that failed for good (`alert`: new mail, which still owes
/// its new-mail alert), or lost to a rule that moved the mail first.
pub(in crate::store) fn ai_sort_log_failed(
    conn: &Connection,
    account: &str,
    folder: &str,
    action: &RuleAction,
    reason: &str,
    alert: bool,
) -> Result<()> {
    for id in ai_log_ids(&action.rule_ids) {
        conn.execute(
            "UPDATE ai_sort_log SET state = 'failed', detail = ?2, alert = ?3
             WHERE id = ?1 AND state = 'moving'",
            params![id, reason, alert && action.run == 0],
        )?;
    }
    ai_sort_release(conn, account, folder, &[action.uid])
}

/// The new mail whose AI move failed for good since the last look, to alert
/// for now, as a failed rule move still alerts.
pub fn ai_sort_take_alerts(
    conn: &Connection,
    account: &str,
    folder: &str,
) -> Result<Vec<MessageHeader>> {
    let validity =
        crate::store::get_folder_state(conn, account, folder)?.map(|(validity, _)| validity);
    let mut stmt = conn.prepare(
        "SELECT id, uid, uidvalidity FROM ai_sort_log
         WHERE account = ?1 AND alert <> 0",
    )?;
    let rows = stmt
        .query_map(params![account], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, u32>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for (id, uid, uidvalidity) in rows {
        conn.execute(
            "UPDATE ai_sort_log SET alert = 0 WHERE id = ?1",
            params![id],
        )?;
        if Some(uidvalidity) != validity {
            continue;
        }
        let header = conn
            .query_row(
                "SELECT uid, subject, from_name, from_addr, date, seen, starred, thread_key,
                        json_extract(json, '$.to') FROM messages
                 WHERE account = ?1 AND folder = ?2 AND uid = ?3 AND seen = 0",
                params![account, folder, uid],
                crate::store::message_header_from_row,
            )
            .optional()?;
        out.extend(header);
    }
    Ok(out)
}

fn log_entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AiSortLogEntry> {
    Ok(AiSortLogEntry {
        id: row.get(0)?,
        at: row.get(1)?,
        account: row.get(2)?,
        subject: row.get(3)?,
        sender: row.get(4)?,
        to_folder: row.get(5)?,
        reason: row.get(6)?,
        state: row.get(7)?,
        detail: row.get(8)?,
        message_id: row.get(9)?,
        meron_folder_id: row.get(10)?,
        uid: row.get(11)?,
        from_folder: row.get(12)?,
        uidvalidity: row.get(13)?,
    })
}

/// What the AI did lately, newest first.
pub fn ai_sort_log(conn: &Connection) -> Result<Vec<AiSortLogEntry>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {LOG_COLUMNS} FROM {LOG_FROM} ORDER BY l.id DESC"
    ))?;
    let rows = stmt.query_map([], log_entry_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn ai_sort_log_entry(conn: &Connection, id: i64) -> Result<Option<AiSortLogEntry>> {
    Ok(conn
        .query_row(
            &format!("SELECT {LOG_COLUMNS} FROM {LOG_FROM} WHERE l.id = ?1"),
            params![id],
            log_entry_from_row,
        )
        .optional()?)
}

pub fn ai_sort_set_log_state(conn: &Connection, id: i64, state: &str, detail: &str) -> Result<()> {
    conn.execute(
        "UPDATE ai_sort_log SET state = ?2, detail = ?3 WHERE id = ?1",
        params![id, state, detail],
    )?;
    Ok(())
}

/// The cached Inbox mail of `account` that "Sort Inbox now" would send: stored
/// on this device, not decided yet, not held back, with no rule work queued,
/// and not past [`AI_SORT_MAX_AGE_DAYS`].
pub fn ai_sort_existing_candidates(
    conn: &Connection,
    account: &str,
) -> Result<Vec<AiSortCandidate>> {
    let now = crate::store::now_unix();
    let folder = crate::store::EXISTING_RULE_FOLDER;
    let pending: BTreeSet<u32> = crate::store::pending_rule_actions(conn, account, folder)?
        .into_iter()
        .map(|action| action.uid)
        .collect();
    let mut stmt = conn.prepare(&format!(
        "SELECT uid, COALESCE(from_name, ''), COALESCE(from_addr, ''), COALESCE(subject, ''),
                COALESCE(json_extract(json, '$.message_id'), ''), {IDENTITY_SQL}
           FROM messages
          WHERE account = ?1 AND folder = ?2 AND uid <> 0
            AND NOT EXISTS (SELECT 1 FROM ai_sort_hidden h
                 WHERE h.account = messages.account AND h.folder = messages.folder
                   AND h.uid = messages.uid
                   AND h.hide_until > CAST(strftime('%s','now') AS INTEGER))
            AND NOT {hidden}
            AND NOT (COALESCE(date, 0) > 0 AND date <= ?4 AND date < ?3)
          ORDER BY date DESC, uid DESC",
        hidden = crate::store::meron_hidden_sql("messages"),
    ))?;
    let rows = stmt
        .query_map(
            params![account, folder, ai_sort_age_cutoff(now), now],
            |row| {
                Ok(AiSortCandidate {
                    uid: row.get(0)?,
                    from_name: row.get(1)?,
                    from_addr: row.get(2)?,
                    subject: row.get(3)?,
                    message_id: row.get(4)?,
                    identity: row.get(5)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for candidate in rows {
        if !pending.contains(&candidate.uid) && !is_done(conn, account, &candidate.identity)? {
            out.push(candidate);
        }
    }
    Ok(out)
}

/// Stop a "sort Inbox now" job: what is done stays done, the queued rest is
/// dropped and its log rows say so.
pub fn ai_sort_stop_run(conn: &Connection) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let mut stmt = tx.prepare(
        "SELECT account, folder, uid, rule_ids FROM rule_actions
         WHERE run <> 0 AND rule_ids LIKE '[\"ai-sort:%'",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u32>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for (account, folder, uid, ids) in rows {
        let ids: Vec<String> = serde_json::from_str(&ids).unwrap_or_default();
        for id in ai_log_ids(&ids) {
            tx.execute(
                "UPDATE ai_sort_log SET state = 'left', detail = 'stopped'
                 WHERE id = ?1 AND state = 'moving'",
                params![id],
            )?;
        }
        tx.execute(
            "DELETE FROM rule_actions
             WHERE account = ?1 AND folder = ?2 AND uid = ?3 AND run <> 0
               AND rule_ids LIKE '[\"ai-sort:%'",
            params![account, folder, uid],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Take a removed account out of AI sorting: its rows here, and its folders
/// from the setting.
pub fn ai_sort_forget_account(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM ai_sort_hidden WHERE account = ?1", params![id])?;
    conn.execute("DELETE FROM ai_sort_done WHERE account = ?1", params![id])?;
    conn.execute("DELETE FROM ai_sort_log WHERE account = ?1", params![id])?;
    crate::store::ai_lessons_forget_account(conn, id)?;
    crate::store::ai_sort_inbox::ai_sort_inbox_forget_account(conn, id)?;
    let mut config = ai_sort_config(conn)?;
    if config.folders.iter().any(|folder| folder.account == id) {
        config.folders.retain(|folder| folder.account != id);
        if config.folders.is_empty() {
            config.enabled = false;
        }
        crate::store::setting_set(conn, AI_SORT_KEY, &serde_json::to_value(&config)?)?;
    }
    Ok(())
}

/// A deleted Meron folder: out of the setting (sorting goes off when no folder
/// is left), and its log rows read as moved back. Inside the delete's
/// transaction.
pub fn ai_sort_forget_meron_folder(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE ai_sort_log SET state = 'moved_back'
          WHERE meron_folder_id = ?1 AND state = 'moved'",
        params![id],
    )?;
    crate::store::ai_lessons_forget_meron_folder(conn, id)?;
    let mut config = ai_sort_config(conn)?;
    if config
        .folders
        .iter()
        .any(|folder| folder.meron_folder_id == id)
    {
        config.folders.retain(|folder| folder.meron_folder_id != id);
        if config.folders.is_empty() {
            config.enabled = false;
        }
        crate::store::setting_set(conn, AI_SORT_KEY, &serde_json::to_value(&config)?)?;
    }
    Ok(())
}

/// After a backup restore kept Meron folders under other ids (a folder of the
/// same name was here already): point the setting at the kept ones.
pub fn ai_sort_remap_meron_ids(
    conn: &Connection,
    map: &std::collections::HashMap<String, String>,
) -> Result<()> {
    crate::store::ai_lessons_remap_meron_ids(conn, map)?;
    let mut config = ai_sort_config(conn)?;
    let mut changed = false;
    for folder in &mut config.folders {
        if let Some(kept) = map.get(&folder.meron_folder_id)
            && *kept != folder.meron_folder_id
        {
            folder.meron_folder_id = kept.clone();
            changed = true;
        }
    }
    if changed {
        crate::store::setting_set(conn, AI_SORT_KEY, &serde_json::to_value(&config)?)?;
    }
    Ok(())
}
