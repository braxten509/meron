//! Mobile mail protocol handlers for synchronization.
//!
//! The mobile dispatcher calls these handlers with protocol parameters.
//! They translate between the wire format and the shared mail engine.

use super::*;

const MOBILE_BODY_PREFETCH_DAYS: u32 = 7;
const MOBILE_BODY_PREFETCH_MAX: usize = 20;
const MOBILE_BODY_PREFETCH_TIMEOUT_SECS: u64 = 20;
pub(super) const DRAFT_SYNC_LIMIT: u32 = 20;

fn prefetch_mobile_bodies(
    data_directory: &str,
    engine: &std::sync::Arc<crate::engine::Engine>,
    account_id: &str,
    folder: &str,
) {
    let options = crate::engine::BodyPrefetchOptions {
        days: MOBILE_BODY_PREFETCH_DAYS,
        max_count: Some(MOBILE_BODY_PREFETCH_MAX),
        media_root: std::path::PathBuf::from(data_directory).join("attachments"),
    };
    let result = crate::ffi::engine_block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(MOBILE_BODY_PREFETCH_TIMEOUT_SECS),
            crate::engine::prefetch_bodies_with_options(engine, account_id, folder, options),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timed out"))?
    });
    match result {
        Ok(fetched) => crate::mlog!(
            crate::log::Level::Info,
            "mail.sync",
            "Prefetched {fetched} message bodies for {account_id}/{folder}"
        ),
        Err(err) => crate::mlog!(
            crate::log::Level::Warn,
            "mail.sync",
            "Body prefetch failed for {account_id}/{folder}: {err}"
        ),
    }
}

/// Longest a `mail.sync` waits on the body fetch its notification snippets
/// need. Past this the response goes back with whatever bodies are cached, so a
/// slow server costs a subject-only notification rather than a missed sync.
const NOTIFY_PREVIEW_TIMEOUT_SECS: u64 = 8;

/// Cache the bodies of new arrivals so the notification can show a snippet of
/// each. Best-effort: a failure only costs the snippets.
fn fetch_notification_bodies(
    data_directory: &str,
    engine: &std::sync::Arc<crate::engine::Engine>,
    account_id: &str,
    folder: &str,
    headers: &[crate::imap::MessageHeader],
) {
    let uids: Vec<u32> = headers
        .iter()
        .take(crate::mail_model::NEW_MESSAGES_DETAIL_MAX)
        .map(|header| header.uid)
        .collect();
    let media_root = std::path::PathBuf::from(data_directory).join("attachments");
    let result = crate::ffi::engine_block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(NOTIFY_PREVIEW_TIMEOUT_SECS),
            crate::engine::fetch_bodies_for_uids(engine, account_id, folder, &uids, media_root),
        )
        .await
        .map_err(|_| anyhow::anyhow!("timed out"))?
    });
    if let Err(err) = result {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.sync",
            "Notification body fetch failed for {account_id}: {err}"
        );
    }
}

/// Inbox syncs whose non-gating tail (`finish_mobile_sync`) is still running on
/// a detached thread, keyed by `account/folder`, so overlapping pull-to-refresh
/// calls don't stack a second prefetch behind the first.
static DEFERRED_SYNC_TAILS: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Claim the deferred-tail slot for `account/folder`. Returns false when a
/// tail for that mailbox is already running, in which case the caller skips.
fn claim_sync_tail(key: &str) -> bool {
    DEFERRED_SYNC_TAILS.lock().unwrap().insert(key.to_string())
}

fn release_sync_tail(key: &str) {
    DEFERRED_SYNC_TAILS.lock().unwrap().remove(key);
}

/// Post-sync work that doesn't gate the inbox list: body prefetch plus Sent and
/// Drafts header syncs (so cross-folder conversation views read from the local
/// store).
fn finish_mobile_sync(
    data_directory: &str,
    engine: &std::sync::Arc<crate::engine::Engine>,
    account_id: &str,
    folder: &str,
    limit: u32,
    announce: bool,
) -> Vec<Value> {
    prefetch_mobile_bodies(data_directory, engine, account_id, folder);

    // Also pull Sent and Drafts so messages sent or drafted from another
    // client surface in the cross-folder conversation view (shared with the
    // desktop sync path; the per-folder recent sync only covers the open
    // folder).
    let outcomes = crate::ffi::engine_block_on(async {
        anyhow::Ok(crate::engine::sync_companion_folders(engine, account_id, folder, limit).await)
    })
    .unwrap_or_default();
    for sync in outcomes {
        let (role, companion) = (sync.role, &sync.folder);
        match sync.result {
            Ok(result) => crate::mlog!(
                crate::log::Level::Info,
                "mail.sync",
                "{role} sync account={account_id} folder={companion} synced={}",
                result.count
            ),
            Err(err) => crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "{role} sync failed for {account_id}: {err:#}"
            ),
        }
    }

    // New mail server filters delivered into other folders goes through AI
    // sorting too (a no-op unless sorting is set up for the account). Last, as
    // it doesn't gate the Inbox list either. What it files into Meron folders
    // with notifications on comes back as `mail.newMessages` details.
    if folder.eq_ignore_ascii_case("INBOX") {
        let filed = crate::ffi::engine_block_on(async {
            anyhow::Ok(crate::engine::ai_sort::check_watched_folders(engine, account_id).await)
        })
        .unwrap_or_default();
        if announce {
            return meron_details(data_directory, engine, account_id, &filed);
        }
    }
    Vec::new()
}

/// `mail.newMessages` details for new mail AI sorting filed into Meron
/// folders with notifications on, one per folder, bodies fetched first.
fn meron_details(
    data_directory: &str,
    engine: &std::sync::Arc<crate::engine::Engine>,
    account_id: &str,
    groups: &[crate::engine::ai_sort::MeronArrivals],
) -> Vec<Value> {
    groups
        .iter()
        .filter_map(|group| {
            fetch_notification_bodies(
                data_directory,
                engine,
                account_id,
                &group.folder,
                &group.headers,
            );
            crate::ffi::mobile_meron_new_messages_detail(data_directory, account_id, group)
        })
        .collect()
}

/// Run the non-gating sync tail.
///
/// When the caller opted in via `defer_tail` (the foreground UI, whose process outlives the call)
/// it runs on a detached thread so `mail.sync` returns as soon as the inbox headers are stored —
/// the tail costs body prefetch alone up to 20s — and emits `mail.synced` when done so the UI
/// re-reads the store.
///
/// Everything else (background workers, the IDLE watcher) keeps it inline: an OS-granted execution
/// window ends when the worker returns, so a detached thread could be frozen mid-fetch.
fn run_mobile_sync_tail(
    data_directory: &str,
    engine: &std::sync::Arc<crate::engine::Engine>,
    account_id: &str,
    folder: &str,
    limit: u32,
    defer_tail: bool,
) -> Vec<Value> {
    if !defer_tail {
        return finish_mobile_sync(data_directory, engine, account_id, folder, limit, true);
    }
    let key = format!("{account_id}/{folder}");
    if !claim_sync_tail(&key) {
        return Vec::new();
    }
    let data_directory = data_directory.to_string();
    let engine = engine.clone();
    let account_id = account_id.to_string();
    let folder = folder.to_string();
    std::thread::spawn(move || {
        // The foreground UI's sync: like its own arrivals, which it doesn't
        // notify for, what this files into Meron folders isn't announced.
        finish_mobile_sync(&data_directory, &engine, &account_id, &folder, limit, false);
        release_sync_tail(&key);
        // Not `mail.synced`: that handler reloads the thread list (resetting
        // pagination), and the tail never changes the open folder's headers —
        // only bodies and Sent/Drafts, which affect the open thread at most.
        crate::ffi::emit_event(
            "mail.tailSynced",
            json!({ "account": account_id, "folder": folder }),
        );
    });
    Vec::new()
}

pub(crate) fn sync_mobile_mail(data_directory: &str, params: &Value) -> Result<Value, String> {
    let account_id = req_account_id(params)?;
    let folder = params
        .get("folder_id")
        .or_else(|| params.get("folder"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(canon_folder)
        .unwrap_or_else(|| "INBOX".to_string());
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(50)
        .clamp(1, 500);
    let sync_folders = params
        .get("folders")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let defer_tail = params
        .get("defer_tail")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // RSS accounts sync through the feed pipeline, not the IMAP engine.
    let is_rss = with_mobile_db(data_directory, |conn| {
        Ok(json!(is_rss_account(&conn, &account_id)?))
    })?;
    if is_rss.as_bool() == Some(true) {
        return with_mobile_db(data_directory, |conn| {
            sync_mobile_rss_with_conn(conn, account_id)
        });
    }

    // Route IMAP sync through the shared Engine: reuses the warm session pool
    // while foreground and runs the same ops as desktop (sync_folders +
    // sync_messages do fetch/upsert/prune/flag-reconcile into the store), so the
    // two platforms can't drift.
    let engine = crate::ffi::engine_for(data_directory)?;
    let started = std::time::Instant::now();
    let creds = crate::ffi::engine_block_on(engine.ensure_valid_creds(&account_id))?;
    if account_needs_reconnect(&creds) {
        return Err(format!("account needs reconnect: {account_id}"));
    }
    let creds_ms = started.elapsed().as_millis();
    let folders_started = std::time::Instant::now();
    let folders_count = if sync_folders {
        crate::ffi::engine_block_on(crate::engine::sync_folders(&engine, &account_id))?.len()
    } else {
        0
    };
    let folders_ms = folders_started.elapsed().as_millis();
    let messages_started = std::time::Instant::now();
    let mut synced = crate::ffi::engine_block_on(crate::engine::sync_messages(
        &engine,
        &account_id,
        &folder,
        limit,
    ))?;
    let messages_ms = messages_started.elapsed().as_millis();
    // New mail AI sorting filed into Meron folders with notifications on.
    let mut meron_new_messages = meron_details(
        data_directory,
        &engine,
        &account_id,
        &std::mem::take(&mut synced.meron_arrivals),
    );
    // Arrivals are only reported for INBOX and folders opted in to notifications.
    let new_messages = (!synced.arrivals.is_empty())
        .then_some(synced.arrivals)
        .and_then(|headers| {
            // Fetch the arrivals' own bodies before building the detail: the
            // notification shows a snippet of each, and the general prefetch in
            // the sync tail runs too late (and may be deferred entirely).
            fetch_notification_bodies(data_directory, &engine, &account_id, &folder, &headers);
            crate::ffi::mobile_new_messages_detail(data_directory, &account_id, &folder, &headers)
        });
    crate::mlog!(
        crate::log::Level::Info,
        "mail.sync",
        "account={account_id} folder={folder} synced={} folders={folders_count} creds_ms={creds_ms} folders_ms={folders_ms} messages_ms={messages_ms}",
        synced.count
    );
    meron_new_messages.extend(run_mobile_sync_tail(
        data_directory,
        &engine,
        &account_id,
        &folder,
        limit,
        defer_tail,
    ));
    let mut response = json!({
        "ok": true,
        "account": account_id,
        "folder": folder,
        "synced": synced.count,
        "folders": folders_count,
    });
    if let Some(detail) = new_messages {
        response["new_messages"] = detail;
    }
    // One `mail.newMessages`-shaped detail per Meron folder, named after it.
    if !meron_new_messages.is_empty() {
        response["meron_new_messages"] = Value::Array(meron_new_messages);
    }
    Ok(response)
}

#[cfg(test)]
mod sync_tail_tests {
    use super::{claim_sync_tail, release_sync_tail};

    #[test]
    fn sync_tail_claim_is_exclusive_per_mailbox() {
        assert!(claim_sync_tail("tail-test@example.com/INBOX"));
        // A second sync for the same mailbox must not stack another tail.
        assert!(!claim_sync_tail("tail-test@example.com/INBOX"));
        // A different mailbox is independent.
        assert!(claim_sync_tail("tail-test@example.com/Archive"));
        release_sync_tail("tail-test@example.com/INBOX");
        assert!(claim_sync_tail("tail-test@example.com/INBOX"));
        release_sync_tail("tail-test@example.com/INBOX");
        release_sync_tail("tail-test@example.com/Archive");
    }
}
