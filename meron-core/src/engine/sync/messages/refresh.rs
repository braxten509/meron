//! Registers cache snapshots while synchronizing messages.
//! Post-write refreshes hold a guard until the caller persists the batch.
//! Dropping the guard also releases removal markers after cancellation.

use super::*;

/// A cache refresh after a successful server write. Keep the guard until the
/// returned batch has been persisted, so cleanup cannot release stale UIDs.
pub struct WrittenFolderRefresh {
    pub batch: imap::RecentBatch,
    _guard: MessageSyncGuard,
}

/// Cache warming must not turn a completed APPEND/save into a failed write.
/// Registration and network failures skip the refresh; callers retain success.
pub async fn refresh_written_folder(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
) -> Option<WrittenFolderRefresh> {
    let result = async {
        let guard = MessageSyncGuard::begin(engine)?;
        let batch = fetch_recent_resilient(engine, account, folder, limit).await?;
        anyhow::Ok(WrittenFolderRefresh {
            batch,
            _guard: guard,
        })
    }
    .await;
    match result {
        Ok(refresh) => Some(refresh),
        Err(operation_error) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "account={account} folder={folder}: post-write refresh skipped: {operation_error:#}"
            );
            None
        }
    }
}

// Keep registration alive through persistence, including when the caller drops
// its future after spawn_blocking has started. Drop also covers network errors
// and cancellation without leaving markers protected by a dead snapshot.
pub struct MessageSyncGuard {
    pub(super) engine: Arc<Engine>,
    pub(super) id: i64,
}

impl MessageSyncGuard {
    pub fn begin(engine: &Arc<Engine>) -> anyhow::Result<Self> {
        let db = engine.db.lock().unwrap();
        let (id, _) = store::begin_message_sync(&db)?;
        Ok(Self {
            engine: engine.clone(),
            id,
        })
    }
}

impl Drop for MessageSyncGuard {
    fn drop(&mut self) {
        let db = self.engine.db.lock().unwrap();
        if let Err(operation_error) = store::end_message_sync(&db, self.id) {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "removal marker cleanup failed: {operation_error:#}"
            );
        }
    }
}
