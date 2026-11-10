//! The Meron folder commands, shared by the desktop sidecar
//! (`meronFolders.<name>`) and the phone (`mail.<name>` names, mapped by
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

fn str_param<'a>(params: &'a Value, key: &str) -> &'a str {
    params.get(key).and_then(Value::as_str).unwrap_or_default().trim()
}

fn required<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    let value = str_param(params, key);
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
            anyhow::ensure!(!store::meron_is_builtin(id), "The Archive folder can't be changed.");
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
            let folder = store::create_meron_folder(conn, str_param(params, "name"))?;
            Ok(json!({ "folder": folder_json(conn, &folder)?, "folders": folders_json(conn)? }))
        }
        "rename" => {
            let folder =
                store::rename_meron_folder(conn, required(params, "id")?, str_param(params, "name"))?;
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
            anyhow::ensure!(store::meron_folder_exists(conn, folder_id)?, "That Meron folder is gone.");
            let limit = params
                .get("limit")
                .and_then(Value::as_u64)
                .filter(|limit| *limit > 0)
                .map(|limit| limit as usize)
                .unwrap_or(DEFAULT_LIMIT);
            let filter = str_param(params, "filter");
            crate::mail_model::meron_folder_page(
                conn,
                folder_id,
                str_param(params, "query"),
                if filter.eq_ignore_ascii_case("all") { "" } else { filter },
                params.get("attachments").and_then(Value::as_bool).unwrap_or(false),
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

/// A sender row stands for every conversation its person has in the row's
/// mailbox: swap it for one id per cached message there, each of which
/// [`resolve_thread`] checks and turns into its conversation. Other ids pass
/// through unchanged.
fn expand_sender_rows(conn: &Connection, ids: Vec<String>) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(ids.len());
    for thread_id in ids {
        let sender = crate::protocol::parse_thread_id(&thread_id).and_then(|parsed| {
            store::sender_key_addr(&parsed.thread_key).map(|addr| (parsed, addr))
        });
        let Some((parsed, addr)) = sender else {
            out.push(thread_id);
            continue;
        };
        let headers = store::sender_folder_headers(conn, &parsed.account, &parsed.folder, &addr)?;
        out.extend(headers.iter().map(|header| {
            let folder = if header.folder.is_empty() {
                &parsed.folder
            } else {
                &header.folder
            };
            crate::mail_model::format_thread_id(
                &parsed.account,
                folder,
                &format!("uid:{}", header.uid),
            )
        }));
    }
    Ok(out)
}

/// Move whole conversations into a Meron folder (or from one to another), in
/// one transaction. Each change says where the conversation was, so Undo can
/// put it back: `previous_folder_id` null means "out of Meron folders".
fn move_in(conn: &Connection, params: &Value) -> Result<(Value, Vec<MarkJob>)> {
    let folder_id = required(params, "folder_id")?;
    anyhow::ensure!(store::meron_folder_exists(conn, folder_id)?, "That Meron folder is gone.");
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let mut moved = 0;
    let mut skipped = Vec::new();
    let mut changes = Vec::new();
    let mut touched = BTreeSet::new();
    // Subject branches of one root are one conversation: moved once.
    let mut done: BTreeSet<(String, String)> = BTreeSet::new();
    let mut filed_entries: BTreeSet<String> = BTreeSet::new();
    for thread_id in expand_sender_rows(&tx, thread_ids(params))? {
        let (parsed, key) = match resolve_thread(&tx, &thread_id)? {
            Ok(found) => found,
            Err(reason) => {
                skipped.push(json!({ "thread_id": thread_id, "reason": reason }));
                continue;
            }
        };
        let first = done.insert((parsed.account.clone(), key.clone()));
        let before = store::meron_entry_of_key(&tx, &parsed.account, &key)?;
        // Already filed in this folder (a sender row can reach such mail, from
        // Trash say): nothing moves, so no count and no change for Undo to
        // take back out.
        if before
            .as_ref()
            .is_some_and(|(_, folder)| folder == folder_id)
        {
            continue;
        }
        let result =
            store::meron_move_in_in(&tx, &parsed.account, &key, folder_id, store::MeronFiling::User)?;
        filed_entries.insert(result.entry_id.clone());
        touched.insert((parsed.account.clone(), parsed.folder.clone()));
        for folder in result.touched {
            touched.insert((parsed.account.clone(), folder));
        }
        // One change per conversation: a later branch would read the folder
        // the first one just moved it into and report no previous folder,
        // which Undo would take as "move it out".
        if first {
            moved += 1;
            changes.push(json!({
                "thread_id": thread_id,
                "account_id": parsed.account,
                "source_folder": parsed.folder,
                "removed": true,
                "previous_folder_id": before.map(|(_, folder)| folder).filter(|folder| folder != folder_id),
            }));
        }
    }
    tx.commit()?;
    let jobs = filed_entries
        .iter()
        .flat_map(|entry| crate::meron_marks::job_for_filed_entry(conn, entry, folder_id))
        .collect();
    Ok((
        json!({
            "ok": true,
            "moved": moved,
            "skipped": skipped,
            "folders": folders_json(conn)?,
            "changes": changes,
            "folder_counts": folder_counts(conn, &touched)?,
        }),
        jobs,
    ))
}

/// Take whole conversations out of their Meron folders: they show where their
/// mail is again.
fn move_out(conn: &Connection, params: &Value) -> Result<(Value, Vec<MarkJob>)> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let mut released = 0;
    let mut jobs = Vec::new();
    let mut touched = BTreeSet::new();
    for thread_id in thread_ids(params) {
        let Some(parsed) = crate::protocol::parse_thread_id(&thread_id) else {
            continue;
        };
        touched.insert((parsed.account.clone(), parsed.folder.clone()));
        if let Some((entry, _)) = store::meron_entry_of_thread(
            &tx,
            &parsed.account,
            &parsed.folder,
            &parsed.thread_key,
            parsed.uid,
        )? {
            jobs.extend(crate::meron_marks::job_for_released_entry(&tx, &entry));
            let result = store::meron_release_entry_in(&tx, &entry)?;
            released += result.released;
            touched.extend(result.touched);
        }
    }
    tx.commit()?;
    Ok((
        json!({
            "ok": true,
            "released": released,
            "folders": folders_json(conn)?,
            "folder_counts": folder_counts(conn, &touched)?,
        }),
        jobs,
    ))
}

/// "Mark all read" inside a Meron folder: `\Seen` on the servers for every
/// unseen message of its conversations (outside Drafts, Trash and Junk), then
/// the store. Per mailbox; one that fails is reported and the rest go on.
pub async fn mark_all_read(engine: &Arc<Engine>, params: &Value) -> Result<Value> {
    let folder_id = required(params, "folder_id")?.to_string();
    let targets: BTreeMap<String, BTreeMap<String, Vec<u32>>> = {
        let db = engine.db.lock().unwrap();
        anyhow::ensure!(store::meron_folder_exists(&db, &folder_id)?, "That Meron folder is gone.");
        store::meron_folder_unseen_uids(&db, &folder_id)?
    };
    let mut updated = 0;
    let mut failures = Vec::new();
    let mut touched = BTreeSet::new();
    for (account, folders) in targets {
        for (folder, uids) in folders {
            let write = engine
                .with_flag_write_session(
                    &account,
                    |session| {
                        let folder = folder.clone();
                        Box::pin(async move {
                            crate::imap::prepare_flag_update(session, &folder).await
                        })
                    },
                    |session| {
                        let uids = uids.clone();
                        Box::pin(async move { crate::imap::store_seen(session, &uids, true).await })
                    },
                )
                .await;
            match write {
                Ok(_) => {
                    store::meron_mark_seen(&engine.db.lock().unwrap(), &account, &folder, &uids)?;
                    updated += uids.len();
                    touched.insert((account.clone(), folder.clone()));
                }
                Err(err) => failures.push(json!({
                    "account_id": account,
                    "folder_id": folder,
                    "message": format!("{err:#}"),
                })),
            }
        }
    }
    let db = engine.db.lock().unwrap();
    Ok(json!({
        "ok": failures.is_empty(),
        "updated": updated,
        "failures": failures,
        "folders": folders_json(&db)?,
        "folder_counts": folder_counts(&db, &touched)?,
    }))
}

/// Before a whole-conversation Archive, Move to or Delete: the Meron-folder
/// card the conversation is, if any, to [`release_after_action`] once the
/// server step succeeded. From any folder, Drafts, Trash and Junk included
/// (owner decision 9.5: the action makes the conversation leave). Only that
/// card's conversation leaves: an entry holding more conversations keeps the
/// others. `None` for a single email, a sender row, the local folders, or an
/// account with nothing in Meron folders.
pub fn entry_before_action(
    conn: &Connection,
    account: &str,
    folder: &str,
    root: &str,
    uid: Option<u32>,
    whole_conversation: bool,
) -> Option<store::MeronCardRelease> {
    if !whole_conversation
        || root.is_empty()
        || store::sender_key_addr(root).is_some()
        || store::is_local_account(account)
        || !store::meron_account_has_entries(conn, account).unwrap_or(false)
    {
        return None;
    }
    store::meron_card_release(conn, account, folder, root, uid)
        .ok()
        .flatten()
}

/// Before the server step of an action [`entry_before_action`] found a card
/// for: take the Meron-folder marks off the card's Inbox copies, so the mail
/// doesn't carry them to its new folder and other devices take it out too.
/// Awaited (within a few seconds) so the marks go before the mail moves; best
/// effort.
pub async fn unmark_before_action(
    engine: &Arc<Engine>,
    account: &str,
    card: &Option<store::MeronCardRelease>,
) {
    let Some(card) = card else {
        return;
    };
    let rows: Vec<(String, u32)> = card.rows.iter().cloned().collect();
    let uids = crate::meron_marks::uids_marked(&engine.db.lock().unwrap(), account, &rows);
    crate::meron_marks::write_all(engine, crate::meron_marks::jobs_for(account, uids, None)).await;
}

/// [`unmark_before_action`] for the phone's blocking commands.
pub fn unmark_before_action_blocking(
    engine: &Arc<Engine>,
    account: &str,
    card: &Option<store::MeronCardRelease>,
) {
    if card.is_none() {
        return;
    }
    let _ = crate::ffi::engine_block_on(async {
        unmark_before_action(engine, account, card).await;
        anyhow::Ok(())
    });
}

// ---- Archive: the built-in Archive Meron folder -----------------------------

/// Before archiving (a move into the account's archive mailbox): the
/// conversation keys of the moved copies, read before the move, when the
/// action files them into the built-in Archive. `None` when it doesn't: not
/// a whole conversation (same rule as [`entry_before_action`]), a sender row,
/// a local or feed account, or nothing cached.
pub fn archive_filing_keys(
    conn: &Connection,
    account: &str,
    folder: &str,
    root: &str,
    uids: &[u32],
    whole_conversation: bool,
) -> Option<BTreeSet<String>> {
    if !whole_conversation
        || uids.is_empty()
        || root.is_empty()
        || store::sender_key_addr(root).is_some()
        || store::is_local_account(account)
        || store::meron_archive_mailbox(conn, account).as_deref() == Some(folder)
    {
        return None;
    }
    match account_engine(conn, account).ok().flatten().as_deref() {
        None | Some("rss") | Some("local") => return None,
        Some(_) => {}
    }
    let keys: BTreeSet<String> = uids
        .iter()
        .filter_map(|uid| store::meron_conversation_key(conn, account, folder, *uid).ok().flatten())
        .collect();
    (!keys.is_empty()).then_some(keys)
}

/// Whether `target` is where archiving puts mail: an archive-role mailbox.
pub fn is_archive_target(conn: &Connection, account: &str, target: &str) -> bool {
    !store::is_local_account(account)
        && (store::folder_role(conn, account, target).is_ok_and(|role| role == "archive")
            || store::meron_archive_mailbox(conn, account).as_deref() == Some(target))
}

/// The built-in Archive card the copies `uids` of archive mailbox `folder`
/// belong to, read before exactly these copies move out (an Undo of an
/// archive): the conversation leaves Archive with them.
pub fn archive_card_of_copies(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
) -> Option<store::MeronCardRelease> {
    if !is_archive_target(conn, account, folder) {
        return None;
    }
    let first = *uids.first()?;
    store::meron_card_release(conn, account, folder, "", Some(first))
        .ok()
        .flatten()
        .filter(|card| card.folder_id == store::MERON_ARCHIVE_FOLDER_ID)
}

/// `moveIn` and `moveOut`, which touch the servers for the built-in Archive
/// (the desktop's `meronFolders.moveIn`/`.moveOut`, the phone's
/// `mail.moveToMeronFolder`/`mail.moveOutOfMeronFolder`):
///
/// - `moveOut` of a conversation in Archive un-archives it: its archived
///   copies lose the Archive's mark and move back to the Inbox, and it leaves
///   Archive. The answer's `unarchived` names it in the Inbox (`thread_id`,
///   `account_id`), for an Undo. Other ids go out store-only, as before.
/// - `moveIn` into Archive (the Undo of that) archives again: the cached Inbox
///   copies move to the archive mailbox and the conversation is filed. With
///   no Inbox copy cached, it is filed store-only.
/// - Every other `moveIn` is store-only, as before.
///
/// The store lock is only held between the server steps.
pub async fn move_with_server(engine: &Arc<Engine>, command: &str, params: &Value) -> Result<Value> {
    match command {
        "moveOut" => move_out_with_server(engine, params).await,
        "moveIn" if str_param(params, "folder_id") == store::MERON_ARCHIVE_FOLDER_ID => {
            archive_again(engine, params).await
        }
        _ => {
            let (result, jobs) = dispatch_with_marks(&engine.db.lock().unwrap(), command, params)?;
            crate::meron_marks::spawn_writes(engine, jobs);
            Ok(result)
        }
    }
}

/// Archive of a whole conversation on an account with no archive mailbox:
/// Meron-only. No server move; the conversation is filed into the built-in
/// Archive here (hidden from the Inbox view, its mail staying where it is),
/// and its Inbox copies get the Archive's mark for the other devices (the
/// returned jobs, for the caller to start). `None` when this doesn't apply:
/// a single email (a `uid:` id), a sender row, a feed or local account; the
/// caller keeps its "no archive folder" answer then. The answer has `moved`,
/// `meron_only: true`, `folder` and `thread_id` (unchanged: the mail didn't
/// move) and, when it came from another Meron folder, `meron_folder_id` and
/// `meron_source` for an Undo to put it back there.
pub fn archive_meron_only(conn: &Connection, thread_id: &str) -> Result<Option<(Value, Vec<MarkJob>)>> {
    let Some(parsed) = crate::protocol::parse_thread_id(thread_id) else {
        return Ok(None);
    };
    if parsed.uid.is_some() || parsed.thread_key.starts_with("uid:") {
        return Ok(None);
    }
    if resolve_thread(conn, thread_id)?.is_err() {
        return Ok(None);
    }
    let previous = store::meron_entry_of_thread(conn, &parsed.account, &parsed.folder, &parsed.thread_key, None)?
        .map(|(entry, folder)| {
            let source: String = conn
                .query_row(
                    "SELECT source FROM meron_folder_entries WHERE id = ?1",
                    params![entry],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "user".to_string());
            (folder, source)
        })
        .filter(|(folder, _)| folder != store::MERON_ARCHIVE_FOLDER_ID);
    let (mut result, jobs) = move_in(
        conn,
        &json!({ "folder_id": store::MERON_ARCHIVE_FOLDER_ID, "thread_ids": [thread_id] }),
    )?;
    if let Some(object) = result.as_object_mut() {
        object.insert("meron_only".to_string(), json!(true));
        object.insert("folder".to_string(), json!(parsed.folder));
        object.insert("thread_id".to_string(), json!(thread_id));
        if let Some((folder, source)) = previous {
            object.insert("meron_folder_id".to_string(), json!(folder));
            object.insert("meron_source".to_string(), json!(source));
        }
    }
    Ok(Some((result, jobs)))
}

/// One conversation in the built-in Archive to take back to the Inbox.
struct Unarchive {
    account: String,
    card: store::MeronCardRelease,
    archive_folder: Option<String>,
    uids: Vec<u32>,
}

async fn move_out_with_server(engine: &Arc<Engine>, params: &Value) -> Result<Value> {
    // Which ids are Archive's: read once, before anything moves.
    let mut plain: Vec<String> = Vec::new();
    let mut unarchive: Vec<Unarchive> = Vec::new();
    {
        let db = engine.db.lock().unwrap();
        for thread_id in thread_ids(params) {
            let Some(parsed) = crate::protocol::parse_thread_id(&thread_id) else {
                plain.push(thread_id);
                continue;
            };
            let in_archive = store::meron_entry_of_thread(
                &db,
                &parsed.account,
                &parsed.folder,
                &parsed.thread_key,
                parsed.uid,
            )?
            .is_some_and(|(_, folder)| folder == store::MERON_ARCHIVE_FOLDER_ID);
            let card = in_archive
                .then(|| {
                    store::meron_card_release(
                        &db,
                        &parsed.account,
                        &parsed.folder,
                        &parsed.thread_key,
                        parsed.uid,
                    )
                })
                .transpose()?
                .flatten();
            let Some(card) = card else {
                plain.push(thread_id);
                continue;
            };
            if unarchive.iter().any(|known| known.card.entry_id == card.entry_id && known.card.rows == card.rows) {
                continue;
            }
            let archive_folder = store::meron_archive_mailbox(&db, &parsed.account);
            let uids: Vec<u32> = card
                .rows
                .iter()
                .filter(|(folder, _)| archive_folder.as_deref() == Some(folder.as_str()))
                .map(|(_, uid)| *uid)
                .collect();
            unarchive.push(Unarchive {
                account: parsed.account.clone(),
                card,
                archive_folder,
                uids,
            });
        }
    }
    let mut released = 0;
    let mut touched: BTreeSet<(String, String)> = BTreeSet::new();
    let mut unarchived = Vec::new();
    let mut failures = Vec::new();
    for item in unarchive {
        let mut inbox_uids: Vec<u32> = Vec::new();
        // Nothing in the archive mailbox (archived Meron-only, on an account
        // with no archive mailbox): no server move. Its marked copies (in the
        // Inbox) lose the Archive's mark, and it leaves Archive where it is.
        if item.uids.is_empty() {
            let rows: Vec<(String, u32)> = item.card.rows.iter().cloned().collect();
            let marked = crate::meron_marks::uids_marked(&engine.db.lock().unwrap(), &item.account, &rows);
            crate::meron_marks::write_all(engine, crate::meron_marks::jobs_for(&item.account, marked, None)).await;
            inbox_uids = item
                .card
                .rows
                .iter()
                .filter(|(folder, _)| folder.eq_ignore_ascii_case("INBOX"))
                .map(|(_, uid)| *uid)
                .collect();
        }
        if let Some(archive) = item.archive_folder.as_deref().filter(|_| !item.uids.is_empty()) {
            // The Archive's mark first, so the copies don't carry it back.
            crate::meron_marks::write_all(
                engine,
                vec![MarkJob {
                    account: item.account.clone(),
                    folder: archive.to_string(),
                    uids: item.uids.clone(),
                    target: None,
                }],
            )
            .await;
            match crate::engine::move_within_account(engine, &item.account, archive, "INBOX", &item.uids).await {
                Ok(copies) => {
                    inbox_uids = copies.uids;
                    touched.insert((item.account.clone(), archive.to_string()));
                    touched.insert((item.account.clone(), "INBOX".to_string()));
                }
                Err(err) => {
                    // Still archived: it keeps its place and its mark.
                    crate::meron_marks::spawn_writes(
                        engine,
                        vec![MarkJob {
                            account: item.account.clone(),
                            folder: archive.to_string(),
                            uids: item.uids.clone(),
                            target: Some(store::MERON_ARCHIVE_FOLDER_ID.to_string()),
                        }],
                    );
                    failures.push(json!({
                        "account_id": item.account,
                        "message": format!("{err:#}"),
                    }));
                    continue;
                }
            }
        }
        let db = engine.db.lock().unwrap();
        let result = store::meron_release_card(&db, &item.card)?;
        released += result.released.max(1);
        touched.extend(result.touched);
        // Named by its first copy back in the Inbox, for an Undo.
        let inbox_thread = inbox_uids.first().and_then(|uid| {
            store::meron_row_header(&db, &item.account, "INBOX", *uid)
                .ok()
                .flatten()
                .map(|header| {
                    crate::mail_model::format_thread_id(&item.account, "INBOX", &header.thread_key)
                })
        });
        if let Some(thread_id) = inbox_thread {
            unarchived.push(json!({
                "thread_id": thread_id,
                "account_id": item.account,
                "target_uids": inbox_uids,
            }));
        }
    }
    crate::meron_marks::flush_archive_unmarks(engine);
    // The rest leave their Meron folders store-only, as always.
    let mut plain_released = 0;
    if !plain.is_empty() {
        let (result, jobs) = move_out(&engine.db.lock().unwrap(), &json!({ "thread_ids": plain }))?;
        crate::meron_marks::spawn_writes(engine, jobs);
        plain_released = result.get("released").and_then(Value::as_u64).unwrap_or(0) as usize;
        if let Some(counts) = result.get("folder_counts").and_then(Value::as_array) {
            for count in counts {
                if let (Some(account), Some(folder)) = (
                    count.get("account_id").and_then(Value::as_str),
                    count.get("folder_id").and_then(Value::as_str),
                ) {
                    touched.insert((account.to_string(), folder.to_string()));
                }
            }
        }
    }
    let db = engine.db.lock().unwrap();
    Ok(json!({
        "ok": failures.is_empty(),
        "released": released + plain_released,
        "unarchived": unarchived,
        "failures": failures,
        "folders": folders_json(&db)?,
        "folder_counts": folder_counts(&db, &touched)?,
    }))
}

/// `moveIn` into the built-in Archive: archive the conversations again (the
/// Undo of "Move to Inbox").
async fn archive_again(engine: &Arc<Engine>, params: &Value) -> Result<Value> {
    struct Job {
        thread_id: String,
        account: String,
        root: String,
        inbox_uids: Vec<u32>,
        archive_folder: String,
        keys: BTreeSet<String>,
        card: Option<store::MeronCardRelease>,
    }
    let mut jobs: Vec<Job> = Vec::new();
    let mut store_only: Vec<String> = Vec::new();
    let mut skipped = Vec::new();
    {
        let db = engine.db.lock().unwrap();
        for thread_id in thread_ids(params) {
            let (parsed, key) = match resolve_thread(&db, &thread_id)? {
                Ok(found) => found,
                Err(reason) => {
                    skipped.push(json!({ "thread_id": thread_id, "reason": reason }));
                    continue;
                }
            };
            // No archive mailbox: archiving is Meron-only, the mail stays
            // where it is and is filed (and marked) here.
            let Some(archive_folder) = store::meron_archive_mailbox(&db, &parsed.account) else {
                store_only.push(thread_id);
                continue;
            };
            let inbox_uids = if parsed.folder.eq_ignore_ascii_case("INBOX") {
                match parsed.uid {
                    Some(uid) => vec![uid],
                    None => store::resolve_message_uids(
                        &db,
                        &parsed.account,
                        "INBOX",
                        &parsed.thread_key,
                        parsed.subject_filter.as_deref(),
                        None,
                        &[],
                    )?,
                }
            } else {
                Vec::new()
            };
            if inbox_uids.is_empty() {
                store_only.push(thread_id);
                continue;
            }
            let mut keys = archive_filing_keys(
                &db,
                &parsed.account,
                "INBOX",
                &parsed.thread_key,
                &inbox_uids,
                true,
            )
            .unwrap_or_default();
            keys.insert(key);
            let card = entry_before_action(
                &db,
                &parsed.account,
                "INBOX",
                &parsed.thread_key,
                parsed.uid,
                true,
            );
            jobs.push(Job {
                thread_id,
                account: parsed.account,
                root: parsed.thread_key,
                inbox_uids,
                archive_folder,
                keys,
                card,
            });
        }
    }
    let mut moved = 0;
    let mut changes = Vec::new();
    let mut touched: BTreeSet<(String, String)> = BTreeSet::new();
    let mut failures = Vec::new();
    for job in jobs {
        prepare_archive_marks(engine, &job.account, &job.card, "INBOX", &job.inbox_uids).await;
        let copies = match crate::engine::move_within_account(
            engine,
            &job.account,
            "INBOX",
            &job.archive_folder,
            &job.inbox_uids,
        )
        .await
        {
            Ok(copies) => copies,
            Err(err) => {
                restore_marks_after_failed_archive(
                    engine,
                    &engine.db.lock().unwrap(),
                    &job.account,
                    &job.card,
                    "INBOX",
                    &job.inbox_uids,
                );
                failures.push(json!({ "thread_id": job.thread_id, "message": format!("{err:#}") }));
                continue;
            }
        };
        let db = engine.db.lock().unwrap();
        release_after_action(&db, job.card);
        touched.extend(file_after_archive(&db, &job.account, &job.archive_folder, &copies.uids, &job.keys));
        touched.insert((job.account.clone(), "INBOX".to_string()));
        touched.insert((job.account.clone(), job.archive_folder.clone()));
        moved += 1;
        let archived_thread = copies.uids.first().and_then(|uid| {
            store::meron_row_header(&db, &job.account, &job.archive_folder, *uid)
                .ok()
                .flatten()
                .map(|header| header.thread_key)
        });
        changes.push(json!({
            "thread_id": job.thread_id,
            "account_id": job.account,
            "source_folder": "INBOX",
            "removed": true,
            "previous_folder_id": Value::Null,
            "archived_thread_id": crate::mail_model::format_thread_id(
                &job.account,
                &job.archive_folder,
                archived_thread.as_deref().unwrap_or(&job.root),
            ),
        }));
    }
    crate::meron_marks::flush_archive_unmarks(engine);
    // No Inbox copy cached: filed store-only, its archived copies marked.
    if !store_only.is_empty() {
        let (result, mark_jobs) = move_in(
            &engine.db.lock().unwrap(),
            &json!({ "folder_id": store::MERON_ARCHIVE_FOLDER_ID, "thread_ids": store_only }),
        )?;
        crate::meron_marks::spawn_writes(engine, mark_jobs);
        moved += result.get("moved").and_then(Value::as_u64).unwrap_or(0) as usize;
        if let Some(more) = result.get("changes").and_then(Value::as_array) {
            changes.extend(more.iter().cloned());
        }
        if let Some(more) = result.get("skipped").and_then(Value::as_array) {
            skipped.extend(more.iter().cloned());
        }
        if let Some(counts) = result.get("folder_counts").and_then(Value::as_array) {
            for count in counts {
                if let (Some(account), Some(folder)) = (
                    count.get("account_id").and_then(Value::as_str),
                    count.get("folder_id").and_then(Value::as_str),
                ) {
                    touched.insert((account.to_string(), folder.to_string()));
                }
            }
        }
    }
    let db = engine.db.lock().unwrap();
    Ok(json!({
        "ok": failures.is_empty(),
        "moved": moved,
        "skipped": skipped,
        "failures": failures,
        "folders": folders_json(&db)?,
        "changes": changes,
        "folder_counts": folder_counts(&db, &touched)?,
    }))
}

/// Before the server move of an archive [`archive_filing_keys`] files: the
/// moved copies get the built-in Archive's mark in their source folder (a
/// MOVE carries keywords along), and the card's other mark-folder copies
/// that aren't moved lose theirs. Awaited (a few seconds at most); best
/// effort.
pub async fn prepare_archive_marks(
    engine: &Arc<Engine>,
    account: &str,
    card: &Option<store::MeronCardRelease>,
    source_folder: &str,
    uids: &[u32],
) {
    let mut jobs = vec![MarkJob {
        account: account.to_string(),
        folder: source_folder.to_string(),
        uids: uids.to_vec(),
        target: Some(store::MERON_ARCHIVE_FOLDER_ID.to_string()),
    }];
    if let Some(card) = card {
        let rows: Vec<(String, u32)> = card
            .rows
            .iter()
            .filter(|(folder, uid)| !(folder == source_folder && uids.contains(uid)))
            .cloned()
            .collect();
        let marked = crate::meron_marks::uids_marked(&engine.db.lock().unwrap(), account, &rows);
        jobs.extend(crate::meron_marks::jobs_for(account, marked, None));
    }
    crate::meron_marks::write_all(engine, jobs).await;
}

/// [`prepare_archive_marks`] for the phone's blocking commands.
pub fn prepare_archive_marks_blocking(
    engine: &Arc<Engine>,
    account: &str,
    card: &Option<store::MeronCardRelease>,
    source_folder: &str,
    uids: &[u32],
) {
    let _ = crate::ffi::engine_block_on(async {
        prepare_archive_marks(engine, account, card, source_folder, uids).await;
        anyhow::Ok(())
    });
}

/// The server move after [`prepare_archive_marks`] failed: put the marks
/// back as they were, in the background. A card that was in a Meron folder
/// gets that folder's mark again; else the moved copies lose the Archive's.
pub fn restore_marks_after_failed_archive(
    engine: &Arc<Engine>,
    conn: &Connection,
    account: &str,
    card: &Option<store::MeronCardRelease>,
    source_folder: &str,
    uids: &[u32],
) {
    // First the unmark, so a filed card's mark below is the latest wish.
    let mut jobs = vec![MarkJob {
        account: account.to_string(),
        folder: source_folder.to_string(),
        uids: uids.to_vec(),
        target: None,
    }];
    if let Some(card) = card {
        jobs.extend(crate::meron_marks::job_for_filed_entry(conn, &card.entry_id, &card.folder_id));
    }
    crate::meron_marks::spawn_writes(engine, jobs);
}

/// The server move of copies out of the archive mailbox failed after the
/// card's built-in Archive marks were taken off (an Undo of an archive, say):
/// still in Archive, so its copies get the Archive's mark back, in the
/// background. Nothing for a card of any other folder.
pub fn restore_archive_card_marks(
    engine: &Arc<Engine>,
    conn: &Connection,
    card: &Option<store::MeronCardRelease>,
) {
    let Some(card) = card.as_ref().filter(|card| card.folder_id == store::MERON_ARCHIVE_FOLDER_ID) else {
        return;
    };
    let jobs = crate::meron_marks::job_for_filed_entry(conn, &card.entry_id, &card.folder_id);
    crate::meron_marks::spawn_writes(engine, jobs);
}

/// After an archive's server move and caching: file the conversations into
/// the built-in Archive. `keys` are [`archive_filing_keys`]'s, read before
/// the move; the archived copies `target_uids` (when known) add their own,
/// and location keys of the old folder are dropped. A conversation that
/// still has cached Inbox mail isn't filed (Archive never holds Inbox mail).
/// The archived copies carry the Archive's mark already: remembered, so this
/// device's next read doesn't act on it. Answers the mailboxes whose counts
/// changed.
pub fn file_after_archive(
    conn: &Connection,
    account: &str,
    archive_folder: &str,
    target_uids: &[u32],
    keys: &BTreeSet<String>,
) -> BTreeSet<(String, String)> {
    let mut wanted: BTreeSet<String> = keys
        .iter()
        .filter(|key| {
            key.strip_prefix("uid:")
                .and_then(|rest| rest.rsplit_once(':'))
                .is_none_or(|(folder, _)| folder == archive_folder)
        })
        .cloned()
        .collect();
    // The conversation key of each archived copy, for its mark below.
    let mut key_of_uid: Vec<(u32, Option<String>)> = Vec::with_capacity(target_uids.len());
    for uid in target_uids {
        let key = store::meron_conversation_key(conn, account, archive_folder, *uid).ok().flatten();
        if let Some(key) = &key {
            wanted.insert(key.clone());
        }
        key_of_uid.push((*uid, key));
    }
    let mut touched = BTreeSet::new();
    // Conversations not filed because Inbox mail of theirs is still cached.
    let mut skipped: BTreeSet<String> = BTreeSet::new();
    let filed = (|| -> Result<()> {
        let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
        for key in &wanted {
            if store::meron_conversation_has_inbox_copy(&tx, account, key)? {
                skipped.insert(key.clone());
                continue;
            }
            if store::meron_entry_of_key(&tx, account, key)?
                .is_some_and(|(_, folder)| folder == store::MERON_ARCHIVE_FOLDER_ID)
            {
                continue;
            }
            let moved = store::meron_move_in_in(
                &tx,
                account,
                key,
                store::MERON_ARCHIVE_FOLDER_ID,
                store::MeronFiling::User,
            )?;
            touched.extend(moved.touched.into_iter().map(|folder| (account.to_string(), folder)));
        }
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = filed {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.meronFolders",
            "could not file an archived conversation into Archive: {err:#}"
        );
        return BTreeSet::new();
    }
    if !target_uids.is_empty() && store::meron_reads_marks(conn, account, archive_folder) {
        // Filed copies: handled. Copies of a conversation not filed yet wait
        // for its Inbox mail to go; every archive-mailbox read re-checks them
        // and files them then (see `store::WAITING_FOR_INBOX`).
        let (waiting, handled): (Vec<_>, Vec<_>) = key_of_uid
            .into_iter()
            .partition(|(_, key)| key.as_ref().is_some_and(|key| skipped.contains(key)));
        let waiting: Vec<u32> = waiting.into_iter().map(|(uid, _)| uid).collect();
        let handled: Vec<u32> = handled.into_iter().map(|(uid, _)| uid).collect();
        for (uids, written_at) in [
            (handled, store::meron_mark_tick()),
            (waiting, store::WAITING_FOR_INBOX),
        ] {
            if uids.is_empty() {
                continue;
            }
            if let Err(err) = store::meron_marks_written(
                conn,
                account,
                archive_folder,
                &uids,
                Some(store::MERON_ARCHIVE_FOLDER_ID),
                written_at,
            ) {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.meronFolders",
                    "could not remember Archive marks: {err:#}"
                );
            }
        }
    }
    touched
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
    let Err(err) = store::meron_release_card(conn, &card)
        .or_else(|_| store::meron_release_card(conn, &card))
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

/// What an Undo of an archive, delete or move asks for besides the move back
/// (`"undo": true` on the move): show the copies in the Inbox again
/// (`show_in_inbox`, for mail that was shown there from its own folder), or
/// put them back in the Meron folder the action took them out of
/// (`meron_folder_id`, `meron_source` from the action's answer). Shared by
/// the phone's `mail.move` and the desktop's `messages.move`.
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
            refile_after_undo(conn, account, folder, uids, root, folder_id, &self.meron_source);
            // Back in its Meron folder: its mark-folder copies get the mark
            // again.
            let rows: Vec<(String, u32)> = uids.iter().map(|uid| (folder.to_string(), *uid)).collect();
            let by_folder = crate::meron_marks::uids_to_mark(conn, account, &rows, Some(folder_id));
            return crate::meron_marks::jobs_for(account, by_folder, Some(folder_id));
        }
        if self.show_in_inbox
            && let Err(err) = store::ai_sort_inbox_show_again_after_undo(conn, account, folder, uids)
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
