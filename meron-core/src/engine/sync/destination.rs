//! Destination in the mail synchronization engine.
//!
//! Coordinates the corresponding IMAP and store operations during folder updates.
//! The parent module exposes these operations to foreground and background sync.

use super::*;

/// The UIDs of the copies a move created, among the target's messages at or
/// above its pre-move UIDNEXT (`arrivals`). `moved` holds the lowercased
/// Message-ID of each moved message, `""` for one without.
///
/// Arrivals whose Message-ID the move didn't carry are other mail delivered
/// meanwhile, and are passed over. Each Message-ID the move did carry — the
/// empty one included — must account for exactly as many arrivals as moved
/// messages: fewer means a copy is missing, more means a concurrent delivery
/// is indistinguishable from it. Either way the answer would be wrong, so none
/// is given, and no Undo is offered.
pub(in crate::engine) fn moved_copy_uids(
    arrivals: &[imap::MessageHeader],
    moved: &[String],
) -> Vec<u32> {
    let mut expected: HashMap<&str, usize> = HashMap::new();
    for id in moved {
        *expected.entry(id.as_str()).or_default() += 1;
    }
    let mut found: HashMap<&str, Vec<u32>> = HashMap::new();
    for arrival in arrivals {
        let id = arrival.message_id.trim().to_lowercase();
        if let Some((&key, _)) = expected.get_key_value(id.as_str()) {
            found.entry(key).or_default().push(arrival.uid);
        }
    }
    let mut uids = Vec::with_capacity(moved.len());
    for (id, count) in expected {
        match found.remove(id) {
            Some(copies) if copies.len() == count => uids.extend(copies),
            _ => return Vec::new(),
        }
    }
    uids.sort_unstable();
    uids
}

/// Re-read `target` after moving `count` messages into it, and pick out the
/// copies the move created (see [`moved_copy_uids`]): every message at or
/// above the target's UIDNEXT from before the move is read, not just the
/// recent window, so a large move can't lose its earliest copies. Undo moves
/// exactly these back, rather than resolving the thread key in the target —
/// which would also take older mail of the same conversation, and finds
/// nothing for a `uid:` key, as UIDs change on a move.
///
/// `moved` is `None` when the moved messages couldn't all be described, and
/// then no copies are picked, as when the pre-move UIDNEXT is unknown.
///
/// Read-only and best-effort, on its own sessions: the MOVE has landed and
/// must not be retried or reported failed because a re-read went wrong.
pub async fn refresh_moved_copies(
    engine: &Arc<Engine>,
    account: &str,
    target: &str,
    count: usize,
    uid_next_before: Option<u32>,
    moved: Option<&[String]>,
) -> MovedCopies {
    refresh_moved_copies_with(engine, account, target, count, uid_next_before, moved, true).await
}

/// Messages moved into a folder while its re-read was put off: how many the
/// next re-read of that (account, folder) still has to cover.
static OWED_REREADS: std::sync::LazyLock<std::sync::Mutex<HashMap<(String, String), u32>>> =
    std::sync::LazyLock::new(Default::default);

/// Upper bound on one catch-up re-read, however much a long run put off.
const MAX_OWED_REREAD: u32 = 1000;

fn take_owed_reread(account: &str, target: &str) -> u32 {
    OWED_REREADS
        .lock()
        .unwrap()
        .remove(&(account.to_string(), target.to_string()))
        .unwrap_or(0)
}

fn owe_reread(account: &str, target: &str, count: u32) {
    let mut owed = OWED_REREADS.lock().unwrap();
    let entry = owed
        .entry((account.to_string(), target.to_string()))
        .or_insert(0);
    *entry = entry.saturating_add(count).min(MAX_OWED_REREAD);
}

/// One action's part of a re-read of the folder it moved mail into (see
/// [`refresh_moved_copies_with`], whose arguments these are).
pub(crate) struct CopiesAsk {
    pub account: String,
    pub target: String,
    pub count: usize,
    pub uid_next_before: Option<u32>,
    pub moved: Option<Vec<String>>,
    pub reread_window: bool,
}

/// [`refresh_moved_copies`], optionally without re-reading `target`'s recent
/// window. With `reread_window` false the window is left alone and the moved
/// count is remembered, so the next re-read of that folder (any later move
/// into it) reaches back far enough to cover everything that was skipped.
/// Locating the moved copies is unchanged.
///
/// Inside a bulk run this is the copies step every member shares (see
/// [`crate::engine::bulk`]): the target is read once for all of them.
pub async fn refresh_moved_copies_with(
    engine: &Arc<Engine>,
    account: &str,
    target: &str,
    count: usize,
    uid_next_before: Option<u32>,
    moved: Option<&[String]>,
    reread_window: bool,
) -> MovedCopies {
    let ask = || CopiesAsk {
        account: account.to_string(),
        target: target.to_string(),
        count,
        uid_next_before,
        moved: moved.map(<[String]>::to_vec),
        reread_window,
    };
    if let Some(seat) = bulk::seat() {
        // The run stopped doing its steps: as when the re-read fails.
        return seat.copies(ask()).await.unwrap_or(MovedCopies {
            batch: None,
            uids: Vec::new(),
            _snapshot_guard: None,
        });
    }
    refresh_moved_copies_shared(engine, &[ask()])
        .await
        .pop()
        .expect("one answer per ask")
}

/// [`refresh_moved_copies_with`] for several actions that moved mail into the
/// same folder of the same account: the folder is read once, and each action
/// gets the window and its own copies, in the order of `asks`.
pub(crate) async fn refresh_moved_copies_shared(
    engine: &Arc<Engine>,
    asks: &[CopiesAsk],
) -> Vec<MovedCopies> {
    let Some(first) = asks.first() else {
        return Vec::new();
    };
    let (account, target) = (first.account.as_str(), first.target.as_str());
    // One guard each: every action keeps its own through its cache writes.
    let guards: anyhow::Result<Vec<MessageSyncGuard>> = asks
        .iter()
        .map(|_| MessageSyncGuard::begin(engine))
        .collect();
    let guards = match guards {
        Ok(guards) => guards,
        Err(err) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "cannot guard moved-copy refresh: {err:#}"
            );
            return asks
                .iter()
                .map(|_| MovedCopies {
                    batch: None,
                    uids: Vec::new(),
                    _snapshot_guard: None,
                })
                .collect();
        }
    };
    let count = asks
        .iter()
        .fold(0u32, |sum, ask| sum.saturating_add(ask.count as u32));
    let batch = if asks.iter().any(|ask| ask.reread_window) {
        let owed = take_owed_reread(account, target);
        match fetch_recent_resilient(engine, account, target, 50.max(count.saturating_add(owed)))
            .await
        {
            Ok(batch) => Some(batch),
            Err(err) => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.sync",
                    "account={account} folder={target}: refresh after move failed: {err:#}"
                );
                // Still not read: the next re-read has to cover these too.
                owe_reread(account, target, count.saturating_add(owed));
                None
            }
        }
    } else {
        owe_reread(account, target, count);
        None
    };
    // Everything that arrived since the earliest of the moves began; each
    // action picks its own copies out by Message-ID.
    let floor = asks
        .iter()
        .filter(|ask| ask.moved.is_some())
        .filter_map(|ask| ask.uid_next_before.filter(|uid_next| *uid_next > 0))
        .min();
    let arrivals = match floor {
        Some(floor) => match moved_arrivals(engine, account, target, floor).await {
            Ok(arrivals) => Some(arrivals),
            Err(err) => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.sync",
                    "account={account} folder={target}: locating moved copies failed: {err:#}"
                );
                None
            }
        },
        None => None,
    };
    // How many of the actions carried each Message-ID. Copies of one that two
    // actions carried cannot be told apart, so neither is given any.
    let mut carriers: HashMap<&str, usize> = HashMap::new();
    for ask in asks {
        let ids: HashSet<&str> = ask.moved.iter().flatten().map(String::as_str).collect();
        for id in ids {
            *carriers.entry(id).or_default() += 1;
        }
    }
    asks.iter()
        .zip(guards)
        .map(|(ask, guard)| {
            let uids = match (
                ask.uid_next_before.filter(|uid_next| *uid_next > 0),
                &ask.moved,
                &arrivals,
            ) {
                (Some(_), Some(moved), Some(_))
                    if moved.iter().any(|id| carriers[id.as_str()] > 1) =>
                {
                    Vec::new()
                }
                // Only what arrived since this action's own move began.
                (Some(own_floor), Some(moved), Some(arrivals)) => {
                    let since: Vec<imap::MessageHeader> = arrivals
                        .iter()
                        .filter(|arrival| arrival.uid >= own_floor)
                        .cloned()
                        .collect();
                    moved_copy_uids(&since, moved)
                }
                _ => Vec::new(),
            };
            MovedCopies {
                batch: batch.clone(),
                uids,
                _snapshot_guard: Some(guard),
            }
        })
        .collect()
}

/// Every message in `target` at or above `floor`. Fails rather than return a
/// partial set, which [`moved_copy_uids`] would take for a missing copy.
async fn moved_arrivals(
    engine: &Arc<Engine>,
    account: &str,
    target: &str,
    floor: u32,
) -> anyhow::Result<Vec<imap::MessageHeader>> {
    let uids = engine
        .with_read_session(account, |session| {
            let target = target.to_string();
            Box::pin(async move { imap::uids_from(session, &target, floor).await })
        })
        .await?;
    let (headers, skipped) =
        fetch_headers_isolating_unparseable(engine, account, target, uids.clone()).await?;
    anyhow::ensure!(
        skipped.is_empty() && headers.len() == uids.len(),
        "read {} of {} messages",
        headers.len(),
        uids.len()
    );
    Ok(headers)
}

#[cfg(test)]
mod owed_reread_tests {
    use super::*;

    // A bulk run puts off the re-read of the folder it moves mail into. What
    // it skipped has to be remembered, or the run's last re-read (sized for
    // one conversation) would leave the earlier ones out of the cache.
    #[test]
    fn a_put_off_reread_is_remembered_until_the_next_one_takes_it() {
        let (account, folder) = ("owed-test@example.com", "Trash");
        assert_eq!(take_owed_reread(account, folder), 0);
        owe_reread(account, folder, 12);
        owe_reread(account, folder, 30);
        owe_reread(account, "Archive", 5);
        assert_eq!(take_owed_reread(account, folder), 42);
        assert_eq!(take_owed_reread(account, folder), 0);
        assert_eq!(take_owed_reread(account, "Archive"), 5);
        owe_reread(account, folder, MAX_OWED_REREAD);
        owe_reread(account, folder, 10);
        assert_eq!(take_owed_reread(account, folder), MAX_OWED_REREAD);
    }
}
