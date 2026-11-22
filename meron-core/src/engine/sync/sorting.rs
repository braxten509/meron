//! Sorting in the mail synchronization engine.
//!
//! Coordinates the corresponding IMAP and store operations during folder updates.
//! The parent module exposes these operations to foreground and background sync.

use super::*;

/// New mail of a folder AI sorting watches (besides the Inbox) that a sync's
/// window couldn't hold, because more arrived at once than it reads: the
/// oldest `max` of it not cached yet, from the folder's watch up, read and put
/// through the same steps as a sync's new mail (held, stored, rules, then the
/// AI). The watch moves past what was read; the rest waits for the next check.
/// Answers with what it filed into Meron folders with notifications on.
pub(in crate::engine) async fn sort_carried_mail(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    max: usize,
) -> anyhow::Result<Vec<ai_sort::MeronArrivals>> {
    let Some(plan) = ai_sort::plan_for(engine, account, folder).await else {
        return Ok(Vec::new());
    };
    let (validity, uid_next, from) = {
        let db = engine.db.lock().unwrap();
        let state = store::get_folder_state(&db, account, folder)?;
        let watch = store::ai_sort_watch(&db, account, folder)?;
        match (state, watch) {
            (Some((validity, uid_next)), Some((watched, from)))
                if validity == watched && from < uid_next =>
            {
                (validity, uid_next, from)
            }
            _ => return Ok(Vec::new()),
        }
    };
    let guard = MessageSyncGuard::begin(engine)?;
    let listed = engine
        .with_read_session(account, |session| {
            let folder = folder.to_string();
            Box::pin(async move { imap::uids_from(session, &folder, from).await })
        })
        .await?;
    let cached = store::header_synced_uids(&engine.db.lock().unwrap(), account, folder)?;
    let missing: Vec<u32> = listed
        .into_iter()
        .filter(|uid| *uid < uid_next && !cached.contains(uid))
        .collect();
    let take: Vec<u32> = missing.iter().copied().take(max).collect();
    // Past the last UID read when some are left for later; else all the way.
    let read_to = match take.last() {
        Some(last) if take.len() < missing.len() => last + 1,
        _ => uid_next,
    };
    let (headers, skipped) = if take.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        fetch_headers_isolating_unparseable(engine, account, folder, take).await?
    };
    // Mail that couldn't be read this time is asked for again next check:
    // the mark stops before the first such UID.
    let read_to = skipped
        .iter()
        .copied()
        .min()
        .map_or(read_to, |first| read_to.min(first));
    let persist_engine = engine.clone();
    let persist_account = account.to_string();
    let persist_folder = folder.to_string();
    let (candidates, ruled, new_unread) = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        let account = persist_account.as_str();
        let folder = persist_folder.as_str();
        let db = persist_engine.db.lock().unwrap();
        if store::get_folder_state(&db, account, folder)?.map(|(validity, _)| validity)
            != Some(validity)
        {
            return anyhow::Ok((Vec::new(), false, Vec::new()));
        }
        // As a sync does: first seen and held before the upsert stores them.
        let first_seen_headers =
            store::first_seen_messages(&db, account, folder, from, uid_next, &headers)?;
        let first_seen: Vec<u32> = first_seen_headers.iter().map(|header| header.uid).collect();
        // What a sync would count as arrivals: new to the account and unread.
        let new_unread: Vec<imap::MessageHeader> = first_seen_headers
            .into_iter()
            .filter(|header| !header.seen)
            .collect();
        let held = store::ai_sort_hold_folder(
            &db,
            account,
            folder,
            validity,
            uid_next,
            (from, read_to),
            &headers,
            store::now_unix(),
        )?;
        store::upsert_messages(&db, account, folder, &headers)?;
        if let Err(err) = store::apply_rules(&db, account, folder, validity, &first_seen) {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.rules",
                "account={account} folder={folder}: rules skipped: {err:#}"
            );
        }
        let held = store::ai_sort_skip_rule_actions(&db, account, folder, held)?;
        let held = store::ai_sort_skip_meron_foldered(&db, account, folder, held)?;
        anyhow::Ok((held, !first_seen.is_empty(), new_unread))
    })
    .await??;
    let mut alerts = Vec::new();
    if !candidates.is_empty() {
        let sorted =
            ai_sort::sort_new_mail(engine, account, folder, validity, plan, candidates).await;
        alerts = ai_sort::meron_arrivals(engine, folder, &new_unread, &sorted.filed);
    }
    if ruled {
        spawn_rule_worker(engine, account, folder);
    }
    Ok(alerts)
}
