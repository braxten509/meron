//! Searches starred messages across eligible mailboxes.
//! The engine combines live folder matches with cached starred results.
//! Page assembly preserves the cross-folder ordering used by the reader.

use super::*;

/// The mailboxes a starred view reads.
///
/// Gmail keeps every starred message in its Starred mailbox (special-use `\Flagged`, localized
/// name), so that one folder answers the whole account. Failing that, All Mail holds each message
/// exactly once; scanning every label instead listed a message once per label it carried and
/// SELECTed every folder. Other servers flag per mailbox, so the open one is the answer.
pub async fn starred_search_folders(
    engine: &Arc<Engine>,
    account: &str,
    requested: &str,
) -> Vec<String> {
    let is_gmail = {
        let accounts = engine.accounts.lock().await;
        accounts
            .get(account)
            .map(|creds| creds.auth_type == "gmail_oauth")
            .unwrap_or(false)
    };
    if !is_gmail {
        return vec![requested.to_string()];
    }
    let find_starred = |folders: &[imap::Folder]| {
        folders
            .iter()
            .find(|folder| {
                let name = folder.name.as_str();
                folder.special_use.as_deref() == Some("flagged")
                    || name.eq_ignore_ascii_case("starred")
                    || name.eq_ignore_ascii_case("[gmail]/starred")
                    || name.eq_ignore_ascii_case("[google mail]/starred")
            })
            .map(|folder| folder.name.clone())
    };
    let mut folders = store::get_folders(&engine.db.lock().unwrap(), account).unwrap_or_default();
    if let Some(folder) = find_starred(&folders) {
        return vec![folder];
    }
    // The cache predates special-use `\Flagged` or has never listed folders.
    let fresh = engine
        .with_read_session(account, |session| {
            Box::pin(async move { imap::list_folders(session).await })
        })
        .await;
    match fresh {
        Ok(fresh) => {
            if let Ok(db) = engine.db.lock() {
                let _ = store::upsert_folders(&db, account, &fresh);
            }
            folders = fresh;
            if let Some(folder) = find_starred(&folders) {
                return vec![folder];
            }
        }
        Err(err) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.starred",
                "folder LIST failed for account={account}: {err:#}"
            );
        }
    }
    let all_mail = folders
        .into_iter()
        .find(|folder| folder.special_use.as_deref() == Some("all"))
        .map(|folder| folder.name);
    vec![all_mail.unwrap_or_else(|| requested.to_string())]
}

/// Starred messages across `folders`, newest first. The cache answers at once; `refresh` asks the
/// server for each folder's full `\Flagged` set, fetches the newest of those, and drops (and
/// unflags) cached rows the server no longer has flagged — a star removed in another client
/// otherwise lingered until the next flag sync.
pub async fn search_starred_mail_messages(
    engine: &Arc<Engine>,
    account: &str,
    folders: &[String],
    limit: u32,
    refresh: bool,
) -> anyhow::Result<Vec<imap::MessageHeader>> {
    let mut by_key: HashMap<(String, u32), imap::MessageHeader> = HashMap::new();
    for folder in folders {
        for mut message in store::get_starred(&engine.db.lock().unwrap(), account, folder, limit)? {
            // The Inbox's starred list carries mail the AI shows there from
            // other folders under their real folder; keep it, so the unflag
            // check below never takes one for an Inbox UID.
            if message.folder.is_empty() {
                message.folder = folder.clone();
            }
            by_key.insert((message.folder.clone(), message.uid), message);
        }
    }

    if refresh {
        let _snapshot_guard = MessageSyncGuard::begin(engine)?;
        // Best-effort per folder: a single folder's failure is logged and
        // skipped, so the closure always succeeds (no stale-retry needed here).
        let fetch_cap = (limit as usize).max(LIVE_SEARCH_BATCH);
        let server = engine
            .with_read_session(account, |session| {
                let folders = folders.to_vec();
                Box::pin(async move {
                    let mut found = Vec::new();
                    for folder in &folders {
                        let result = async {
                            let flagged = imap::search_starred_uids(session, folder).await?;
                            // Fetch the newest UIDs, then order by date below;
                            // the cap bounds a mailbox with thousands of stars.
                            let newest = &flagged[..flagged.len().min(fetch_cap)];
                            let headers =
                                imap::fetch_headers_by_uid(session, folder, newest).await?;
                            anyhow::Ok((flagged, headers))
                        }
                        .await;
                        match result {
                            Ok((flagged, headers)) => {
                                found.push((folder.clone(), flagged, headers))
                            }
                            Err(err) => crate::mlog!(
                                crate::log::Level::Warn,
                                "mail.starred",
                                "starred search failed for folder={folder}: {err:#}"
                            ),
                        }
                    }
                    anyhow::Ok(found)
                })
            })
            .await;

        match server {
            Ok(found) => {
                for (folder, flagged, mut headers) in found {
                    let flagged: std::collections::HashSet<u32> = flagged.into_iter().collect();
                    let unstarred = by_key
                        .keys()
                        .filter(|(key_folder, uid)| *key_folder == folder && !flagged.contains(uid))
                        .cloned()
                        .collect::<Vec<_>>();
                    for header in &mut headers {
                        header.folder = folder.clone();
                        header.starred = true;
                    }
                    {
                        let db = engine.db.lock().unwrap();
                        for key in &unstarred {
                            store::update_message_starred(&db, account, &folder, key.1, false)?;
                        }
                        store::upsert_read_messages(&db, account, &folder, &headers)?;
                    }
                    for key in unstarred {
                        by_key.remove(&key);
                    }
                    for header in headers {
                        by_key.insert((folder.clone(), header.uid), header);
                    }
                }
            }
            Err(err) => crate::mlog!(
                crate::log::Level::Warn,
                "mail.starred",
                "starred search connect failed for account={account}: {err:#}"
            ),
        }
    }

    let mut messages = by_key.into_values().collect::<Vec<_>>();
    store::sort_search_hits(&mut messages, limit);
    Ok(messages)
}

/// One cursor-paged page of starred mail across `folders`, newest first, for a folder's Starred filter.
///
/// The first page (no cursor) with `refresh` first brings the store up to date with the server's
/// `\Flagged` set ([`search_starred_mail_messages`] stores what it fetches); the page itself is
/// then read from the store, where conversations in a Meron folder are left out before the LIMIT,
/// so pages stay full and the cursor exact.
///
/// Later pages are store reads (`date:<date>:<uid>` cursors).
pub async fn starred_mail_page(
    engine: &Arc<Engine>,
    account: &str,
    folders: &[String],
    limit: u32,
    before_cursor: Option<(i64, u32)>,
    before_folder: Option<&str>,
    refresh: bool,
) -> anyhow::Result<(Vec<imap::MessageHeader>, Option<String>)> {
    if refresh && before_cursor.is_none() {
        search_starred_mail_messages(engine, account, folders, limit, true).await?;
    }
    let db = engine.db.lock().unwrap();
    let mut merged = Vec::new();
    let mut more = false;
    for folder in folders {
        let (page, next) = store::get_starred_page_from(
            &db,
            account,
            folder,
            limit,
            before_cursor,
            before_folder,
        )?;
        more |= next.is_some();
        merged.extend(page);
    }
    // Folder breaks (date, uid) ties, as the store's pages do: the Inbox's
    // page also holds mail from other folders, whose UIDs can repeat.
    merged.sort_by(|a, b| {
        b.date
            .cmp(&a.date)
            .then_with(|| b.uid.cmp(&a.uid))
            .then_with(|| b.folder.cmp(&a.folder))
    });
    if merged.len() > limit as usize {
        merged.truncate(limit as usize);
        more = true;
    }
    let next_cursor = more.then(|| merged.last()).flatten().map(|header| {
        store::format_mail_cursor(header.date, header.uid, Some(header.folder.as_str()))
    });
    Ok((merged, next_cursor))
}
