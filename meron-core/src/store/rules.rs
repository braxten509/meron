//! Persists mail rules and matches them against messages in the cache.
//! Incoming mail and explicit rule runs both produce pending actions.
//! The engine consumes the queue and records completion for the desktop and mobile hosts.

mod pending;
pub(super) use pending::queue_rule_actions;
pub use pending::{
    count_existing_rule_matches, finish_rule_actions, log_rule_lost_moves, pending_rule_actions,
    queue_existing_rule, replace_rule_tag, retry_rule_actions, saved_rule_accounts,
};

use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use super::tags::{message_tag_keys, write_tag_changes};

const RULES_KEY: &str = "rules";

/// Tries a queued action gets before it is given up on, so a folder deleted
/// since the rule ran can't fail on every sync forever.
const MAX_RULE_ACTION_ATTEMPTS: i64 = 5;

/// Entries the rule log keeps.
const RULE_LOG_SIZE: i64 = 200;

/// One search and everything to do with the new mail it matches. Rules live in
/// `settings`, so backups carry them, and are kept per device: they run on the
/// accounts in `accounts`, or every account when that is empty.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub id: String,
    /// In the search box's syntax, read against what a new message's envelope
    /// says: its body is not here yet. Empty for all new mail.
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub mark_read: bool,
    #[serde(default)]
    pub star: bool,
    /// `""` to leave the message where it is, or `archive`, `trash` or `folder`.
    #[serde(default)]
    pub move_to: String,
    /// The mailbox for `move_to: "folder"`.
    #[serde(default)]
    pub folder: String,
    /// The account of `folder`. Empty for the mail's own account, where a folder
    /// the account lacks skips the move.
    #[serde(default)]
    pub folder_account: String,
    /// The accounts this rule runs on; empty for all of them.
    #[serde(default)]
    pub accounts: Vec<String>,
    /// The mailbox to copy matching mail to, `""` for none, in `copy_account`.
    #[serde(default)]
    pub copy_folder: String,
    #[serde(default)]
    pub copy_account: String,
    /// Kept, but not run.
    #[serde(default)]
    pub disabled: bool,
}

impl Rule {
    fn has_action(&self) -> bool {
        !self.tags.is_empty()
            || self.mark_read
            || self.star
            || !self.move_to.is_empty()
            || !self.copy_folder.is_empty()
    }
}

pub fn rules(conn: &Connection) -> Result<Vec<Rule>> {
    let stored = super::settings_get(conn, &[RULES_KEY.to_string()])?;
    Ok(stored
        .get(RULES_KEY)
        .and_then(|rules| serde_json::from_value(rules.clone()).ok())
        .unwrap_or_default())
}

/// Replace the rules, refusing any that could never apply.
pub fn set_rules(conn: &Connection, rules: Vec<Rule>) -> Result<Vec<Rule>> {
    let mut clean = Vec::with_capacity(rules.len());
    for mut rule in rules {
        rule.query = rule.query.trim().to_string();
        anyhow::ensure!(
            rule.query.is_empty()
                || !crate::search_query::SearchQuery::parse(&rule.query).is_empty(),
            "a rule's search means nothing"
        );
        let tags = serde_json::json!(rule.tags);
        rule.tags = crate::tags::parse_tag_list(Some(&tags)).map_err(anyhow::Error::msg)?;
        rule.folder = rule.folder.trim().to_string();
        match rule.move_to.as_str() {
            "" | "archive" | "trash" => {
                rule.folder.clear();
                rule.folder_account.clear();
            }
            "folder" => anyhow::ensure!(!rule.folder.is_empty(), "a rule needs a folder"),
            other => anyhow::bail!("unknown rule move: {other}"),
        }
        rule.accounts.sort();
        rule.accounts.dedup();
        rule.copy_folder = rule.copy_folder.trim().to_string();
        anyhow::ensure!(
            rule.copy_folder.is_empty() == rule.copy_account.is_empty(),
            "a rule's copy needs an account and a folder"
        );
        anyhow::ensure!(rule.has_action(), "a rule needs an action");
        if rule.id.is_empty() {
            rule.id = uuid::Uuid::new_v4().to_string();
        }
        clean.push(rule);
    }
    super::setting_set(conn, RULES_KEY, &serde_json::json!(clean))?;
    Ok(clean)
}

/// A rule's server work on one message, waiting for the sync that matched it to
/// finish. Queued in the store so that a failed write is retried, not lost.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuleAction {
    pub folder: String,
    pub uid: u32,
    pub uidvalidity: u32,
    pub mark_read: bool,
    pub star: bool,
    pub trash: bool,
    /// The mailbox to move to, `""` for none.
    pub target: String,
    /// Where to copy to first. Kept until all of this is done, so each retry asks
    /// the target again before giving up the source.
    pub copies: Vec<RuleCopy>,
    /// The rules behind this, for the log.
    pub rule_ids: Vec<String>,
    /// Which run by hand queued this, 0 for new mail. Marks whose work a finished
    /// step was: one still in flight when its run is stopped must not touch what
    /// a later run queued for the same message.
    pub run: i64,
}

/// One line of what rules did.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct RuleLogEntry {
    pub at: i64,
    pub rule_id: String,
    pub account: String,
    /// The message's subject; empty for a run by hand, which logs one line.
    pub subject: String,
    pub count: i64,
    pub failed: bool,
    pub detail: String,
}

/// A mailbox on any account.
#[derive(
    Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct RuleCopy {
    pub account: String,
    pub folder: String,
}

/// What a rule's move comes to for one message.
struct RuleDestination {
    trash: bool,
    target: String,
    copy: Option<RuleCopy>,
}

impl RuleAction {
    pub fn moves(&self) -> bool {
        self.trash || !self.target.is_empty()
    }
}

/// Where `rule` sends mail in `folder`: a message already there has nowhere to
/// go, and neither does one bound for a folder this account lacks. A move to
/// another account is a copy there and then Trash here, and what fails between
/// the two is retried from the copy, which looks before it appends.
fn rule_move(
    conn: &Connection,
    account: &str,
    folder: &str,
    rule: &Rule,
) -> Result<Option<RuleDestination>> {
    let target = match rule.move_to.as_str() {
        "trash" => {
            return Ok(Some(RuleDestination {
                trash: true,
                target: String::new(),
                copy: None,
            }));
        }
        "folder" if !rule.folder_account.is_empty() && rule.folder_account != account => {
            return Ok(Some(RuleDestination {
                trash: true,
                target: String::new(),
                copy: Some(RuleCopy {
                    account: rule.folder_account.clone(),
                    folder: crate::mail_model::canon_folder(&rule.folder),
                }),
            }));
        }
        "archive" => crate::engine::cached_archive_folder_from_folders(
            super::get_folders(conn, account)?,
            folder,
        ),
        "folder" => {
            let wanted = crate::mail_model::canon_folder(&rule.folder);
            super::get_folders(conn, account)?
                .into_iter()
                .find(|item| item.name == wanted || item.display_name == rule.folder)
                .map(|item| item.name)
        }
        _ => return Ok(None),
    };
    let Some(target) = target else {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.rules",
            "account={account}: rule {} has no folder to move to here",
            rule.id
        );
        return Ok(None);
    };
    Ok((target != folder).then_some(RuleDestination {
        trash: false,
        target,
        copy: None,
    }))
}

/// Where `rule` copies mail in `folder`, unless that is where it already is.
fn rule_copy(account: &str, folder: &str, rule: &Rule) -> Option<RuleCopy> {
    if rule.copy_folder.is_empty() {
        return None;
    }
    let target = crate::mail_model::canon_folder(&rule.copy_folder);
    (rule.copy_account != account || target != folder).then(|| RuleCopy {
        account: rule.copy_account.clone(),
        folder: target,
    })
}

/// Run the rules over mail that has just arrived in `folder`: never seen on this account, in any folder.
///
/// Only then, so undoing what a rule did by hand sticks. Mail the user wrote or threw away has not
/// arrived in that sense.
///
/// Tags are saved here; what must reach the server first waits in the queue for
/// [`crate::engine::run_rule_actions`]. Several rules can match one message: each adds what it
/// does, and the first to move it decides where.
pub fn apply_rules(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    uids: &[u32],
) -> Result<()> {
    if uids.is_empty() {
        return Ok(());
    }
    // Reading the rules and applying them are one step: see
    // [`super::replace_tag_and_rules`].
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let rules = rules(&tx)?;
    if rules.is_empty() {
        return Ok(());
    }
    let role = super::folder_role(&tx, account, folder)?;
    if matches!(role.as_str(), "sent" | "drafts" | "trash" | "junk") {
        return Ok(());
    }
    let mut actions: BTreeMap<u32, RuleAction> = BTreeMap::new();
    for rule in rules {
        if rule.disabled
            || (!rule.accounts.is_empty() && !rule.accounts.iter().any(|item| item == account))
        {
            continue;
        }
        let matched = if rule.query.is_empty() {
            uids.to_vec()
        } else {
            super::matching_uids(&tx, account, folder, &rule.query, uids)?
        };
        for uid in &matched {
            let subject = rule_message_subject(&tx, account, folder, *uid)?;
            log_rule(&tx, &rule.id, account, &subject, 1, None)?;
        }
        add_rule_effects(
            &tx,
            account,
            folder,
            uidvalidity,
            &rule,
            matched,
            &mut actions,
        )?;
    }
    queue_rule_actions(&tx, account, &actions, 0)?;
    trim_rule_log(&tx)?;
    tx.commit()?;
    Ok(())
}

fn rule_message_subject(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
) -> Result<String> {
    Ok(conn
        .query_row(
            "SELECT COALESCE(subject, '') FROM messages
             WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            params![account, folder, uid],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_default())
}

fn log_rule(
    conn: &Connection,
    rule_id: &str,
    account: &str,
    subject: &str,
    count: usize,
    failure: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO rule_log (at, rule_id, account, subject, count, failed, detail)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            super::now_unix(),
            rule_id,
            account,
            subject,
            count as i64,
            failure.is_some(),
            failure.unwrap_or_default()
        ],
    )?;
    Ok(())
}

fn trim_rule_log(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM rule_log WHERE id <= (SELECT MAX(id) FROM rule_log) - ?1",
        params![RULE_LOG_SIZE],
    )?;
    Ok(())
}

/// What rules did lately, newest first.
pub fn rule_log(conn: &Connection) -> Result<Vec<RuleLogEntry>> {
    let mut stmt = conn.prepare(
        "SELECT at, rule_id, account, subject, count, failed, detail
         FROM rule_log ORDER BY id DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(RuleLogEntry {
            at: row.get(0)?,
            rule_id: row.get(1)?,
            account: row.get(2)?,
            subject: row.get(3)?,
            count: row.get(4)?,
            failed: row.get(5)?,
            detail: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// How many messages each rule run by hand has left.
pub fn running_rules(conn: &Connection) -> Result<BTreeMap<String, i64>> {
    let mut stmt =
        conn.prepare("SELECT rule_ids, COUNT(*) FROM rule_actions WHERE run <> 0 GROUP BY 1")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut running = BTreeMap::new();
    for row in rows {
        let (ids, count) = row?;
        for id in serde_json::from_str::<Vec<String>>(&ids).unwrap_or_default() {
            // AI sorting's queued moves are not rules; its status says how far it is.
            if super::is_ai_rule_id(&id) {
                continue;
            }
            *running.entry(id).or_insert(0) += count;
        }
    }
    Ok(running)
}

/// Stop a rule run by hand: what is done stays done, the rest is dropped.
pub fn stop_rule(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM rule_actions WHERE run <> 0 AND rule_ids = ?1",
        params![serde_json::to_string(&[id])?],
    )?;
    Ok(())
}

/// Take a removed account out of the rules. A rule that still has accounts to
/// run on carries on; one left with none, or that sent mail there, is turned off
/// rather than left to fail, or to start running on every account.
pub fn forget_rule_account(conn: &Connection, id: &str) -> Result<()> {
    let mut stored = rules(conn)?;
    if stored.is_empty() {
        return Ok(());
    }
    for rule in &mut stored {
        let scoped = rule.accounts.iter().any(|item| item == id);
        if scoped && rule.accounts.len() > 1 {
            rule.accounts.retain(|item| item != id);
        }
        if (scoped && rule.accounts.len() == 1 && rule.accounts[0] == id)
            || rule.copy_account == id
            || rule.folder_account == id
        {
            rule.disabled = true;
        }
    }
    super::setting_set(conn, RULES_KEY, &serde_json::json!(stored))?;
    Ok(())
}

/// Save `rule`'s tags on `matched` and add what is left for the server to `actions`.
fn add_rule_effects(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    rule: &Rule,
    matched: Vec<u32>,
    actions: &mut BTreeMap<u32, RuleAction>,
) -> Result<()> {
    if matched.is_empty() {
        return Ok(());
    }
    if !rule.tags.is_empty() {
        let copies = BTreeMap::from([(folder.to_string(), matched.clone())]);
        let keys = message_tag_keys(conn, account, &copies)?;
        write_tag_changes(conn, account, &keys, &rule.tags, &[], true)?;
    }
    let destination = rule_move(conn, account, folder, rule)?;
    let copy = rule_copy(account, folder, rule);
    // Mail with no Message-ID can't be told from its own copies: each would
    // count as new where it lands, and rules pointing at each other would never
    // stop. It stays put, for the actions that can't loop.
    let copying = copy.is_some() || destination.as_ref().is_some_and(|it| it.copy.is_some());
    let ids = if copying {
        super::cached_message_ids(conn, account, folder, &matched)?
    } else {
        Vec::new()
    };
    for (at, uid) in matched.into_iter().enumerate() {
        let identified = ids
            .get(at)
            .is_none_or(|id| id.as_ref().is_some_and(|id| !id.is_empty()));
        if !identified {
            let subject = rule_message_subject(conn, account, folder, uid)?;
            let reason = "this message has no Message-ID, so a rule can't safely copy it";
            log_rule(conn, &rule.id, account, &subject, 1, Some(reason))?;
        }
        let action = actions.entry(uid).or_insert_with(|| RuleAction {
            folder: folder.to_string(),
            uid,
            uidvalidity,
            ..RuleAction::default()
        });
        action.mark_read |= rule.mark_read;
        action.star |= rule.star;
        action.rule_ids.push(rule.id.clone());
        let destination = destination
            .as_ref()
            .filter(|it| identified || it.copy.is_none());
        if let Some(destination) = destination.filter(|_| !action.moves()) {
            action.trash = destination.trash;
            action.target = destination.target.clone();
            action.copies.extend(destination.copy.clone());
        }
        if identified {
            action.copies.extend(copy.clone());
        }
        action.copies.sort();
        action.copies.dedup();
    }
    Ok(())
}

/// Where running a rule by hand looks: the rest of a mailbox is mostly mail
/// already filed, on purpose.
pub const EXISTING_RULE_FOLDER: &str = "INBOX";
