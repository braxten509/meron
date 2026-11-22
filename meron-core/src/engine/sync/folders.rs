//! Folders in the mail synchronization engine.
//!
//! Coordinates the corresponding IMAP and store operations during folder updates.
//! The parent module exposes these operations to foreground and background sync.

use super::*;

/// Reconnect to IMAP, list folders, and refresh the store.
pub async fn sync_folders(
    engine: &Arc<Engine>,
    account: &str,
) -> anyhow::Result<Vec<imap::Folder>> {
    let folders = engine
        .with_read_session(account, |session| {
            Box::pin(async move { imap::list_folders(session).await })
        })
        .await?;
    crate::mlog!(
        crate::log::Level::Debug,
        "mail.sync",
        "folders account={account}: {} listed [{}]",
        folders.len(),
        folders
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let db = engine.db.lock().unwrap();
    store::upsert_folders(&db, account, &folders)?;
    Ok(folders)
}

/// Update every physical copy selected by a star action. Commit each mailbox to
/// the cache only after its server write succeeds, including on partial failure.
pub async fn mark_starred_copies(
    engine: &Engine,
    account: &str,
    targets: &std::collections::BTreeMap<String, Vec<u32>>,
    starred: bool,
) -> anyhow::Result<()> {
    for (folder, uids) in targets {
        engine
            .with_flag_write_session(
                account,
                |session| {
                    let folder = folder.clone();
                    Box::pin(async move { imap::prepare_flag_update(session, &folder).await })
                },
                |session| {
                    let uids = uids.clone();
                    Box::pin(async move { imap::store_starred(session, &uids, starred).await })
                },
            )
            .await?;
        let db = engine.db.lock().unwrap();
        for uid in uids {
            store::update_message_starred(&db, account, folder, *uid, starred)?;
        }
    }
    Ok(())
}

/// Bring the server up to date with pending tag changes. Each write commits on
/// its own, so a failure leaves only that write pending for the next try.
pub async fn push_pending_tags(engine: &Engine, account: &str) -> anyhow::Result<()> {
    let turn = engine
        .tag_pushes
        .lock()
        .unwrap()
        .entry(account.to_string())
        .or_default()
        .clone();
    let _turn = turn.lock().await;

    let refused_key = |folder: &str, tag: &str| format!("{account}\n{folder}\n{tag}");
    let pushes = {
        let db = engine.db.lock().unwrap();
        if store::account_paused(&db, account)? {
            return Ok(());
        }
        store::pending_tag_pushes(&db, account)?
    };
    let mut refused = std::collections::HashSet::new();
    for push in &pushes {
        let key = refused_key(&push.folder, &push.tag);
        if push.add && engine.refused_tags.lock().unwrap().contains(&key) {
            refused.insert((push.folder.clone(), push.tag.clone()));
            continue;
        }
        let writable = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stored = engine
            .with_flag_write_session(
                account,
                |session| {
                    let writable = writable.clone();
                    let push = push.clone();
                    Box::pin(async move {
                        let can =
                            imap::prepare_tag_update(session, &push.folder, &push.tag).await?;
                        writable.store(can, std::sync::atomic::Ordering::Relaxed);
                        anyhow::Ok(())
                    })
                },
                |session| {
                    let writable = writable.load(std::sync::atomic::Ordering::Relaxed);
                    let push = push.clone();
                    Box::pin(async move {
                        // Only a new keyword can be refused. Taking one away is
                        // always asked for: it may be there either way.
                        if push.add && !writable {
                            return anyhow::Ok(false);
                        }
                        imap::store_tag(session, &push.uids, &push.tag, push.add).await?;
                        anyhow::Ok(true)
                    })
                },
            )
            .await;
        match stored {
            Ok(true) => store::record_tag_push(&engine.db.lock().unwrap(), account, push)?,
            Ok(false) => {
                engine.refused_tags.lock().unwrap().insert(key);
                refused.insert((push.folder.clone(), push.tag.clone()));
            }
            Err(err) => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.tags",
                    "account={account} folder={}: tag push deferred: {err:#}",
                    push.folder
                );
            }
        }
    }
    store::settle_pending_tags(&engine.db.lock().unwrap(), account, &refused)
}

/// [`push_pending_tags`] without waiting for it: a tag is already saved locally,
/// and a slow or absent server must not hold up the action or the sync that
/// asked. Whatever fails stays pending for the next sync to retry.
pub fn spawn_tag_push(engine: &Arc<Engine>, account: &str) {
    let engine = engine.clone();
    let account = account.to_string();
    tokio::spawn(async move {
        if let Err(err) = push_pending_tags(&engine, &account).await {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.tags",
                "account={account}: tag push failed: {err:#}"
            );
        }
    });
}
