//! Discovers arrivals delivered outside Inbox and reconciles their sorting state.
//! Account sync calls this path for watched folders and for taught spam.
//! It reports eligible arrivals under the destination chosen for each message.

use super::*;

/// Least time between two checks of one account's other folders, so repeated
/// pulls to refresh don't repeat the STATUS round.
const FOLDER_CHECK_EVERY: Duration = Duration::from_secs(60);
/// How many headers a sync of a changed folder reads, as an IDLE refresh does.
const FOLDER_CHECK_LIMIT: u32 = 50;

/// When each account's other folders were last checked.
static FOLDER_CHECKS: std::sync::Mutex<Option<HashMap<String, Instant>>> =
    std::sync::Mutex::new(None);
/// Most new mail one check reads per folder past a sync's window: more than
/// [`FOLDER_CHECK_LIMIT`] arrived at once; the rest waits for the next check.
const FOLDER_CARRY_MAX: usize = 100;

/// Folders the server says can't be opened (a `\Noselect` parent, or gone),
/// skipped from then on while the app runs.
static FOLDER_CHECK_FAILED: std::sync::Mutex<BTreeSet<(String, String)>> =
    std::sync::Mutex::new(BTreeSet::new());

/// Spam mail already looked at for another device's lesson, per account, so
/// mail a sync can't reach doesn't start one every check: mail found with the
/// lesson's tag, and (`1`) mail found to have lost it.
static SPAM_LESSONS_SEEN: std::sync::Mutex<
    Option<HashMap<String, (BTreeSet<u32>, BTreeSet<u32>)>>,
> = std::sync::Mutex::new(None);

/// Most Spam mail one check reads for its lesson tag; the rest waits for the
/// next check.
const SPAM_LESSON_READ_MAX: usize = 200;

/// Read spam lessons added or forgotten on another device.
///
/// Normal background sync covers the Inbox. For Spam, search once for the lesson
/// keyword and fetch headers for messages whose known keyword state has changed.
async fn check_taught_spam(engine: &Arc<Engine>, account: &str) {
    let Some(junk) = store::folder_for_role(&engine.db.lock().unwrap(), account, "junk")
        .ok()
        .flatten()
    else {
        return;
    };
    let tag = store::LessonPlace::Spam.tag();
    let keyword = crate::tags::tag_to_keyword(&tag);
    let found = engine
        .with_read_session(account, |session| {
            let junk = junk.clone();
            let keyword = keyword.clone();
            Box::pin(async move { imap::uids_with_keyword(session, &junk, &keyword).await })
        })
        .await;
    let uids = match found {
        Ok(uids) => uids,
        Err(error) => {
            warn(format_args!(
                "account={account}: Spam lesson check: {error:#}"
            ));
            return;
        }
    };
    let found: BTreeSet<u32> = uids.iter().copied().collect();
    // Mail this device read the tag on that the server no longer names.
    let had = store::ai_lesson_tagged_uids(&engine.db.lock().unwrap(), account, &junk, &tag)
        .unwrap_or_default();
    let (fresh, lost): (Vec<u32>, Vec<u32>) = {
        let mut seen = SPAM_LESSONS_SEEN.lock().unwrap();
        let (tagged, untagged) = seen
            .get_or_insert_with(HashMap::new)
            .entry(account.to_string())
            .or_default();
        // Tagged again later counts as new again, and the other way round.
        tagged.retain(|uid| found.contains(uid));
        untagged.retain(|uid| !found.contains(uid));
        (
            uids.into_iter().filter(|uid| tagged.insert(*uid)).collect(),
            had.into_iter()
                .filter(|uid| !found.contains(uid) && untagged.insert(*uid))
                .collect(),
        )
    };
    let unread = !fresh.is_empty()
        && store::ai_lesson_tag_unread(&engine.db.lock().unwrap(), account, &junk, &fresh, &tag)
            .unwrap_or(false);
    let mut wanted: Vec<u32> = lost.clone();
    if unread {
        wanted.extend(&fresh);
    }
    if wanted.is_empty() {
        return;
    }
    wanted.sort_unstable_by(|a, b| b.cmp(a));
    let later = wanted.split_off(wanted.len().min(SPAM_LESSON_READ_MAX));
    // Mail not read this time is looked at again at the next check.
    let look_again = |uids: &[u32]| {
        let mut seen = SPAM_LESSONS_SEEN.lock().unwrap();
        if let Some((tagged, untagged)) = seen.as_mut().and_then(|seen| seen.get_mut(account)) {
            for uid in uids {
                tagged.remove(uid);
                untagged.remove(uid);
            }
        }
    };
    look_again(&later);
    let fetched = engine
        .with_read_session(account, |session| {
            let junk = junk.clone();
            let wanted = wanted.clone();
            Box::pin(async move { imap::fetch_headers_by_uid(session, &junk, &wanted).await })
        })
        .await;
    let stored = fetched.and_then(|mut headers| {
        for header in &mut headers {
            header.folder = junk.clone();
        }
        store::upsert_read_messages(&engine.db.lock().unwrap(), account, &junk, &headers)
    });
    if let Err(error) = stored {
        warn(format_args!(
            "account={account}: Spam lessons not read: {error:#}"
        ));
        look_again(&wanted);
    }
}

/// Check watched folders for mail delivered by server-side filters.
///
/// After an Inbox refresh, check each enabled account at most once a minute. Reuse
/// one pooled session for STATUS and sync folders whose UIDNEXT or UIDVALIDITY changed.
///
/// Leave mail in its server folder. Return new arrivals filed under Meron folders
/// with notifications enabled so the caller can announce them.
pub async fn check_watched_folders(engine: &Arc<Engine>, account: &str) -> Vec<MeronArrivals> {
    let mut alerts = Vec::new();
    if store::is_local_account(account) || engine.is_paused(account) {
        return alerts;
    }
    let Some(plan) = plan_for(engine, account, "INBOX").await else {
        return alerts;
    };
    {
        let mut checks = FOLDER_CHECKS.lock().unwrap();
        let checks = checks.get_or_insert_with(HashMap::new);
        if checks
            .get(account)
            .is_some_and(|at| at.elapsed() < FOLDER_CHECK_EVERY)
        {
            return alerts;
        }
        checks.insert(account.to_string(), Instant::now());
    }
    // One check per account at a time.
    let key = format!("ai-folders:{account}");
    if !engine.syncing.lock().unwrap().insert(key.clone()) {
        return alerts;
    }
    struct Claim<'a>(&'a Engine, String);
    impl Drop for Claim<'_> {
        fn drop(&mut self) {
            self.0.syncing.lock().unwrap().remove(&self.1);
        }
    }
    let _claim = Claim(engine, key);

    check_taught_spam(engine, account).await;
    // Mail another device claimed and never finished (see "Claims"). On its
    // own: it may ask the AI, which this check doesn't wait for.
    {
        let engine = engine.clone();
        let account = account.to_string();
        tokio::spawn(async move { take_over_stuck(&engine, &account).await });
    }

    let folders = {
        let db = engine.db.lock().unwrap();
        store::ai_sort_check_folders(&db, &plan.config, account).unwrap_or_else(|error| {
            warn(format_args!(
                "account={account}: folder check skipped: {error:#}"
            ));
            Vec::new()
        })
    };
    let failed = FOLDER_CHECK_FAILED.lock().unwrap().clone();
    let folders: Vec<store::AiSortCheckFolder> = folders
        .into_iter()
        .filter(|folder| !failed.contains(&(account.to_string(), folder.name.clone())))
        .collect();
    if folders.is_empty() {
        return alerts;
    }
    let names: Vec<String> = folders.iter().map(|folder| folder.name.clone()).collect();
    // Per folder: its `(UIDVALIDITY, UIDNEXT)`, or the server's refusal and
    // whether it means the folder can't be opened at all. A failing connection
    // ends the round instead; every folder is asked again next check.
    let statuses = engine
        .with_read_session(account, |session| {
            let names = names.clone();
            Box::pin(async move {
                let mut out = Vec::with_capacity(names.len());
                for folder in names {
                    match imap::folder_status(session, &folder).await {
                        Ok(status) => out.push(Ok(status)),
                        Err(error) if imap::is_server_refusal(&error) => out.push(Err((
                            imap::is_unselectable_mailbox(&error),
                            format!("{error:#}"),
                        ))),
                        Err(error) => return Err(error),
                    }
                }
                anyhow::Ok(out)
            })
        })
        .await;
    let statuses = match statuses {
        Ok(statuses) => statuses,
        Err(error) => {
            warn(format_args!("account={account}: folder check: {error:#}"));
            return alerts;
        }
    };
    let mut changed = Vec::new();
    for (folder, status) in folders.iter().zip(statuses) {
        match status {
            // Never synced, new UIDs, or new mail since the last sync. A first
            // sync only starts the folder's watch: nothing is sorted then.
            Ok(now) if folder.state != Some(now) => changed.push(folder.name.clone()),
            Ok(_) => {}
            // `\Noselect` or gone: skipped from now on while the app runs.
            Err((true, error)) => {
                warn(format_args!(
                    "account={account} folder={}: can't be opened, skipped: {error}",
                    folder.name
                ));
                FOLDER_CHECK_FAILED
                    .lock()
                    .unwrap()
                    .insert((account.to_string(), folder.name.clone()));
            }
            // Refused for now (busy, unavailable…): asked again next check.
            Err((false, error)) => warn(format_args!(
                "account={account} folder={}: STATUS refused, trying again later: {error}",
                folder.name
            )),
        }
    }
    for folder in &changed {
        match sync_background_messages(engine, account, folder, FOLDER_CHECK_LIMIT).await {
            Ok(synced) => alerts.extend(synced.meron_arrivals),
            Err(error) if error.is::<BackgroundSyncCancelled>() => {}
            Err(error) => warn(format_args!(
                "account={account} folder={folder}: check sync failed: {error:#}"
            )),
        }
    }
    // More new mail than one sync's window holds: read the rest, oldest first,
    // a bounded amount per folder per check; what's left waits for the next.
    let behind: Vec<String> = {
        let db = engine.db.lock().unwrap();
        store::ai_sort_check_folders(&db, &plan.config, account)
            .unwrap_or_default()
            .into_iter()
            .filter(|folder| folder.behind())
            .map(|folder| folder.name)
            .collect()
    };
    for folder in behind {
        if engine.is_paused(account) {
            break;
        }
        match crate::engine::sync::sort_carried_mail(engine, account, &folder, FOLDER_CARRY_MAX)
            .await
        {
            Ok(filed) => alerts.extend(filed),
            Err(error) => warn(format_args!(
                "account={account} folder={folder}: reading older new mail failed: {error:#}"
            )),
        }
    }
    alerts
}

/// What a sync announces: its arrivals less what rules or the AI moved or are
/// moving (`silenced`), plus mail whose AI move failed for good (`late`).
pub(crate) fn settle_arrivals(
    arrivals: &mut Vec<imap::MessageHeader>,
    silenced: &BTreeSet<u32>,
    late: Vec<imap::MessageHeader>,
) {
    arrivals.retain(|message| !silenced.contains(&message.uid));
    for header in late {
        if !arrivals.iter().any(|known| known.uid == header.uid) {
            arrivals.push(header);
        }
    }
}
