//! Manages queued mail-rule actions and their completion state.
//! Matching existing mail feeds the same pending-action store as incoming mail.
//! Retries, lost moves and tag replacements update the affected rules and records.

use super::*;

pub(in crate::store) fn queue_rule_actions(
    conn: &Connection,
    account: &str,
    actions: &BTreeMap<u32, RuleAction>,
    run: i64,
) -> Result<()> {
    for action in actions.values() {
        // The move delivers it there, on all the rules combined.
        let mut copies: Vec<&RuleCopy> = Vec::new();
        for copy in &action.copies {
            let delivered = copy.account == account
                && (copy.folder == action.target
                    || (action.trash
                        && crate::store::folder_role(conn, account, &copy.folder)? == "trash"));
            if !delivered {
                copies.push(copy);
            }
        }
        if !action.mark_read && !action.star && !action.moves() && copies.is_empty() {
            continue;
        }
        conn.execute(
            "INSERT OR REPLACE INTO rule_actions
               (account, folder, uid, uidvalidity, mark_read, star, trash, target, copies,
                rule_ids, run, attempts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
            params![
                account,
                action.folder,
                action.uid,
                action.uidvalidity,
                action.mark_read,
                action.star,
                action.trash,
                action.target,
                serde_json::to_string(&copies)?,
                serde_json::to_string(&action.rule_ids)?,
                run
            ],
        )?;
    }
    Ok(())
}

/// The accounts `rule` can run on: its own, or every account with a server.
fn rule_accounts(conn: &Connection, rule: &Rule) -> Result<Vec<String>> {
    let field = |account: &serde_json::Value, name: &str| {
        account[name].as_str().unwrap_or_default().to_string()
    };
    Ok(crate::store::list_accounts(conn)?
        .iter()
        .filter(|account| field(account, "engine") != "rss" && field(account, "provider") != "rss")
        .map(|account| field(account, "id"))
        .filter(|id| !crate::store::is_local_account(id))
        .filter(|id| rule.accounts.is_empty() || rule.accounts.contains(id))
        .collect())
}

fn saved_rule(conn: &Connection, id: &str) -> Result<Rule> {
    rules(conn)?
        .into_iter()
        .find(|rule| rule.id == id)
        .ok_or_else(|| anyhow::anyhow!("no such rule: {id}"))
}

/// The accounts a saved rule can run on.
pub fn saved_rule_accounts(conn: &Connection, id: &str) -> Result<Vec<String>> {
    rule_accounts(conn, &saved_rule(conn, id)?)
}

/// What `rule` matches among the mail cached in `account`'s Inbox.
fn existing_rule_matches(conn: &Connection, account: &str, rule: &Rule) -> Result<Vec<u32>> {
    let folder = EXISTING_RULE_FOLDER;
    let mut stmt =
        conn.prepare("SELECT uid FROM messages WHERE account = ?1 AND folder = ?2 AND uid <> 0")?;
    let uids = stmt
        .query_map(params![account, folder], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<u32>>>()?;
    if rule.query.is_empty() {
        return Ok(uids);
    }
    crate::store::matching_uids(conn, account, folder, &rule.query, &uids)
}

/// How many messages [`queue_existing_rule`] would look at.
pub fn count_existing_rule_matches(conn: &Connection, id: &str) -> Result<usize> {
    let rule = saved_rule(conn, id)?;
    let mut count = 0;
    for account in rule_accounts(conn, &rule)? {
        count += existing_rule_matches(conn, &account, &rule)?.len();
    }
    Ok(count)
}

/// Run a saved rule over mail already in the Inbox, for [`crate::engine::run_rule_on_existing`] to carry out.
///
/// Answers with the accounts and how many messages each queued. Mail this rule already has work
/// queued for keeps that work; other rules' work runs alongside, under its own run. And
/// [`crate::engine`] asks the target before each copy, so running a rule twice doesn't double what
/// it did.
pub fn queue_existing_rule(conn: &Connection, id: &str) -> Result<Vec<(String, usize)>> {
    let folder = EXISTING_RULE_FOLDER;
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let rule = saved_rule(&tx, id)?;
    anyhow::ensure!(!rule.disabled, "the rule is turned off");
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_micros() as i64;
    let mut queued = Vec::new();
    for account in rule_accounts(&tx, &rule)? {
        let Some((uidvalidity, _)) = crate::store::get_folder_state(&tx, &account, folder)? else {
            continue;
        };
        let pending = pending_rule_actions(&tx, &account, folder)?;
        let matched: Vec<u32> = existing_rule_matches(&tx, &account, &rule)?
            .into_iter()
            .filter(|uid| {
                !pending
                    .iter()
                    .any(|action| action.uid == *uid && action.rule_ids.contains(&rule.id))
            })
            .collect();
        let count = matched.len();
        let mut actions = BTreeMap::new();
        add_rule_effects(
            &tx,
            &account,
            folder,
            uidvalidity,
            &rule,
            matched,
            &mut actions,
        )?;
        queue_rule_actions(&tx, &account, &actions, run)?;
        if count > 0 {
            log_rule(&tx, &rule.id, &account, "", count, None)?;
            queued.push((account, count));
        }
    }
    trim_rule_log(&tx)?;
    tx.commit()?;
    Ok(queued)
}

pub fn pending_rule_actions(
    conn: &Connection,
    account: &str,
    folder: &str,
) -> Result<Vec<RuleAction>> {
    let mut stmt = conn.prepare(
        "SELECT folder, uid, uidvalidity, mark_read, star, trash, target, copies, rule_ids, run
         FROM rule_actions WHERE account = ?1 AND folder = ?2 ORDER BY uid",
    )?;
    let rows = stmt.query_map(params![account, folder], |row| {
        Ok(RuleAction {
            folder: row.get(0)?,
            uid: row.get(1)?,
            uidvalidity: row.get(2)?,
            mark_read: row.get(3)?,
            star: row.get(4)?,
            trash: row.get(5)?,
            target: row.get(6)?,
            copies: serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default(),
            rule_ids: serde_json::from_str(&row.get::<_, String>(8)?).unwrap_or_default(),
            run: row.get(9)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Note moves that didn't happen, as another rule had already moved the message.
pub fn log_rule_lost_moves(conn: &Connection, account: &str, lost: &[RuleAction]) -> Result<()> {
    for action in lost {
        for rule_id in &action.rule_ids {
            if crate::store::is_ai_rule_id(rule_id) {
                let reason = "a rule moved it first";
                crate::store::ai_sort::ai_sort_log_failed(
                    conn,
                    account,
                    &action.folder,
                    action,
                    reason,
                    false,
                )?;
                continue;
            }
            let reason = "another rule had already moved this message";
            log_rule(conn, rule_id, account, "", 1, Some(reason))?;
        }
    }
    trim_rule_log(conn)
}

/// Clear what `run` queued for `uids`.
pub fn finish_rule_actions(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
    run: i64,
) -> Result<()> {
    for uid in uids {
        conn.execute(
            "DELETE FROM rule_actions
             WHERE account = ?1 AND folder = ?2 AND uid = ?3 AND run = ?4",
            params![account, folder, uid, run],
        )?;
    }
    Ok(())
}

/// Count a failed try, dropping what has now had its last one and logging it
/// against the rules behind it. Answers with how many were dropped.
pub fn retry_rule_actions(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
    run: i64,
    reason: &str,
) -> Result<usize> {
    let queued = pending_rule_actions(conn, account, folder)?;
    let mut dropped = 0;
    for uid in uids {
        conn.execute(
            "UPDATE rule_actions SET attempts = attempts + 1
             WHERE account = ?1 AND folder = ?2 AND uid = ?3 AND run = ?4",
            params![account, folder, uid, run],
        )?;
        let gone = conn.execute(
            "DELETE FROM rule_actions
             WHERE account = ?1 AND folder = ?2 AND uid = ?3 AND run = ?4 AND attempts >= ?5",
            params![account, folder, uid, run, MAX_RULE_ACTION_ATTEMPTS],
        )?;
        dropped += gone;
        if gone == 0 {
            continue;
        }
        let subject = rule_message_subject(conn, account, folder, *uid)?;
        let action = queued
            .iter()
            .find(|action| action.uid == *uid && action.run == run);
        for rule_id in action.iter().flat_map(|action| &action.rule_ids) {
            // The AI's own log says its move failed; new mail alerts now.
            if crate::store::is_ai_rule_id(rule_id)
                && let Some(action) = action
            {
                crate::store::ai_sort::ai_sort_log_failed(
                    conn, account, folder, action, reason, true,
                )?;
                continue;
            }
            log_rule(conn, rule_id, account, &subject, 1, Some(reason))?;
        }
    }
    trim_rule_log(conn)?;
    Ok(dropped)
}

/// [`crate::store::replace_tag`] for the rules: they apply the new name, or lose the
/// tag, and a rule left doing nothing is gone. Answers with the rules as they
/// now stand.
pub fn replace_rule_tag(conn: &Connection, tag: &str, to: Option<&str>) -> Result<Vec<Rule>> {
    let current = rules(conn)?;
    if to == Some(tag) {
        return Ok(current);
    }
    let mut next = Vec::with_capacity(current.len());
    for mut rule in current {
        if let Some(to) = to {
            rule.query = crate::search_query::rename_tag_terms(&rule.query, tag, to);
        }
        if rule.tags.iter().any(|item| item == tag) {
            rule.tags.retain(|item| item != tag);
            rule.tags.extend(to.map(str::to_string));
            rule.tags.sort();
            rule.tags.dedup();
        }
        if rule.has_action() {
            next.push(rule);
        }
    }
    set_rules(conn, next)
}
