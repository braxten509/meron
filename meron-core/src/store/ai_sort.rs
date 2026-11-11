//! AI sorting's share of the store: its setting, the "hidden until sorted" rows,
//! what this device already had decided, the log, and the daily cap. The API key
//! is never here; see [`crate::engine::EngineHost::load_named_secret`].

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

/// SQL that is true when the `messages` row aliased `alias` belongs in the
/// Inbox views (lists, sender rows, unread count):
/// - not past [`AI_SORT_MAX_AGE_DAYS`] at `now`: the [`ai_sort_too_old`] rule,
///   negated (an unknown (0) or future date counts as new);
/// - not Inbox backfill ([`inbox_backfill_uids`]): mail that was on the server
///   before Meron watched the Inbox, read in only because the sync window
///   reached further back.
///
/// The rest stays on the server untouched and still shows in search, Starred
/// and when opened, just not in the Inbox views.
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
    Ok(super::db::meta_get(conn, &inbox_floor_key(account))?.and_then(|value| {
        let (validity, uid) = value.split_once(':')?;
        (validity.parse::<u32>().ok()? == uidvalidity)
            .then(|| uid.parse::<u32>().ok())
            .flatten()
    }))
}

fn set_inbox_floor(conn: &Connection, account: &str, uidvalidity: u32, uid: u32) -> Result<()> {
    super::db::meta_set(conn, &inbox_floor_key(account), &format!("{uidvalidity}:{uid}"))
}

/// Which of `messages`, a window just read from the server's `folder` (under
/// `uidvalidity`, with UIDNEXT `uid_next`) and about to be cached, are Inbox
/// backfill: not cached yet, and below the Inbox's arrival floor, so on the
/// server before Meron watched it. They show up only because the window
/// reached further back (mail left the Inbox), and the Inbox views leave them
/// out ([`inbox_listed_sql`]) once
/// [`super::upsert_messages_marking`] stores them marked.
///
/// The floor is the UIDNEXT from when Meron started watching: set by the
/// first sync of the folder (or after a UIDVALIDITY change), which shows its
/// whole window as always and so hides nothing; an Inbox synced before the
/// floor existed starts it at the UIDNEXT of its previous sync. Mail that
/// arrives later, including copies a move or an undo puts back (new UIDs), is
/// at or above it. Empty for anything but a real Inbox.
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
    Ok(below.into_iter().filter(|uid| !cached.contains(uid)).collect())
}

/// Cache a window `batch` just read from the server's `folder`, marking the
/// Inbox backfill in it ([`inbox_backfill_uids`]). For the re-reads after a
/// move, copy, send or undo; call before [`super::set_folder_state`] moves the
/// folder's UIDNEXT on. The moved or copied mail itself has new UIDs above the
/// arrival floor, so it always shows.
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

/// Cache `headers` read from the server's `folder` outside a window read (a
/// live search, the starred refresh, a conversation read by Message-ID),
/// marking the Inbox backfill among them as [`upsert_window_messages`] does,
/// against the folder's stored UIDVALIDITY. Those reads reach mail the Inbox
/// never saw arrive; it shows where it was asked for, not in the Inbox views.
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
    for (provider, model) in [(&before.provider, &before.model), (&config.provider, &config.model)] {
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

fn is_done(conn: &Connection, account: &str, identity: &str) -> Result<bool> {
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

/// [`ai_sort_hold`] for a folder other than the Inbox that AI sorting watches
/// (see [`super::ai_sort_watches_folder`]), before `headers` are stored: mail
/// at or past the folder's watch (the first UID not yet looked at; the first
/// look starts it at `uid_next` and holds nothing) that no folder of the
/// account had before, and not a copy of mail the Inbox has or another folder
/// is sorting or sorted (Gmail labels). Mail over the cap shows in the Inbox.
///
/// `read` is the UID range `[from, to)` of which every message on the server
/// was among `headers` or already cached: the watch moves on past it when it
/// starts at or below the watch, and otherwise stays, so what one sync's
/// window couldn't hold is read later (see `engine::ai_sort`).
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
    let Some(from) = super::ai_sort_watch_from(conn, account, folder, uidvalidity, uid_next)?
    else {
        return Ok(Vec::new());
    };
    let mut fresh = Vec::new();
    for header in super::first_seen_messages(conn, account, folder, from, uid_next, headers)? {
        if !super::ai_sort_inbox::ai_sort_seen_in_inbox(conn, account, &header)? {
            fresh.push(header);
        }
    }
    let (read_from, read_to) = read;
    if read_from <= from {
        super::ai_sort_inbox::ai_sort_watch_advance(conn, account, folder, uidvalidity, read_to)?;
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
fn sorted_elsewhere(conn: &Connection, account: &str, folder: &str, message_id: &str) -> Result<bool> {
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
fn queue_inbox_copy_move(
    conn: &Connection,
    account: &str,
    validity: u32,
    inbox_uid: u32,
    log_id: i64,
    target: &str,
    now: i64,
) -> Result<bool> {
    let folder = super::EXISTING_RULE_FOLDER;
    let ruled = super::pending_rule_actions(conn, account, folder)?
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
    super::rules::queue_rule_actions(conn, account, &BTreeMap::from([(inbox_uid, action)]), 0)?;
    conn.execute(
        "INSERT OR REPLACE INTO ai_sort_hidden(account, folder, uid, uidvalidity, hide_until)
         VALUES(?1, ?2, ?3, ?4, ?5)",
        params![account, folder, inbox_uid, validity, now + AI_SORT_MOVE_HOLD_SECS],
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
        if header.ai_sorted || header.uid == 0 || id.is_empty() || ai_sort_too_old(header.date, now) {
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
        if super::meron_header_in_folder(conn, account, header)? {
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
        insert_log(conn, account, folder, uidvalidity, candidate, "", "", "left", &detail)?;
    }
    if !over_cap.is_empty() {
        // Left by the AI: mail from another folder shows in the Inbox.
        super::ai_sort_show_in_inbox(conn, account, folder, uidvalidity, &over_cap)?;
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
    let pending: BTreeSet<u32> = super::pending_rule_actions(conn, account, folder)?
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
    let foldered = super::meron_foldered_uids(conn, account, folder, &uids)?;
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

/// `meta` row with this device's id for AI sorting claims.
const CLAIM_ID_KEY: &str = "ai_sort_claim_id";

/// This device's id for its AI sorting claims (see `imap::claim_for_ai_sort`):
/// eight letters and digits, made once. Kept in `meta`, which no backup and no
/// setup copy carries: two devices must never share one.
pub fn ai_sort_claim_id(conn: &Connection) -> Result<String> {
    let known: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", params![CLAIM_ID_KEY], |row| row.get(0))
        .optional()?;
    if let Some(id) = known.filter(|id| !id.is_empty()) {
        return Ok(id);
    }
    let id: String = uuid::Uuid::new_v4().simple().to_string().chars().take(8).collect();
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
    let until = super::now_unix() + AI_SORT_MOVE_HOLD_SECS;
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
    let pending: BTreeSet<u32> = super::pending_rule_actions(conn, account, folder)?
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
fn conversation_of(
    conn: &Connection,
    account: &str,
    folder: &str,
    candidate: &AiSortCandidate,
) -> Result<String> {
    Ok(super::meron_conversation_key(conn, account, folder, candidate.uid)?
        .unwrap_or_else(|| format!("\u{0}{}", candidate.identity)))
}

/// The UIDs in `folder` of the conversation `key` names.
fn conversation_uids(conn: &Connection, account: &str, folder: &str, key: &str) -> Result<Vec<u32>> {
    let mut stmt = conn.prepare_cached(
        "SELECT uid FROM messages
          WHERE account = ?1 AND lower(COALESCE(thread_key, '')) = ?2 AND folder = ?3 AND uid <> 0",
    )?;
    let rows = stmt.query_map(params![account, key, folder], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Show held-back mail again.
pub fn ai_sort_release(conn: &Connection, account: &str, folder: &str, uids: &[u32]) -> Result<()> {
    let mut stmt = conn.prepare(
        "DELETE FROM ai_sort_hidden WHERE account = ?1 AND folder = ?2 AND uid = ?3",
    )?;
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

/// One log row. `folder` is where the email was when the AI saw it, so Move
/// back finds mail from a folder other than the Inbox.
#[allow(clippy::too_many_arguments)]
fn insert_log(
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
            super::now_unix(),
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
fn trim_log(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM ai_sort_log
         WHERE id <= (SELECT MAX(id) FROM ai_sort_log) - ?1
           AND state <> 'moving' AND alert = 0",
        params![AI_SORT_LOG_SIZE],
    )?;
    Ok(())
}

/// What the AI answered for one email.
#[derive(Clone, Debug, PartialEq)]
pub struct AiSortDecision {
    pub candidate: AiSortCandidate,
    /// The server folder to move to or the Meron folder to file the
    /// conversation in; `None` to leave it in the Inbox.
    pub target: Option<crate::ai_sort::SortTarget>,
    pub why: String,
}

/// Record what the AI decided and queue its moves, all in one step that first
/// checks each email still stands where the AI saw it: still here, under the
/// same UIDVALIDITY, with no rule work queued meanwhile, not decided already.
/// `run` is 0 for new mail, or the run of a "sort Inbox now" job. `in_time` is
/// asked once the transaction holds the store: `false` records nothing and
/// answers `None` (the caller ran out of time). Otherwise answers with the UIDs
/// queued to move and every UID whose decision was accepted (moved or left);
/// what was passed over is shown again.
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
    let current = super::get_folder_state(&tx, account, folder)?.map(|(validity, _)| validity);
    let pending: BTreeSet<u32> = super::pending_rule_actions(&tx, account, folder)?
        .into_iter()
        .map(|action| action.uid)
        .collect();
    let now = super::now_unix();
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
        if super::meron_uid_in_folder(&tx, account, folder, candidate.uid)? {
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
            let Some(name) = super::meron_folder_name(&tx, id)? else {
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
                    super::ai_sort_show_in_inbox(&tx, account, folder, uidvalidity, std::slice::from_ref(candidate))?;
                }
                shown.push(candidate.uid);
                continue;
            };
            let key = super::meron_conversation_key(&tx, account, folder, candidate.uid)?
                .ok_or_else(|| anyhow::anyhow!("the email is no longer cached"))?;
            // The whole conversation moves; nothing is queued for the server
            // but the `$MeronSorted` mark.
            // Mail of the conversation older than 90 days in other folders
            // stays out (it would show there as old cards).
            super::meron_move_in_in(
                &tx,
                account,
                &key,
                id,
                super::MeronFiling::Ai { folder, now },
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
                && let Some(inbox_uid) = super::ai_sort_find_cached(
                    &tx,
                    account,
                    super::EXISTING_RULE_FOLDER,
                    &candidate.message_id,
                )?
                .filter(|_| !candidate.message_id.trim().is_empty())
                && let Some((inbox_validity, _)) =
                    super::get_folder_state(&tx, account, super::EXISTING_RULE_FOLDER)?
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
                if queue_inbox_copy_move(&tx, account, inbox_validity, inbox_uid, id, target, now)? {
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
                super::ai_sort_show_in_inbox(&tx, account, folder, uidvalidity, std::slice::from_ref(candidate))?;
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
        super::rules::queue_rule_actions(
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

/// Of `candidates`, those still open for the AI: still cached here, not held
/// back, not decided already, with no rule work queued.
pub fn ai_sort_still_open(
    conn: &Connection,
    account: &str,
    folder: &str,
    candidates: &[AiSortCandidate],
) -> Result<Vec<AiSortCandidate>> {
    let pending: BTreeSet<u32> = super::pending_rule_actions(conn, account, folder)?
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
                    hidden = super::meron_hidden_sql("m"),
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
    let mut out: Vec<u32> = super::pending_rule_actions(conn, account, folder)?
        .into_iter()
        .filter(|action| {
            action.run == 0
                && wanted.contains(&action.uid)
                && action.rule_ids.iter().any(|id| is_ai_rule_id(id))
        })
        .map(|action| action.uid)
        .collect();
    // And what such a sort filed into a Meron folder, which arrives silently.
    for uid in super::meron_foldered_uids(conn, account, folder, uids)? {
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
        insert_log(conn, account, folder, uidvalidity, candidate, "", "", state, detail)?;
    }
    // Mail from another folder the AI failed on shows in the Inbox. (Not tried
    // again: "Sort Inbox now" covers the Inbox's own mail only.)
    if !folder.eq_ignore_ascii_case("INBOX") {
        let open: Vec<AiSortCandidate> = open.into_iter().cloned().collect();
        super::ai_sort_show_in_inbox(conn, account, folder, uidvalidity, &open)?;
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
pub(super) fn ai_sort_log_failed(
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
    let validity = super::get_folder_state(conn, account, folder)?.map(|(validity, _)| validity);
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
                super::message_header_from_row,
            )
            .optional()?;
        out.extend(header);
    }
    Ok(out)
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

/// A Meron folder's row shows its current name.
const LOG_COLUMNS: &str = "l.id, l.at, l.account, l.subject, l.sender,
    COALESCE(mf.name, l.to_folder), l.reason, l.state, l.detail, l.message_id,
    l.meron_folder_id, l.uid, l.from_folder, l.uidvalidity";
const LOG_FROM: &str = "ai_sort_log l
    LEFT JOIN meron_folders mf ON l.meron_folder_id <> '' AND mf.id = l.meron_folder_id";

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
pub fn ai_sort_existing_candidates(conn: &Connection, account: &str) -> Result<Vec<AiSortCandidate>> {
    let now = super::now_unix();
    let folder = super::EXISTING_RULE_FOLDER;
    let pending: BTreeSet<u32> = super::pending_rule_actions(conn, account, folder)?
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
        hidden = super::meron_hidden_sql("messages"),
    ))?;
    let rows = stmt
        .query_map(params![account, folder, ai_sort_age_cutoff(now), now], |row| {
            Ok(AiSortCandidate {
                uid: row.get(0)?,
                from_name: row.get(1)?,
                from_addr: row.get(2)?,
                subject: row.get(3)?,
                message_id: row.get(4)?,
                identity: row.get(5)?,
            })
        })?
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
    super::ai_lessons_forget_account(conn, id)?;
    super::ai_sort_inbox::ai_sort_inbox_forget_account(conn, id)?;
    let mut config = ai_sort_config(conn)?;
    if config.folders.iter().any(|folder| folder.account == id) {
        config.folders.retain(|folder| folder.account != id);
        if config.folders.is_empty() {
            config.enabled = false;
        }
        super::setting_set(conn, AI_SORT_KEY, &serde_json::to_value(&config)?)?;
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
    super::ai_lessons_forget_meron_folder(conn, id)?;
    let mut config = ai_sort_config(conn)?;
    if config.folders.iter().any(|folder| folder.meron_folder_id == id) {
        config.folders.retain(|folder| folder.meron_folder_id != id);
        if config.folders.is_empty() {
            config.enabled = false;
        }
        super::setting_set(conn, AI_SORT_KEY, &serde_json::to_value(&config)?)?;
    }
    Ok(())
}

/// After a backup restore kept Meron folders under other ids (a folder of the
/// same name was here already): point the setting at the kept ones.
pub fn ai_sort_remap_meron_ids(
    conn: &Connection,
    map: &std::collections::HashMap<String, String>,
) -> Result<()> {
    super::ai_lessons_remap_meron_ids(conn, map)?;
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
        super::setting_set(conn, AI_SORT_KEY, &serde_json::to_value(&config)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn a_device_keeps_one_claim_id_of_its_own() {
        let conn = test_conn();
        let id = ai_sort_claim_id(&conn).unwrap();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
        // Made once: the same on every later ask.
        assert_eq!(ai_sort_claim_id(&conn).unwrap(), id);
        // Another device makes its own.
        assert_ne!(ai_sort_claim_id(&test_conn()).unwrap(), id);
        // Not a setting: settings travel with backups and setup copies.
        let settings: i64 = conn
            .query_row("SELECT COUNT(*) FROM settings WHERE value LIKE ?1", [format!("%{id}%")], |row| row.get(0))
            .unwrap();
        assert_eq!(settings, 0);
    }

    fn insert_unread(conn: &Connection, uid: u32) {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen)
             VALUES('acct', 'INBOX', ?1, ?1, 'Your order', 'Shop', 'orders@shop.example', ?2, 0)",
            // Recent: the Inbox leaves out mail past its age limit.
            params![uid, super::super::now_unix() - 3600],
        )
        .unwrap();
    }

    fn header(uid: u32, message_id: &str) -> MessageHeader {
        MessageHeader {
            uid,
            subject: "Your order".into(),
            from_addr: "orders@shop.example".into(),
            message_id: message_id.into(),
            ..Default::default()
        }
    }

    #[test]
    fn held_mail_is_out_of_the_list_and_counts_until_the_hold_runs_out() {
        let conn = test_conn();
        insert_unread(&conn, 1);
        let now = super::super::now_unix();
        let hide = |until: i64| {
            conn.execute(
                "INSERT OR REPLACE INTO ai_sort_hidden(account, folder, uid, uidvalidity, hide_until)
                 VALUES('acct', 'INBOX', 1, 7, ?1)",
                params![until],
            )
            .unwrap();
        };
        let shown = || {
            let filter = super::super::RecentFilter::default();
            let (page, _) =
                super::super::get_recent_page(&conn, "acct", "INBOX", 50, None, filter).unwrap();
            let (senders, _) =
                super::super::get_sender_page(&conn, "acct", "INBOX", 50, None, filter).unwrap();
            let unread = super::super::get_folder_unread(&conn, "acct", "INBOX").unwrap();
            (page.len(), senders.len(), unread)
        };

        hide(now + 1_000);
        assert_eq!(shown(), (0, 0, 0));
        hide(now - 1);
        assert_eq!(shown(), (1, 1, 1));
    }

    #[test]
    fn only_new_unsorted_inbox_mail_without_rule_work_is_sorted() {
        let conn = test_conn();
        let marked = MessageHeader {
            ai_sorted: true,
            ..header(2, "b@shop.example")
        };
        // Moved back once: decided on this device already.
        conn.execute(
            "INSERT INTO ai_sort_done(account, identity, at)
             VALUES('acct', 'message-id:c@shop.example', 1)",
            [],
        )
        .unwrap();
        let headers = [
            header(1, "a@shop.example"),
            marked,
            header(3, "c@shop.example"),
            header(4, "d@shop.example"),
        ];

        // The first sync of a folder has no new mail to give.
        let first =
            super::super::first_seen_messages(&conn, "acct", "INBOX", 0, 10, &headers).unwrap();
        assert!(first.is_empty());
        assert!(ai_sort_hold(&conn, "acct", "Archive", 7, &headers, 100).unwrap().is_empty());

        let held = ai_sort_hold(&conn, "acct", "INBOX", 7, &headers, 100).unwrap();
        let uids: Vec<u32> = held.iter().map(|candidate| candidate.uid).collect();
        assert_eq!(uids, [1, 4]);

        // A rule queued work for 4: it is the rule's, not the AI's, and shows again.
        let action = RuleAction {
            folder: "INBOX".into(),
            uid: 4,
            uidvalidity: 7,
            mark_read: true,
            rule_ids: vec!["rule".into()],
            ..RuleAction::default()
        };
        super::super::rules::queue_rule_actions(&conn, "acct", &BTreeMap::from([(4, action)]), 0)
            .unwrap();
        let kept = ai_sort_skip_rule_actions(&conn, "acct", "INBOX", held).unwrap();
        assert_eq!(kept.iter().map(|candidate| candidate.uid).collect::<Vec<_>>(), [1]);
        let hidden: i64 = conn
            .query_row("SELECT COUNT(*) FROM ai_sort_hidden WHERE uid = 4", [], |row| row.get(0))
            .unwrap();
        assert_eq!(hidden, 0);
    }
}
