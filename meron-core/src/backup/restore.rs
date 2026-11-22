//! Restores backup contents inside a single SQLite transaction.
//! Accounts, subscriptions and task lists are reconciled with existing records.
//! The caller receives counts of restored and skipped entries.

use super::*;
use base64::Engine as _;

/// Write a parsed backup into the store.
///
/// Accounts are matched by id; one that already exists is left completely alone (counted in
/// `skipped`) rather than overwritten, so restoring onto a machine that is already set up cannot
/// break a working connection. Settings, by contrast, are the backup's whole point and do
/// overwrite.
///
/// `store_secrets` is called only for accounts actually created, and only for backups that carry
/// secrets.
pub fn apply(
    conn: &Connection,
    data: &BackupData,
    store_secrets: &dyn Fn(&Connection, &str, &Secrets) -> Result<()>,
) -> Result<ImportSummary> {
    let tx = conn.unchecked_transaction()?;
    let summary = apply_in(&tx, data, store_secrets)?;
    tx.commit()?;
    Ok(summary)
}

/// [`apply`] inside a transaction the caller holds.
pub fn apply_in(
    tx: &Connection,
    data: &BackupData,
    store_secrets: &dyn Fn(&Connection, &str, &Secrets) -> Result<()>,
) -> Result<ImportSummary> {
    let mut summary = ImportSummary::default();
    let now = store::now_unix();

    for account in &data.accounts {
        let id = account.id.trim();
        if id.is_empty() {
            continue;
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
            params![id],
            |row| row.get::<_, i64>(0),
        )? != 0;
        if exists {
            summary.skipped += 1;
            continue;
        }
        let engine = if account.engine.is_empty() {
            "mail"
        } else {
            &account.engine
        };
        let provider = if account.provider.is_empty() {
            "custom"
        } else {
            &account.provider
        };
        tx.execute(
            "INSERT INTO accounts(id, engine, provider, email, display_name, avatar_url,
                                  sender_name, config, prefs, sort_order, created_at, updated_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                id,
                engine,
                provider,
                account.email,
                account.display_name,
                account.avatar_url,
                account.sender_name,
                json_column(&account.config),
                json_column(&account.prefs),
                account.sort_order,
                now,
            ],
        )?;
        summary.accounts += 1;

        for sub in &account.subscriptions {
            summary.feeds += restore_subscription(&tx, id, sub, now)?;
        }

        if let Some(secrets) = &account.secrets
            && !secrets.is_empty()
        {
            store_secrets(&tx, id, secrets)?;
            summary.secrets += 1;
        }
    }

    // After the accounts, whose conversations they hold. A folder named like
    // one already here is kept under the existing id.
    let (meron_added, meron_ids) = store::meron_backup_restore(&tx, &data.meron_folders)?;
    summary.meron_folders = meron_added;

    for (key, value) in &data.settings {
        store::setting_set(&tx, key, value)?;
        summary.settings += 1;
    }
    // AI sorting's Meron entries follow the folders to the ids kept.
    store::ai_sort_remap_meron_ids(&tx, &meron_ids)?;

    for (index, list) in data.task_lists.iter().enumerate() {
        summary.tasks += restore_task_list(&tx, list, index, now)?;
    }

    // By id, never over an event already here: importing the same file twice
    // adds nothing, and an edit made since the export is kept.
    for event in &data.events {
        if !event.id.is_empty() && store::insert_event_if_absent(&tx, event)? {
            summary.events += 1;
        }
    }

    // Keyed by Message-ID, so they rejoin their mail as it syncs back in.
    for entry in &data.tags {
        let known: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
            params![entry.account],
            |row| row.get(0),
        )?;
        if known {
            store::restore_tag(tx, &entry.account, &entry.key, &entry.tag)?;
        }
    }
    Ok(summary)
}

/// Insert one subscription, re-deriving its id from the normalized URL the way
/// a live subscribe does. `INSERT OR IGNORE` because `url` is UNIQUE across all
/// accounts: a feed already subscribed elsewhere is skipped, not duplicated.
fn restore_subscription(
    conn: &Connection,
    account: &str,
    sub: &BackupSubscription,
    now: i64,
) -> Result<u32> {
    let Ok(url) = crate::rss::normalize_feed_url(&sub.url) else {
        return Ok(0);
    };
    let id = crate::rss::rss_subscription_id(&url);
    let added = conn.execute(
        "INSERT OR IGNORE INTO subscriptions
           (id, account, url, title, site_url, feed_title, enabled, created_at, updated_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            id,
            account,
            url,
            sub.title.trim(),
            sub.site_url.trim(),
            sub.feed_title.trim(),
            i64::from(sub.enabled),
            now,
        ],
    )?;
    Ok(added as u32)
}

/// Preserve identities rather than merging unrelated rows with equal titles.
/// Old backups lack IDs: derive repeatable IDs from their contents and position
/// so even identical rows survive, and importing the same file is idempotent.
fn restore_task_list(
    conn: &Connection,
    list: &BackupTaskList,
    index: usize,
    now: i64,
) -> Result<u32> {
    let list_id = if list.id.is_empty() {
        legacy_task_id("tasklist", &serde_json::to_vec(&(index, list))?)
    } else {
        list.id.clone()
    };
    conn.execute(
        "INSERT OR IGNORE INTO task_lists(id, title, sort_order, created_at, updated_at)
         VALUES(?1, ?2, ?3, ?4, ?4)",
        params![list_id, list.title.trim(), list.sort_order, now],
    )?;

    let mut restored = 0;
    for (index, task) in list.tasks.iter().enumerate() {
        let task_id = if task.id.is_empty() {
            legacy_task_id("task", &serde_json::to_vec(&(&list_id, index, task))?)
        } else {
            task.id.clone()
        };
        let added = conn.execute(
            "INSERT OR IGNORE INTO tasks(id, list_id, title, notes, due_at, completed_at, sort_order,
                               account, thread_id, message_id, json, created_at, updated_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, json_object('reminder_after', ?12), ?11, ?12)",
            params![
                task_id,
                list_id,
                task.title.trim(),
                task.notes,
                task.due_at.max(0),
                task.completed_at.max(0),
                task.sort_order,
                task.account,
                task.thread_id,
                task.message_id,
                if task.created_at > 0 {
                    task.created_at
                } else {
                    now
                },
                now,
            ],
        )?;
        if added > 0 {
            store::set_task_due_time(conn, &task_id, task.due_has_time)?;
        }
        restored += added as u32;
    }
    Ok(restored)
}

fn legacy_task_id(prefix: &str, bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    format!(
        "{prefix}-legacy-{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest.as_ref())
    )
}

fn json_column(value: &Value) -> String {
    if value.is_object() {
        value.to_string()
    } else {
        "{}".to_string()
    }
}
