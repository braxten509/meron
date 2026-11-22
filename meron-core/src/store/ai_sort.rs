//! AI sorting's share of the store: its setting, the "hidden until sorted" rows,
//! what this device already had decided, the log, and the daily cap. The API key
//! is never here; see [`crate::engine::EngineHost::load_named_secret`].

mod claims;
pub use claims::{
    ai_sort_cap_left, ai_sort_claim_id, ai_sort_clear_folder, ai_sort_count_taken_over,
    ai_sort_group_conversations, ai_sort_hidden_until, ai_sort_hold, ai_sort_hold_folder,
    ai_sort_left_to_other, ai_sort_release, ai_sort_reuse_for_inbox, ai_sort_skip_meron_foldered,
    ai_sort_skip_rule_actions,
};
use claims::{conversation_of, is_done, queue_inbox_copy_move};
mod decisions;
pub use decisions::{
    ai_sort_find_cached, ai_sort_give_up, ai_sort_queued_new_moves, ai_sort_record_decisions,
    ai_sort_still_open,
};
use decisions::{insert_log, trim_log};
mod history;
pub(super) use history::ai_sort_log_failed;
pub use history::{
    ai_sort_existing_candidates, ai_sort_forget_account, ai_sort_forget_meron_folder, ai_sort_log,
    ai_sort_log_entry, ai_sort_mark_moved, ai_sort_remap_meron_ids, ai_sort_set_log_state,
    ai_sort_stop_run, ai_sort_take_alerts, is_ai_rule_id,
};

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::imap::MessageHeader;

use super::RuleAction;

/// The `settings` key. In `settings`, so backups carry it like `rules`.
pub const AI_SORT_KEY: &str = "ai_sort";

/// What every queued AI move's `rule_ids` entry starts with:
/// `ai-sort:<log id>`, so a move marks exactly its own log row.
pub const AI_RULE_ID: &str = "ai-sort";

/// Folders the AI may choose from.
pub const AI_SORT_MAX_FOLDERS: usize = 25;
/// Characters a folder's note keeps.
pub const AI_SORT_MAX_NOTE: usize = 120;
/// Entries the log keeps, like the rule log.
const AI_SORT_LOG_SIZE: i64 = 200;
/// Emails automatic sorting sends per device per day.
pub const AI_SORT_DAILY_CAP: i64 = 1_000;
/// How long new Inbox mail stays out of the list while the AI decides.
pub const AI_SORT_HOLD_SECS: i64 = 20;
/// How long mail the AI decided to move stays out of the list while the move
/// runs: the sync's rule budget.
pub const AI_SORT_MOVE_HOLD_SECS: i64 = 60;
/// Mail older than this many days (by its date) is never held, sent to the AI
/// or shown in the Inbox from another folder: it stays where it is.
pub const AI_SORT_MAX_AGE_DAYS: i64 = 90;

/// Whether mail dated `date` (Unix seconds: its Date header, or INTERNALDATE
/// when that was missing) is past [`AI_SORT_MAX_AGE_DAYS`]. An unknown (0) or
/// future date doesn't count as old.
pub fn ai_sort_too_old(date: i64, now: i64) -> bool {
    date > 0 && date <= now && date < ai_sort_age_cutoff(now)
}

/// The oldest date AI sorting still takes, at `now`.
pub fn ai_sort_age_cutoff(now: i64) -> i64 {
    now - AI_SORT_MAX_AGE_DAYS * 86_400
}

/// SQL that is true when the `messages` row aliased `alias` belongs in the Inbox views (lists, sender rows, unread count):
///
/// - not past [`AI_SORT_MAX_AGE_DAYS`] at `now`: the [`ai_sort_too_old`] rule,
///   negated (an unknown (0) or future date counts as new);
/// - not Inbox backfill ([`inbox_backfill_uids`]): mail that was on the server
///   before Meron watched the Inbox, read in only because the sync window
///   reached further back.
///
/// The rest stays on the server untouched and still shows in search, Starred and when opened, just not
/// in the Inbox views.
pub fn inbox_listed_sql(alias: &str, now: i64) -> String {
    format!(
        "((COALESCE({alias}.date, 0) <= 0 OR {alias}.date >= {cutoff})
          AND COALESCE(json_extract({alias}.json, '$.{INBOX_BACKFILL_KEY}'), 0) = 0)",
        cutoff = ai_sort_age_cutoff(now)
    )
}

/// Whether `folder` of `account` is a real Inbox, whose lists and counts keep
/// to [`inbox_listed_sql`]. The local archive's folders (one may be called
/// INBOX) keep everything.
pub fn inbox_hides_old(account: &str, folder: &str) -> bool {
    folder.eq_ignore_ascii_case("INBOX") && !super::is_local_account(account)
}

/// The key in a `messages` row's `json` that marks it Inbox backfill. A key in
/// the catch-all column, not a column: header re-reads `json_patch` it, which
/// keeps keys they don't carry, and no schema change is needed.
pub(super) const INBOX_BACKFILL_KEY: &str = "inbox_backfill";

/// `meta` key holding the account's Inbox arrival floor, `<uidvalidity>:<uid>`:
/// every Inbox UID at or above it arrived while Meron watched the Inbox.
fn inbox_floor_key(account: &str) -> String {
    format!("inbox_arrival_floor:{account}")
}

fn inbox_floor(conn: &Connection, account: &str, uidvalidity: u32) -> Result<Option<u32>> {
    Ok(
        super::db::meta_get(conn, &inbox_floor_key(account))?.and_then(|value| {
            let (validity, uid) = value.split_once(':')?;
            (validity.parse::<u32>().ok()? == uidvalidity)
                .then(|| uid.parse::<u32>().ok())
                .flatten()
        }),
    )
}

fn set_inbox_floor(conn: &Connection, account: &str, uidvalidity: u32, uid: u32) -> Result<()> {
    super::db::meta_set(
        conn,
        &inbox_floor_key(account),
        &format!("{uidvalidity}:{uid}"),
    )
}

/// Identify which recently fetched Inbox headers belong to an older backfill window.
///
/// Which of `messages`, a window just read from the server's `folder` (under `uidvalidity`, with UIDNEXT
/// `uid_next`) and about to be cached, are Inbox backfill: not cached yet, and below the Inbox's arrival
/// floor, so on the server before Meron watched it.
///
/// They show up only because the window reached further back (mail left the Inbox), and the Inbox views
/// leave them out ([`inbox_listed_sql`]) once [`super::upsert_messages_marking`] stores them marked.
///
/// The floor is the UIDNEXT from when Meron started watching: set by the first sync of the folder (or after
/// a UIDVALIDITY change), which shows its whole window as always and so hides nothing; an Inbox synced
/// before the floor existed starts it at the UIDNEXT of its previous sync. Mail that arrives later,
/// including copies a move or an undo puts back (new UIDs), is at or above it.
///
/// Empty for anything but a real Inbox.
pub fn inbox_backfill_uids(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    uid_next: u32,
    messages: &[MessageHeader],
) -> Result<Vec<u32>> {
    if !inbox_hides_old(account, folder) || uidvalidity == 0 {
        return Ok(Vec::new());
    }
    let floor = match super::get_folder_state(conn, account, folder)? {
        Some((validity, prior_next)) if validity == uidvalidity => {
            match inbox_floor(conn, account, uidvalidity)? {
                Some(floor) => floor,
                None => {
                    set_inbox_floor(conn, account, uidvalidity, prior_next)?;
                    prior_next
                }
            }
        }
        // First look, or new UIDs: this window shows as it always did.
        _ => {
            set_inbox_floor(conn, account, uidvalidity, uid_next)?;
            return Ok(Vec::new());
        }
    };
    let below: Vec<u32> = messages
        .iter()
        .map(|message| message.uid)
        .filter(|uid| *uid != 0 && *uid < floor)
        .collect();
    if below.is_empty() {
        return Ok(below);
    }
    let cached = super::cached_uids_in_folder(conn, account, folder, &below)?;
    Ok(below
        .into_iter()
        .filter(|uid| !cached.contains(uid))
        .collect())
}

/// Cache a window `batch` just read from the server's `folder`, marking the Inbox backfill in it
/// ([`inbox_backfill_uids`]). For the re-reads after a move, copy, send or undo; call before
/// [`super::set_folder_state`] moves the folder's UIDNEXT on. The moved or copied mail itself has
/// new UIDs above the arrival floor, so it always shows.
pub fn upsert_window_messages(
    conn: &Connection,
    account: &str,
    folder: &str,
    batch: &crate::imap::RecentBatch,
) -> Result<()> {
    let backfill = inbox_backfill_uids(
        conn,
        account,
        folder,
        batch.uidvalidity,
        batch.uid_next,
        &batch.messages,
    )?;
    super::upsert_messages_marking(conn, account, folder, &batch.messages, &backfill)
}

/// Cache `headers` read from the server's `folder` outside a window read (a live search, the
/// starred refresh, a conversation read by Message-ID), marking the Inbox backfill among them as
/// [`upsert_window_messages`] does, against the folder's stored UIDVALIDITY. Those reads reach mail
/// the Inbox never saw arrive; it shows where it was asked for, not in the Inbox views.
pub fn upsert_read_messages(
    conn: &Connection,
    account: &str,
    folder: &str,
    headers: &[MessageHeader],
) -> Result<()> {
    let backfill = match super::get_folder_state(conn, account, folder)? {
        Some((validity, uid_next)) if inbox_hides_old(account, folder) => {
            inbox_backfill_uids(conn, account, folder, validity, uid_next, headers)?
        }
        _ => Vec::new(),
    };
    super::upsert_messages_marking(conn, account, folder, headers, &backfill)
}

/// The message's identity, as `observed_mail_identities` files it, from a
/// cached `messages` row.
pub(super) const IDENTITY_SQL: &str = "CASE
    WHEN json_extract(json, '$.gmail_msg_id') IS NOT NULL THEN 'gmail:' || json_extract(json, '$.gmail_msg_id')
    WHEN COALESCE(json_extract(json, '$.message_id'), '') <> '' THEN 'message-id:' || lower(json_extract(json, '$.message_id'))
    ELSE 'uid:' || lower(folder) || ':' || uid END";

/// A folder the AI may sort into, with what belongs there: a server folder of
/// one account, or (with `meron_folder_id` set, `account` and `folder` empty)
/// a Meron folder, which takes new mail from every account's Inbox.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AiSortFolder {
    #[serde(default)]
    pub account: String,
    /// The server's folder name.
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub note: String,
    /// The Meron folder (`mf_…`) this entry stands for. Older settings and
    /// backups have none; older apps drop such an entry.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub meron_folder_id: String,
}

impl AiSortFolder {
    pub fn is_meron(&self) -> bool {
        !self.meron_folder_id.is_empty()
    }
}

/// The `ai_sort` setting. There is no default model: sorting can't be on
/// until one is picked.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AiSortConfig {
    #[serde(default)]
    pub enabled: bool,
    /// `openrouter` or `tinfoil`.
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    /// `headers` (sender and subject) or `start` (also the start of the email).
    #[serde(default = "default_reads")]
    pub reads: String,
    #[serde(default)]
    pub folders: Vec<AiSortFolder>,
    /// The last few models picked on each service, newest first, so the
    /// picker can offer them and a switch back to a service brings its model
    /// back. Kept by the core: what a client sends here is ignored.
    #[serde(default)]
    pub recent_models: std::collections::BTreeMap<String, Vec<String>>,
}

/// Models remembered per service in [`AiSortConfig::recent_models`].
const RECENT_MODELS: usize = 3;

fn default_provider() -> String {
    "tinfoil".to_string()
}

fn default_reads() -> String {
    "headers".to_string()
}

impl Default for AiSortConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_provider(),
            model: String::new(),
            reads: default_reads(),
            folders: Vec::new(),
            recent_models: Default::default(),
        }
    }
}

impl AiSortConfig {
    /// The chosen server folders of `account`.
    pub fn folders_of(&self, account: &str) -> Vec<&AiSortFolder> {
        self.folders
            .iter()
            .filter(|folder| !folder.is_meron() && folder.account == account)
            .collect()
    }

    /// The accounts that have chosen server folders, in order, once each.
    pub fn accounts(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        self.folders
            .iter()
            .filter(|folder| !folder.is_meron())
            .filter(|folder| seen.insert(folder.account.clone()))
            .map(|folder| folder.account.clone())
            .collect()
    }

    /// The chosen Meron folders' ids, in order.
    pub fn meron_ids(&self) -> Vec<String> {
        self.folders
            .iter()
            .filter(|folder| folder.is_meron())
            .map(|folder| folder.meron_folder_id.clone())
            .collect()
    }
}

/// The accounts AI sorting can work for: mail accounts, not feeds, not the
/// local folders. With a Meron folder chosen, every one of them is sorted.
pub fn sort_accounts(conn: &Connection) -> Result<Vec<String>> {
    Ok(super::list_accounts(conn)?
        .iter()
        .filter(|account| {
            account["engine"].as_str() != Some("rss") && account["provider"].as_str() != Some("rss")
        })
        .filter_map(|account| account["id"].as_str().map(str::to_string))
        .filter(|id| !super::is_local_account(id))
        .collect())
}

/// Whether the setting has a Meron folder that still exists.
pub fn ai_sort_has_live_meron(conn: &Connection, config: &AiSortConfig) -> Result<bool> {
    for id in config.meron_ids() {
        if super::meron_folder_exists(conn, &id)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn ai_sort_config(conn: &Connection) -> Result<AiSortConfig> {
    let stored = super::settings_get(conn, &[AI_SORT_KEY.to_string()])?;
    Ok(stored
        .get(AI_SORT_KEY)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default())
}

/// A system folder, or one mail can't be sorted into.
fn is_system_folder(conn: &Connection, account: &str, folder: &str) -> Result<bool> {
    if folder.eq_ignore_ascii_case("INBOX") {
        return Ok(true);
    }
    let role = super::folder_role(conn, account, folder)?;
    Ok(matches!(
        role.as_str(),
        "inbox" | "sent" | "drafts" | "trash" | "junk"
    ))
}

/// Clean `config` the way [`set_ai_sort_config`] saves it: known provider and
/// reads, trimmed notes, no system or unknown-account folders, no repeats, at
/// most [`AI_SORT_MAX_FOLDERS`]. Sorting stays off without a model or folder.
pub fn clean_ai_sort_config(conn: &Connection, mut config: AiSortConfig) -> Result<AiSortConfig> {
    if !matches!(config.provider.as_str(), "openrouter" | "tinfoil") {
        config.provider = default_provider();
    }
    if !matches!(config.reads.as_str(), "headers" | "start") {
        config.reads = default_reads();
    }
    config.model = config.model.trim().to_string();
    let accounts: BTreeSet<String> = sort_accounts(conn)?.into_iter().collect();
    let mut seen = BTreeSet::new();
    let mut folders = Vec::new();
    for mut folder in std::mem::take(&mut config.folders) {
        folder.note = folder
            .note
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(AI_SORT_MAX_NOTE)
            .collect();
        folder.meron_folder_id = folder.meron_folder_id.trim().to_string();
        if folder.is_meron() {
            folder.account.clear();
            folder.folder.clear();
            // The built-in Archive is no sorting target.
            if super::meron_is_builtin(&folder.meron_folder_id)
                || !super::meron_folder_exists(conn, &folder.meron_folder_id)?
                || !seen.insert(("meron".to_string(), folder.meron_folder_id.clone()))
            {
                continue;
            }
            folders.push(folder);
            if folders.len() == AI_SORT_MAX_FOLDERS {
                break;
            }
            continue;
        }
        folder.account = folder.account.trim().to_string();
        folder.folder = crate::mail_model::canon_folder(folder.folder.trim());
        if folder.folder.is_empty()
            || !accounts.contains(&folder.account)
            || is_system_folder(conn, &folder.account, &folder.folder)?
            || !seen.insert((folder.account.clone(), folder.folder.clone()))
        {
            continue;
        }
        folders.push(folder);
        if folders.len() == AI_SORT_MAX_FOLDERS {
            break;
        }
    }
    config.folders = folders;
    if config.model.is_empty() || config.folders.is_empty() {
        config.enabled = false;
    }
    Ok(config)
}

/// Save the setting, cleaned (see [`clean_ai_sort_config`]).
pub fn set_ai_sort_config(conn: &Connection, config: AiSortConfig) -> Result<AiSortConfig> {
    let before = ai_sort_config(conn)?;
    let mut config = clean_ai_sort_config(conn, config)?;
    // The stored list, with the model in use before this feature counted too.
    let mut recent = before.recent_models;
    for (provider, model) in [
        (&before.provider, &before.model),
        (&config.provider, &config.model),
    ] {
        if model.is_empty() {
            continue;
        }
        let list = recent.entry(provider.clone()).or_default();
        list.retain(|id| id != model);
        list.insert(0, model.clone());
        list.truncate(RECENT_MODELS);
    }
    config.recent_models = recent;
    super::setting_set(conn, AI_SORT_KEY, &serde_json::to_value(&config)?)?;
    Ok(config)
}

/// An email the AI is to look at.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiSortCandidate {
    pub uid: u32,
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub message_id: String,
    pub identity: String,
}

impl AiSortCandidate {
    fn from_header(header: &MessageHeader, folder: &str) -> Self {
        Self {
            uid: header.uid,
            from_name: header.from_name.clone(),
            from_addr: header.from_addr.clone(),
            subject: header.subject.clone(),
            message_id: header.message_id.clone(),
            identity: super::messages::message_identity(header, folder),
        }
    }

    fn sender(&self) -> String {
        match (self.from_name.trim(), self.from_addr.trim()) {
            ("", addr) => addr.to_string(),
            (name, "") => name.to_string(),
            (name, addr) => format!("{name} <{addr}>"),
        }
    }
}

/// `meta` row with this device's id for AI sorting claims.
const CLAIM_ID_KEY: &str = "ai_sort_claim_id";

/// What the AI answered for one email.
#[derive(Clone, Debug, PartialEq)]
pub struct AiSortDecision {
    pub candidate: AiSortCandidate,
    /// The server folder to move to or the Meron folder to file the
    /// conversation in; `None` to leave it in the Inbox.
    pub target: Option<crate::ai_sort::SortTarget>,
    pub why: String,
}

/// What [`ai_sort_record_decisions`] took on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiSortRecorded {
    /// Queued to move.
    pub moving: Vec<u32>,
    /// Decided (moved, filed or left): these get the `$MeronSorted` mark.
    pub accepted: Vec<u32>,
    /// Filed into a Meron folder, every new email of each filed conversation
    /// (the one the AI decided on is also in `accepted`): arrives silently.
    /// Later replies to the conversation are not here and still alert.
    pub filed: Vec<u32>,
    /// `filed`, each with the Meron folder it went into (for notifications).
    pub filed_to: Vec<(u32, String)>,
    /// Inbox UIDs queued to move: the Inbox copies (Gmail) of mail from
    /// another folder the AI picked a server folder for.
    pub inbox_moving: Vec<u32>,
}

/// One line of what the AI did.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct AiSortLogEntry {
    pub id: i64,
    pub at: i64,
    pub account: String,
    pub subject: String,
    pub sender: String,
    pub to_folder: String,
    pub reason: String,
    /// `moving`, `moved`, `left`, `slow`, `failed` or `moved_back`.
    pub state: String,
    pub detail: String,
    /// The Meron folder the AI filed into (its current name is `to_folder`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub meron_folder_id: String,
    #[serde(skip)]
    pub message_id: String,
    #[serde(skip)]
    pub uid: u32,
    /// Where the email was when the AI saw it (empty on rows from before this
    /// was kept: the Inbox).
    #[serde(skip)]
    pub from_folder: String,
    #[serde(skip)]
    pub uidvalidity: u32,
}

/// A Meron folder's row shows its current name.
const LOG_COLUMNS: &str = "l.id, l.at, l.account, l.subject, l.sender,
    COALESCE(mf.name, l.to_folder), l.reason, l.state, l.detail, l.message_id,
    l.meron_folder_id, l.uid, l.from_folder, l.uidvalidity";
const LOG_FROM: &str = "ai_sort_log l
    LEFT JOIN meron_folders mf ON l.meron_folder_id <> '' AND mf.id = l.meron_folder_id";

#[cfg(test)]
mod tests;
