//! Implements the local Meron folder commands.
//!
//! Desktop uses `meronFolders.<name>` and the phone uses `mail.<name>` (mapped by
//! [`mobile_command`]). Meron folders exist only on this device: every command
//! but `markAllRead` is a store read or write and never touches a server.
//!
//! | command       | params                                                        |
//! |---------------|---------------------------------------------------------------|
//! | `list`        | `{}`                                                          |
//! | `create`      | `{"name"}`                                                    |
//! | `rename`      | `{"id","name"}`                                               |
//! | `setNotify`   | `{"id","enabled"}` (new mail AI sorting files there notifies) |
//! | `delete`      | `{"id"}`                                                      |
//! | `moveIn`      | `{"folder_id","thread_ids":[…]}` (into Archive: see [`move_with_server`]) |
//! | `moveOut`     | `{"thread_ids":[…]}` (out of Archive: see [`move_with_server`]) |
//! | `threads`     | `{"folder_id","query"?,"filter"?,"attachments"?,"limit"?,"before_cursor"?}` |
//! | `markAllRead` | `{"folder_id"}` (sets `\Seen` on the servers; see [`mark_all_read`]) |

mod filing;
pub use filing::{
    entry_before_action, mark_all_read, unmark_before_action, unmark_before_action_blocking,
};
use filing::{move_in, move_out};
mod archive;
pub use archive::{
    archive_card_of_copies, archive_filing_keys, archive_meron_only, is_archive_target,
    move_with_server,
};
mod archive_marks;
pub use archive_marks::{
    file_after_archive, prepare_archive_marks, prepare_archive_marks_blocking,
    restore_archive_card_marks, restore_marks_after_failed_archive,
};

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};

use crate::engine::Engine;
use crate::meron_marks::MarkJob;
use crate::store;

/// Cards a `threads` page holds when the caller doesn't say.
const DEFAULT_LIMIT: usize = 50;

/// The command behind a phone method name (`mail.meronFolders` → `list`).
pub fn mobile_command(method: &str) -> Option<&'static str> {
    Some(match method {
        "mail.meronFolders" => "list",
        "mail.createMeronFolder" => "create",
        "mail.renameMeronFolder" => "rename",
        "mail.setMeronFolderNotify" => "setNotify",
        "mail.deleteMeronFolder" => "delete",
        "mail.moveToMeronFolder" => "moveIn",
        "mail.moveOutOfMeronFolder" => "moveOut",
        "mail.meronFolderThreads" => "threads",
        "mail.markMeronFolderRead" => "markAllRead",
        _ => return None,
    })
}

/// What the phone's command answers before the core is set up.
pub fn mobile_stub(command: &str) -> Value {
    match command {
        "list" | "rename" | "setNotify" => json!({ "folders": [] }),
        "create" => json!({ "folder": Value::Null, "folders": [] }),
        "delete" => json!({ "ok": true, "released": 0, "folders": [], "folder_counts": [] }),
        "moveIn" => json!({
            "ok": true, "moved": 0, "skipped": [], "folders": [], "changes": [],
            "folder_counts": [],
        }),
        "moveOut" => json!({
            "ok": true, "released": 0, "unarchived": [], "folders": [], "folder_counts": [],
        }),
        "threads" => json!({ "threads": [], "folder_unread": 0 }),
        "markAllRead" => json!({
            "ok": true, "updated": 0, "failures": [], "folders": [], "folder_counts": [],
        }),
        _ => json!({ "ok": true }),
    }
}

fn string_parameter<'a>(params: &'a Value, key: &str) -> &'a str {
    params
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
}

fn required<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    let value = string_parameter(params, key);
    anyhow::ensure!(!value.is_empty(), "missing {key}");
    Ok(value)
}

fn thread_ids(params: &Value) -> Vec<String> {
    params
        .get("thread_ids")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Every Meron folder, A–Z, the built-in Archive last, with its unread count
/// and how many conversations it holds.
pub fn folders_json(conn: &Connection) -> Result<Value> {
    let unread = store::meron_unread_by_folder(conn)?;
    let counts = store::meron_folder_counts(conn)?;
    Ok(Value::Array(
        store::list_meron_folders(conn)?
            .iter()
            .map(|folder| folder_value(folder, &unread, &counts))
            .collect(),
    ))
}

/// One folder as the commands answer it; `builtin` is `"archive"` for the
/// built-in Archive, else null.
fn folder_value(
    folder: &store::MeronFolder,
    unread: &std::collections::HashMap<String, u32>,
    counts: &std::collections::HashMap<String, u32>,
) -> Value {
    json!({
        "id": folder.id,
        "name": folder.name,
        "unread": unread.get(&folder.id).copied().unwrap_or(0),
        "count": counts.get(&folder.id).copied().unwrap_or(0),
        "created_at": folder.created_at,
        "notify": folder.notify,
        "builtin": store::meron_is_builtin(&folder.id).then_some("archive"),
    })
}

/// Unread counts of the mailboxes a change touched, in the shape mutation
/// results carry.
fn folder_counts(conn: &Connection, touched: &BTreeSet<(String, String)>) -> Result<Value> {
    let mut out = Vec::new();
    for (account, folder) in touched {
        out.push(json!({
            "account_id": account,
            "folder_id": folder,
            "unread": store::get_folder_unread(conn, account, folder)?,
        }));
    }
    Ok(Value::Array(out))
}

/// The account's engine (`mail`, `rss`, `local`), if the account exists.
fn account_engine(conn: &Connection, account: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT engine FROM accounts WHERE id = ?1",
            params![account],
            |row| row.get(0),
        )
        .optional()?)
}

/// A thread id a Meron folder can take: a mail conversation of a server
/// account, not a sender row, a feed item or a local folder's mail. Answers
/// with the parsed id and its conversation key, or why not.
fn resolve_thread(
    conn: &Connection,
    thread_id: &str,
) -> Result<std::result::Result<(crate::protocol::ParsedThreadId, String), &'static str>> {
    if thread_id.contains("#rss#") {
        return Ok(Err("not_allowed"));
    }
    let Some(parsed) = crate::protocol::parse_thread_id(thread_id) else {
        return Ok(Err("invalid"));
    };
    if store::is_local_account(&parsed.account)
        || store::sender_key_addr(&parsed.thread_key).is_some()
    {
        return Ok(Err("not_allowed"));
    }
    match account_engine(conn, &parsed.account)?.as_deref() {
        None => return Ok(Err("not_found")),
        Some("rss") | Some("local") => return Ok(Err("not_allowed")),
        Some(_) => {}
    }
    let key = store::meron_thread_key(
        conn,
        &parsed.account,
        &parsed.folder,
        &parsed.thread_key,
        parsed.uid,
    )?;
    Ok(match key {
        Some(key) => Ok((parsed, key)),
        None => Err("not_found"),
    })
}

/// Run one store-only command (all but `markAllRead`).
pub fn dispatch(conn: &Connection, command: &str, params: &Value) -> Result<Value> {
    dispatch_with_marks(conn, command, params).map(|(result, _)| result)
}

/// [`dispatch`], then the Meron-folder marks its filing changes call for are
/// written in the background (see [`crate::meron_marks`]).
pub fn dispatch_marked(
    engine: &Arc<Engine>,
    conn: &Connection,
    command: &str,
    params: &Value,
) -> Result<Value> {
    let (result, jobs) = dispatch_with_marks(conn, command, params)?;
    crate::meron_marks::spawn_writes(engine, jobs);
    crate::engine::ai_sort::push_lesson_tags(engine);
    Ok(result)
}

/// [`dispatch`], answering also with the mark writes the change calls for.
pub fn dispatch_with_marks(
    conn: &Connection,
    command: &str,
    params: &Value,
) -> Result<(Value, Vec<MarkJob>)> {
    match command {
        "moveIn" => return move_in(conn, params),
        "moveOut" => return move_out(conn, params),
        "delete" => {
            let id = required(params, "id")?;
            anyhow::ensure!(
                !store::meron_is_builtin(id),
                "The Archive folder can't be changed."
            );
            let entries: Vec<String> = conn
                .prepare("SELECT id FROM meron_folder_entries WHERE folder_id = ?1")?
                .query_map(params![id], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let jobs = entries
                .iter()
                .flat_map(|entry| crate::meron_marks::job_for_released_entry(conn, entry))
                .collect();
            let released = store::delete_meron_folder(conn, id)?;
            return Ok((
                json!({
                    "ok": true,
                    "released": released.released,
                    "folders": folders_json(conn)?,
                    "folder_counts": folder_counts(conn, &released.touched)?,
                }),
                jobs,
            ));
        }
        _ => {}
    }
    dispatch_plain(conn, command, params).map(|result| (result, Vec::new()))
}

fn dispatch_plain(conn: &Connection, command: &str, params: &Value) -> Result<Value> {
    match command {
        "list" => Ok(json!({ "folders": folders_json(conn)? })),
        "create" => {
            let folder = store::create_meron_folder(conn, string_parameter(params, "name"))?;
            Ok(json!({ "folder": folder_json(conn, &folder)?, "folders": folders_json(conn)? }))
        }
        "rename" => {
            let folder = store::rename_meron_folder(
                conn,
                required(params, "id")?,
                string_parameter(params, "name"),
            )?;
            Ok(json!({ "folder": folder_json(conn, &folder)?, "folders": folders_json(conn)? }))
        }
        "setNotify" => {
            let id = required(params, "id")?;
            let enabled = params
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or_else(|| anyhow::anyhow!("missing enabled"))?;
            anyhow::ensure!(
                store::set_meron_folder_notify(conn, id, enabled)?,
                "That Meron folder is gone."
            );
            Ok(json!({ "ok": true, "folders": folders_json(conn)? }))
        }
        "threads" => {
            let folder_id = required(params, "folder_id")?;
            anyhow::ensure!(
                store::meron_folder_exists(conn, folder_id)?,
                "That Meron folder is gone."
            );
            let limit = params
                .get("limit")
                .and_then(Value::as_u64)
                .filter(|limit| *limit > 0)
                .map(|limit| limit as usize)
                .unwrap_or(DEFAULT_LIMIT);
            let filter = string_parameter(params, "filter");
            crate::mail_model::meron_folder_page(
                conn,
                folder_id,
                string_parameter(params, "query"),
                if filter.eq_ignore_ascii_case("all") {
                    ""
                } else {
                    filter
                },
                params
                    .get("attachments")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                limit,
                params
                    .get("before_cursor")
                    .and_then(Value::as_str)
                    .filter(|cursor| !cursor.is_empty()),
            )
        }
        "markAllRead" => anyhow::bail!("markAllRead needs the mail engine"),
        other => anyhow::bail!("unknown Meron folder command: {other}"),
    }
}

fn folder_json(conn: &Connection, folder: &store::MeronFolder) -> Result<Value> {
    let unread = store::meron_unread_by_folder(conn)?;
    let counts = store::meron_folder_counts(conn)?;
    Ok(folder_value(folder, &unread, &counts))
}

// ---- Archive: the built-in Archive Meron folder -----------------------------

/// One conversation in the built-in Archive to take back to the Inbox.
struct Unarchive {
    account: String,
    card: store::MeronCardRelease,
    archive_folder: Option<String>,
    uids: Vec<u32>,
}

/// Releases that failed and could not be queued in the store either (entry id
/// to its Meron folder then). Queued by the next [`retry_pending_releases`].
static UNQUEUED_RELEASES: std::sync::Mutex<BTreeMap<String, String>> =
    std::sync::Mutex::new(BTreeMap::new());

/// Where a released card was, for the action's answer: an Undo hands these
/// back (`meron_folder_id`, `meron_source`) so the card goes back in.
pub struct ReleasedCard {
    pub folder_id: String,
    pub source: String,
}

impl ReleasedCard {
    /// Add `meron_folder_id` and `meron_source` to an action's answer.
    pub fn annotate(released: &Option<ReleasedCard>, result: &mut Value) {
        if let (Some(card), Some(object)) = (released, result.as_object_mut()) {
            object.insert("meron_folder_id".to_string(), json!(card.folder_id));
            object.insert("meron_source".to_string(), json!(card.source));
        }
    }
}

/// The card left its Meron folder by a normal server action: take its
/// conversation out of the entry. Tried twice; a release that still fails is
/// queued (the whole entry then, when the card was all of it), and every mail
/// sync retries the queue ([`retry_pending_releases`]).
pub fn release_after_action(
    conn: &Connection,
    card: Option<store::MeronCardRelease>,
) -> Option<ReleasedCard> {
    let card = card?;
    let released = ReleasedCard {
        folder_id: card.folder_id.clone(),
        source: card.source.clone(),
    };
    let Err(err) =
        store::meron_release_card(conn, &card).or_else(|_| store::meron_release_card(conn, &card))
    else {
        return Some(released);
    };
    crate::mlog!(
        crate::log::Level::Warn,
        "mail.meronFolders",
        "could not release a conversation from its Meron folder, retrying on the next sync: {err:#}"
    );
    // Only a card that was the whole entry can be retried as an entry release:
    // that would take the entry's other conversations out too.
    let whole = store::meron_entry_members(conn, &card.entry_id).is_ok_and(|members| {
        members
            .iter()
            .all(|member| card.rows.contains(&(member.folder.clone(), member.uid)))
    });
    if whole {
        UNQUEUED_RELEASES
            .lock()
            .unwrap()
            .insert(card.entry_id.clone(), card.folder_id.clone());
        queue_unqueued_releases(conn);
    }
    Some(released)
}

/// Undo of an action [`release_after_action`] took a card out for: the copies
/// `(folder, uids)` just moved back go into Meron folder `folder_id` again.
/// Best effort, logged.
pub fn refile_after_undo(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
    root: &str,
    folder_id: &str,
    source: &str,
) -> BTreeSet<(String, String)> {
    match store::meron_refile(conn, account, folder, uids, root, folder_id, source) {
        Ok(touched) => touched
            .into_iter()
            .map(|folder| (account.to_string(), folder))
            .collect(),
        Err(err) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.meronFolders",
                "could not put an undone conversation back in its Meron folder: {err:#}"
            );
            BTreeSet::new()
        }
    }
}

/// Describe the additional local state to restore when undoing a move.
///
/// What an Undo of an archive, delete or move asks for besides the move back (`"undo": true` on the move): show
/// the copies in the Inbox again (`show_in_inbox`, for mail that was shown there from its own folder), or put
/// them back in the Meron folder the action took them out of (`meron_folder_id`, `meron_source` from the
/// action's answer). Shared by the phone's `mail.move` and the desktop's `messages.move`.
pub struct UndoRestore {
    pub show_in_inbox: bool,
    pub meron_folder_id: Option<String>,
    pub meron_source: String,
}

impl UndoRestore {
    /// `None` unless the params say `"undo": true`.
    pub fn from_params(params: &Value) -> Option<Self> {
        if !params.get("undo").and_then(Value::as_bool).unwrap_or(false) {
            return None;
        }
        Some(Self {
            show_in_inbox: params
                .get("show_in_inbox")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            meron_folder_id: params
                .get("meron_folder_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string),
            meron_source: params
                .get("meron_source")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_string(),
        })
    }

    /// After the move back stored the copies `uids` of `folder` (`root`: the
    /// thread key the action named). Answers the mark writes the refiling
    /// calls for, for the caller to start.
    pub fn apply(
        &self,
        conn: &Connection,
        account: &str,
        root: &str,
        folder: &str,
        uids: &[u32],
    ) -> Vec<MarkJob> {
        if let Some(folder_id) = &self.meron_folder_id {
            refile_after_undo(
                conn,
                account,
                folder,
                uids,
                root,
                folder_id,
                &self.meron_source,
            );
            // Back in its Meron folder: its mark-folder copies get the mark
            // again.
            let rows: Vec<(String, u32)> =
                uids.iter().map(|uid| (folder.to_string(), *uid)).collect();
            let by_folder = crate::meron_marks::uids_to_mark(conn, account, &rows, Some(folder_id));
            return crate::meron_marks::jobs_for(account, by_folder, Some(folder_id));
        }
        if self.show_in_inbox
            && let Err(err) =
                store::ai_sort_inbox_show_again_after_undo(conn, account, folder, uids)
        {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.meronFolders",
                "account={account}: could not show undone mail in the Inbox again: {err:#}"
            );
        }
        Vec::new()
    }
}

/// Move the in-memory releases into the store's queue, which survives a
/// restart; on failure they stay in memory for the next try.
fn queue_unqueued_releases(conn: &Connection) {
    let mut unqueued = UNQUEUED_RELEASES.lock().unwrap();
    if unqueued.is_empty() {
        return;
    }
    match store::meron_queue_releases(conn, &unqueued) {
        Ok(()) => unqueued.clear(),
        Err(err) => crate::mlog!(
            crate::log::Level::Warn,
            "mail.meronFolders",
            "could not queue {} Meron folder release(s): {err:#}",
            unqueued.len()
        ),
    }
}

/// Retry the Meron folder releases that failed earlier, in this run or before
/// a restart. Best effort and cheap when nothing waits; mail syncs call it.
pub fn retry_pending_releases(conn: &Connection) {
    queue_unqueued_releases(conn);
    if let Err(err) = store::meron_retry_pending_releases(conn) {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.meronFolders",
            "retrying Meron folder releases failed: {err:#}"
        );
    }
}
