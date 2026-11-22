//! Shared thread reading for both frontends.
//!
//! Desktop (`messages.thread` in the sidecar) and mobile (`mail.threadRead` over FFI) resolve a
//! thread the same way: select and paginate its headers, serve message bodies cache-first, pull the
//! newest uncached body inline, hand older gaps to a deduped background fetch, and shape the final
//! bridge-ready message JSON.
//!
//! Platform callers only parse their own id formats and decide how a finished background fetch is
//! announced (sidecar event vs FFI event callback).

mod recovery;
pub(crate) use recovery::spawn_fill_thread_bodies;
#[cfg(test)]
use recovery::store_thread_bodies;
use recovery::{fetch_into_slots, media_recovery_held};

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};

use crate::engine::{Engine, MessageSyncGuard, TransferLimits, attach_html, message_key};
use crate::reply::ReplyTarget;
use crate::{imap, mail_model, parse, quote, reply, store};

/// Called when a background body fetch has something for the UI: bodies or
/// attachment files that arrived, or a body that could not be downloaded.
pub type BodiesFetchedHook = Box<dyn Fn() + Send + Sync + 'static>;

pub struct ThreadReadArgs<'a> {
    pub account: &'a str,
    /// Canonical nominal folder; headers from the cross-folder query carry
    /// their own source folder and fall back to this one.
    pub folder: &'a str,
    /// Full bridge thread id, used verbatim for the messages' `id`/`thread_id`
    /// fields so they match what the requesting UI keys on.
    pub thread_id: &'a str,
    /// Root thread key, already split from any branch-subject suffix.
    pub thread_key: &'a str,
    pub subject_filter: Option<&'a str>,
    /// Present for UI reads (page size); absent for full-scan reads (markRead,
    /// compose threading), which fetch every missing body synchronously.
    pub limit: Option<u32>,
    /// Bounded best-effort body fetch for printing; never fail for missing bodies.
    pub for_print: bool,
    /// Opaque cursor from a previous page's `next_cursor`: `message:<base64 folder>:<uid>`, since UIDs
    /// are only unique within one mailbox. Legacy `uid:N` cursors are still accepted, but match on UID
    /// alone and so can resolve to the wrong message in a cross-folder thread — never hand-build one.
    pub before_cursor: Option<&'a str>,
    /// Attachment/media directory (env-derived on desktop, `data_dir` on mobile).
    pub media_root: PathBuf,
}

/// A header's resolved body state: the cached message (if any) and whether the
/// cache can serve it in full — threading headers extracted and inline media
/// still on disk. Incomplete slots are candidates for a refetch but still
/// render from whatever the cache has.
struct Slot {
    folder: String,
    cached: Option<parse::Message>,
    complete: bool,
    /// Attachment files of `cached` that are not on disk, counted once when the
    /// message is loaded or fetched rather than on every question about it.
    media_missing: usize,
}

/// The headers one page of a thread shows, oldest first, and the cursor of the
/// page before it: the thread across folders (or one folder for a `uid:` key),
/// narrowed to the subject branch, drafts collapsed, then sliced by
/// `before_cursor` and `limit`.
pub fn thread_page_headers(
    db: &rusqlite::Connection,
    account: &str,
    folder: &str,
    thread_key: &str,
    subject_filter: Option<&str>,
    limit: Option<u32>,
    before_cursor: Option<&str>,
) -> anyhow::Result<(Vec<imap::MessageHeader>, Option<String>)> {
    let (page, next_cursor, _) = thread_page_headers_and_older(
        db,
        account,
        folder,
        thread_key,
        subject_filter,
        limit,
        before_cursor,
    )?;
    Ok((page, next_cursor))
}

/// [`thread_page_headers`], plus the headers of the page before it (at most
/// `limit` of them, oldest first): the page a reader scrolling back reaches
/// next. Empty for full reads and for the oldest page.
fn thread_page_headers_and_older(
    db: &rusqlite::Connection,
    account: &str,
    folder: &str,
    thread_key: &str,
    subject_filter: Option<&str>,
    limit: Option<u32>,
    before_cursor: Option<&str>,
) -> anyhow::Result<(
    Vec<imap::MessageHeader>,
    Option<String>,
    Vec<imap::MessageHeader>,
)> {
    // Thread view spans folders within the account so the user's own Sent
    // replies appear alongside the inbox messages they thread with.
    // Mail in Trash or Junk, and replies to a conversation deleted earlier,
    // stay out unless read from their own folder (see `store::ChatScope`).
    //
    // Exception: a synthetic `uid:N` key (a message with no real threading
    // headers — e.g. a freshly-saved draft) is folder-local, because UIDs are
    // folder-scoped. Spanning folders would match an unrelated message that
    // happens to share UID N elsewhere and render the wrong thread.
    // A sender row's chat: their mail in this folder plus Sent mail and
    // drafts to them (see `store::sender_chat_headers`).
    let mut headers = if let Some(addr) = store::sender_key_addr(thread_key) {
        store::sender_chat_headers(db, account, folder, &addr)?
    } else if thread_key.starts_with("uid:") {
        let mut headers = store::get_thread_headers(db, account, folder, thread_key)?;
        // A message opened straight from a notification can have a cached body
        // before its header row is synced; synthesize a header so the read
        // still renders instead of coming back empty.
        if headers.is_empty()
            && let Ok(uid) = thread_key["uid:".len()..].parse::<u32>()
            && store::get_cached_message(db, account, folder, uid)
                .ok()
                .flatten()
                .is_some()
        {
            headers.push(imap::MessageHeader {
                uid,
                folder: folder.to_string(),
                thread_key: thread_key.to_string(),
                ..Default::default()
            });
        }
        headers
    } else {
        store::get_thread_headers_from_folder(db, account, folder, thread_key)?
    };
    if let Some(filter) = subject_filter {
        headers.retain(|header| store::thread_grouping_subject(&header.subject) == filter);
    }
    let headers = store::collapse_thread_draft_headers(db, account, folder, headers)?;

    // `before_cursor` / `limit` are honored as a date-ordered slice so the
    // cross-folder query stays consistent with the old per-folder pagination
    // contract. New cursors name both the source folder and UID because UIDs
    // are only unique within one mailbox. Legacy uid-only cursors remain
    // readable for pagination requests minted by an older app process.
    let before_location = before_cursor.and_then(parse_thread_cursor);
    Ok(if let Some(limit) = limit {
        let mut headers = headers;
        if let Some(cursor) = before_location
            && let Some(index) = thread_cursor_position(&headers, &cursor)
        {
            headers.truncate(index);
        }
        let total = headers.len();
        let start = total.saturating_sub(limit as usize);
        let page = headers[start..].to_vec();
        let next_cursor = if start > 0 {
            page.first().map(thread_cursor)
        } else {
            None
        };
        let older = headers[start.saturating_sub(limit as usize)..start].to_vec();
        (page, next_cursor, older)
    } else {
        (headers, None, Vec::new())
    })
}

/// Read one page of a thread and return `{ "messages": [...], "next_cursor"? }`
/// with final bridge-shaped message JSON (see [`thread_message_json`]).
pub async fn read_thread_page(
    engine: &Arc<Engine>,
    args: ThreadReadArgs<'_>,
    on_bodies_fetched: Option<BodiesFetchedHook>,
) -> anyhow::Result<Value> {
    read_thread_page_inner(engine, args, on_bodies_fetched, false).await
}

/// [`read_thread_page`] for a reader that scrolls back through the thread a page at a time (mobile,
/// which stores envelopes only): a UI page also starts downloading the bodies of the page before
/// it, so that page is readable by the time the reader scrolls up to it rather than arriving as
/// placeholders that fill in, and grow, under their finger.
pub async fn read_thread_page_with_lookahead(
    engine: &Arc<Engine>,
    args: ThreadReadArgs<'_>,
    on_bodies_fetched: Option<BodiesFetchedHook>,
) -> anyhow::Result<Value> {
    read_thread_page_inner(engine, args, on_bodies_fetched, true).await
}

async fn read_thread_page_inner(
    engine: &Arc<Engine>,
    args: ThreadReadArgs<'_>,
    on_bodies_fetched: Option<BodiesFetchedHook>,
    lookahead: bool,
) -> anyhow::Result<Value> {
    // Shared between this page's own fill and the look-ahead fill: each
    // announces itself when it lands.
    let on_bodies_fetched: Option<Arc<dyn Fn() + Send + Sync + 'static>> =
        on_bodies_fetched.map(Arc::from);
    let hook = || -> Option<BodiesFetchedHook> {
        on_bodies_fetched
            .clone()
            .map(|notify| Box::new(move || (*notify)()) as BodiesFetchedHook)
    };
    let ThreadReadArgs {
        account,
        folder,
        thread_id,
        thread_key,
        subject_filter,
        limit,
        for_print,
        before_cursor,
        media_root,
    } = args;

    let (headers, next_cursor, older_headers) = {
        let db = engine.db.lock().unwrap();
        thread_page_headers_and_older(
            &db,
            account,
            folder,
            thread_key,
            subject_filter,
            limit,
            before_cursor,
        )?
    };

    let cached_messages: Vec<(String, Option<parse::Message>)> = {
        let db = engine.db.lock().unwrap();
        headers
            .iter()
            .map(|header| {
                let message_folder = if header.folder.is_empty() {
                    folder.to_string()
                } else {
                    header.folder.clone()
                };
                let cached = store::get_cached_message(&db, account, &message_folder, header.uid)
                    .ok()
                    .flatten();
                (message_folder, cached)
            })
            .collect()
    };
    // Paged reads return cached text immediately; full scans and print snapshots
    // wait for missing attachment files as well.
    let wait_for_media = limit.is_none() || for_print;
    // The files are looked for here, with no lock held.
    let counted: Vec<(String, Option<parse::Message>, usize)> = cached_messages
        .into_iter()
        .map(|(message_folder, cached)| {
            let media_missing = cached.as_ref().map_or(0, |message| {
                parse::missing_media_count(&media_root, message)
            });
            (message_folder, cached, media_missing)
        })
        .collect();
    let mut slots = {
        counted
            .into_iter()
            .map(|(message_folder, cached, media_missing)| {
                let media_settled = !wait_for_media || media_missing == 0;
                // Rows cached before the threading-header extraction landed
                // have an empty `message_id`; refetch them so
                // reply_to/cc/references populate. Real-world mail almost
                // always carries a Message-ID, so emptiness is a reliable
                // "pre-extraction cache" signal.
                let complete = cached
                    .as_ref()
                    .is_some_and(|message| !message.message_id.is_empty() && media_settled);
                Slot {
                    folder: message_folder,
                    cached,
                    complete,
                    media_missing,
                }
            })
            .collect::<Vec<_>>()
    };

    let missing: Vec<usize> = slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| !slot.complete)
        .map(|(index, _)| index)
        .collect();
    if !missing.is_empty() {
        if for_print {
            // Persist each completed body before starting the next one, so the
            // page deadline cannot discard progress and retries converge.
            // Leave room under the desktop's 30s bridge deadline.
            // Do not start background work for this independent print snapshot.
            let result = tokio::time::timeout(std::time::Duration::from_secs(20), async {
                for &index in &missing {
                    fetch_into_slots(
                        engine,
                        account,
                        &headers,
                        &mut slots,
                        &[index],
                        &media_root,
                        TransferLimits::within(crate::engine::INLINE_BODY_BUDGET),
                    )
                    .await?;
                }
                anyhow::Ok(())
            })
            .await;
            if !matches!(result, Ok(Ok(()))) {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail",
                    "print body fetch incomplete for {account}"
                );
            }
        } else if limit.is_none() {
            // Full-scan path (markRead, compose threading): every message must
            // be present — a reply built off a body-less copy has no
            // Message-ID and would orphan the thread on the recipient's side.
            // Fetch all gaps before answering.
            //
            // The allowance grows with the count, up to a ceiling a caller can
            // sit through. What arrived before it ran out is cached, so a
            // thread too long for one attempt is shorter on the next.
            if let Err(err) = fetch_into_slots(
                engine,
                account,
                &headers,
                &mut slots,
                &missing,
                &media_root,
                thread_budget(missing.len(), FULL_SCAN_BODY_BUDGET),
            )
            .await
            {
                // Degrade to the stale cached bodies when every message still
                // has one (e.g. a local draft with no Message-ID that a
                // refetch can't improve on); fail only when the thread would
                // otherwise be missing content.
                if slots.iter().any(|slot| slot.cached.is_none()) {
                    return Err(err);
                }
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail",
                    "thread body refetch for {account}: {err:#}"
                );
            }
        } else {
            // UI page: it must answer within the caller's timeout budget even
            // on a slow or dead connection, so fetch only the newest gap
            // inline — usually the very message the user tapped, since sync
            // stores envelopes only — and hand older gaps to the background
            // fill below.
            let newest = *missing.last().unwrap();
            // A fill that just failed announced itself, and this is the read
            // that shows it: none of its messages is downloaded again here,
            // or an unreachable server would be asked on every notification.
            // Taking the failures leaves the read after this one free to retry.
            let failures: Vec<(usize, String)> = missing
                .iter()
                .filter_map(|&index| {
                    let key = message_key(account, &slots[index].folder, headers[index].uid);
                    Some((index, engine.take_body_failure(&key)?))
                })
                .collect();
            let failed = failures
                .iter()
                .find(|(index, _)| *index == newest)
                .map(|(_, error)| error);
            if let Some(error) = failed {
                if slots.iter().all(|slot| slot.cached.is_none()) {
                    return Err(anyhow::anyhow!(error.clone()));
                }
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail",
                    "background thread body fetch for {account}: {error}"
                );
            }
            // Re-reads triggered by notifications must not start a second
            // interactive download beside the same message's background fill.
            let filling = engine.body_fetches.lock().unwrap().contains(&message_key(
                account,
                &slots[newest].folder,
                headers[newest].uid,
            ));
            let inline_error = if filling || failed.is_some() {
                None
            } else {
                match fetch_into_slots(
                    engine,
                    account,
                    &headers,
                    &mut slots,
                    &[newest],
                    &media_root,
                    TransferLimits::within(crate::engine::INLINE_BODY_BUDGET),
                )
                .await
                {
                    Ok(()) => None,
                    // FETCH may deliver a body before command completion fails.
                    // Show any bodies now available and retry remaining gaps
                    // in the background.
                    Err(err) if slots.iter().any(|slot| slot.cached.is_some()) => {
                        crate::mlog!(
                            crate::log::Level::Warn,
                            "mail",
                            "inline thread body fetch for {account} uid {}: {err:#}",
                            headers[newest].uid
                        );
                        None
                    }
                    Err(err) => Some(err),
                }
            };
            let mut background = pending_body_recovery(
                &engine.media_recovery_hold.lock().unwrap(),
                account,
                &slots,
                &headers,
                &missing,
            );
            // Keep cached messages visible; a later read can retry the rest.
            background.retain(|(folder, uid)| {
                !failures.iter().any(|(index, _)| {
                    slots[*index].folder == *folder && headers[*index].uid == *uid
                })
            });
            // Bodies we can already show, whose attachment bytes are gone.
            for item in messages_missing_media(
                &engine.media_recovery_hold.lock().unwrap(),
                account,
                &slots,
                &headers,
            ) {
                if !background.contains(&item) {
                    background.push(item);
                }
            }
            if !background.is_empty() {
                spawn_fill_thread_bodies(engine, account, background, media_root.clone(), hook());
            }
            // Even a first-ever body that exceeds the interactive budget gets
            // a longer background attempt, while the caller receives its error.
            if let Some(err) = inline_error {
                return Err(err);
            }
        }
    } else if limit.is_some() && !for_print {
        // Every body is cached, but a cleared attachment cache still leaves
        // keys pointing at files that are gone. Refetch those without holding
        // the body the UI already has.
        let background = messages_missing_media(
            &engine.media_recovery_hold.lock().unwrap(),
            account,
            &slots,
            &headers,
        );
        if !background.is_empty() {
            spawn_fill_thread_bodies(engine, account, background, media_root.clone(), hook());
        }
    }

    // The page before this one, for a reader scrolling back: its missing bodies
    // download now, in one batch, while this page is being read. Not for full
    // or print reads, and never a body whose download just failed — that
    // failure is the read of its own page's to show, and fetching it again on
    // every re-read would turn an unreachable server into a loop.
    if lookahead && limit.is_some() && !for_print && !older_headers.is_empty() {
        let pending: Vec<(String, u32)> = {
            let db = engine.db.lock().unwrap();
            older_headers
                .iter()
                .map(|header| {
                    let message_folder = if header.folder.is_empty() {
                        folder.to_string()
                    } else {
                        header.folder.clone()
                    };
                    (message_folder, header.uid)
                })
                .filter(|(message_folder, uid)| {
                    !store::has_cached_body(&db, account, message_folder, *uid).unwrap_or(true)
                })
                .collect()
        };
        let pending: Vec<(String, u32)> = pending
            .into_iter()
            .filter(|(message_folder, uid)| {
                !engine.body_failure_pending(&message_key(account, message_folder, *uid))
            })
            .collect();
        if !pending.is_empty() {
            spawn_fill_thread_bodies(engine, account, pending, media_root.clone(), hook());
        }
    }

    let (mine, ours, remote_policy) = {
        let db = engine.db.lock().unwrap();
        (
            store::self_addrs(&db, account),
            store::all_self_addrs(&db),
            store::remote_image_policy(&db, account).unwrap_or_default(),
        )
    };
    let mut seen_message_ids = HashSet::new();
    let mut messages = Vec::with_capacity(headers.len());
    // Looked at after any fill above was started, so a placeholder this read
    // returns says whether its body is on the way.
    let filling = engine.body_fetches.lock().unwrap().clone();
    for (header, slot) in headers.iter().zip(slots) {
        let mut cached = slot.cached;
        if let Some(message) = cached.as_mut() {
            attach_html(message, &remote_policy);
        }
        // Newly synced envelope rows do not have json.message_id yet, so the
        // SQL-level cross-folder dedupe cannot collapse a self-addressed
        // Sent/Inbox pair on the first thread read. Once the body fetch has
        // cached the full message, collapse later copies before returning.
        let message_id_key = cached
            .as_ref()
            .map(|message| message.message_id.trim().to_ascii_lowercase())
            .unwrap_or_default();
        if !message_id_key.is_empty() && !seen_message_ids.insert(message_id_key) {
            continue;
        }
        let mut message = thread_message_json(
            account,
            thread_id,
            &slot.folder,
            header,
            cached.as_ref(),
            &mine,
            &ours,
        );
        message["tags"] = json!(store::message_tags(
            &engine.db.lock().unwrap(),
            account,
            &slot.folder,
            header.uid,
        )?);
        // A paged read answers before pruned attachment files are back. The
        // body and its `/media` URLs are the same before and after, so this is
        // what tells a client its pictures are worth asking for again. A count,
        // not a flag: a recovery that restores one file and not another still
        // has to show.
        if slot.media_missing > 0 {
            message["media_missing"] = json!(slot.media_missing);
        }
        if let Some(cached) = cached.as_mut() {
            if cached.invite.is_none() {
                cached.invite =
                    crate::calendar::invite_from_attachments(&media_root, &cached.attachments);
            }
            if let Some(invite) = &cached.invite {
                let db = engine.db.lock().unwrap();
                // Covers an update downloaded before updates were applied on
                // download, or before what it revises was filed.
                let _ = crate::calendar::events::apply_invite_update(
                    &db,
                    account,
                    &cached.from_addr,
                    &cached.message_id,
                    invite,
                );
                message["invite"] = crate::calendar::invite_json_for(&db, account, invite, &mine);
            }
        }
        // Tells a body still downloading in the background from one that
        // could not be downloaded: only the second is worth a retry button.
        if cached.is_none() && filling.contains(&message_key(account, &slot.folder, header.uid)) {
            message["body_loading"] = json!(true);
        }
        messages.push(message);
    }

    let mut out = json!({ "messages": messages });
    if let Some(cursor) = next_cursor {
        out.as_object_mut()
            .unwrap()
            .insert("next_cursor".into(), Value::String(cursor));
    }
    Ok(out)
}

// Attachment holds never block a first body download or its longer background
// retry. Only already-cached bodies participate in media recovery suppression.
fn pending_body_recovery(
    hold: &MediaRecoveryHold,
    account: &str,
    slots: &[Slot],
    headers: &[imap::MessageHeader],
    indices: &[usize],
) -> Vec<(String, u32)> {
    indices
        .iter()
        .copied()
        .filter(|&index| {
            !slots[index].complete
                && (!slots[index]
                    .cached
                    .as_ref()
                    .is_some_and(|message| !message.message_id.is_empty())
                    || !media_recovery_held(
                        hold,
                        &message_key(account, &slots[index].folder, headers[index].uid),
                    ))
        })
        .map(|index| (slots[index].folder.clone(), headers[index].uid))
        .collect()
}

/// Cached messages whose attachment files are not on disk: pruned, never written, or cached before their keys were kept.
///
/// A message is left out while its recovery is on hold (see `Engine::media_recovery_hold`), so the
/// reader is not notified and asked again for as long as the thread stays open.
fn messages_missing_media(
    hold: &MediaRecoveryHold,
    account: &str,
    slots: &[Slot],
    headers: &[imap::MessageHeader],
) -> Vec<(String, u32)> {
    slots
        .iter()
        .enumerate()
        .filter(|(_, slot)| slot.complete && slot.media_missing > 0)
        .map(|(index, slot)| (slot.folder.clone(), headers[index].uid))
        .filter(|(folder, uid)| !media_recovery_held(hold, &message_key(account, folder, *uid)))
        .collect()
}

fn thread_cursor(header: &imap::MessageHeader) -> String {
    format!(
        "message:{}:{}",
        URL_SAFE_NO_PAD.encode(header.folder.as_bytes()),
        header.uid
    )
}

fn parse_thread_cursor(cursor: &str) -> Option<(Option<String>, u32)> {
    if let Some(rest) = cursor.strip_prefix("message:") {
        let (folder, uid) = rest.split_once(':')?;
        let folder = String::from_utf8(URL_SAFE_NO_PAD.decode(folder).ok()?).ok()?;
        return Some((Some(folder), uid.parse().ok()?));
    }
    let uid = cursor.strip_prefix("uid:")?.parse().ok()?;
    Some((None, uid))
}

fn thread_cursor_position(
    headers: &[imap::MessageHeader],
    cursor: &(Option<String>, u32),
) -> Option<usize> {
    let (folder, uid) = cursor;
    headers.iter().position(|header| {
        header.uid == *uid && folder.as_deref().is_none_or(|f| header.folder == f)
    })
}

/// The most a full-scan read (reply, markRead) spends downloading a thread.
const FULL_SCAN_BODY_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

/// The background fill. Nobody waits on it, so a large message gets the time
/// it needs to land, but it holds a session, the sync guard and its
/// messages' places in `body_fetches` while it runs: a silent socket has to
/// end it long before the total does.
const BACKGROUND_BODY_LIMITS: TransferLimits = TransferLimits {
    idle: std::time::Duration::from_secs(120),
    total: std::time::Duration::from_secs(600),
};

/// One on-demand allowance per message, up to `ceiling`: for a caller that is
/// waiting on all of them.
fn thread_budget(messages: usize, ceiling: std::time::Duration) -> TransferLimits {
    TransferLimits::within(
        (crate::engine::INLINE_BODY_BUDGET * messages.max(1) as u32).min(ceiling),
    )
}

type MediaRecoveryHold = std::collections::HashMap<String, Option<std::time::Instant>>;

/// How long readers stop waiting on a message's attachment files after a
/// download did not deliver it: long enough that an offline or struggling
/// connection does not stall every open of a thread that is otherwise cached.
const MEDIA_RECOVERY_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(120);

/// Final bridge-shaped message JSON, rendered from the header row plus the
/// cached body when one exists. A missing body (on-demand fetch failed or
/// still in flight) sets `body_missing` so clients can show a placeholder
/// with a retry instead of dropping the message.
pub fn thread_message_json(
    account_id: &str,
    thread_id: &str,
    folder: &str,
    header: &imap::MessageHeader,
    cached: Option<&parse::Message>,
    mine: &HashSet<String>,
    ours: &HashSet<String>,
) -> Value {
    // A conversation spans folders, but IMAP UIDs do not: INBOX/42 and
    // Sent/42 are two different messages. Key the message by its own mailbox
    // location rather than by the conversation's nominal folder so clients
    // do not collapse equal numeric UIDs from different folders. The ordinary
    // uid-style thread-id format also keeps the id understood by the existing
    // per-message action parsers.
    let id = mail_model::format_thread_id(account_id, folder, &format!("uid:{}", header.uid));
    let from_addr = cached
        .map(|message| message.from_addr.as_str())
        .unwrap_or(header.from_addr.as_str());
    // Who a reply addresses and who it copies is settled here, from the same
    // headers, so every frontend and the MCP tools agree on it (see
    // [`crate::reply`]).
    let reply = reply::reply_json(
        &ReplyTarget {
            from_name: cached
                .map(|message| message.from_name.as_str())
                .unwrap_or(header.from_name.as_str()),
            from_addr,
            reply_to: cached
                .map(|message| message.reply_to.as_str())
                .unwrap_or(""),
            to: cached.map(|message| message.to.as_str()).unwrap_or(""),
            cc: cached.map(|message| message.cc.as_str()).unwrap_or(""),
        },
        ours,
    );
    json!({
        "id": id,
        "account_id": account_id,
        "folder_id": folder,
        "thread_id": thread_id,
        // Classified in the core (own address *or* Sent-folder provenance) so
        // both frontends render alias-sent mail as outgoing without knowing
        // the account's aliases.
        "outgoing": store::account_message_is_outgoing(
            account_id,
            mine,
            folder,
            from_addr,
            cached.is_some_and(|message| message.delivered),
        ),
        "from_name": cached.map(|message| message.from_name.as_str()).unwrap_or(header.from_name.as_str()),
        "from_addr": cached.map(|message| message.from_addr.as_str()).unwrap_or(header.from_addr.as_str()),
        "to": cached.map(|message| message.to.as_str()).unwrap_or(""),
        "reply_to": cached.map(|message| message.reply_to.as_str()).unwrap_or(""),
        "cc": cached.map(|message| message.cc.as_str()).unwrap_or(""),
        "bcc": cached.map(|message| message.bcc.as_str()).unwrap_or(""),
        "message_id": cached.map(|message| message.message_id.as_str()).unwrap_or(""),
        "in_reply_to": header.in_reply_to,
        "references": cached.map(|message| message.references.as_str()).unwrap_or(""),
        "subject": cached.map(|message| message.subject.as_str()).unwrap_or(header.subject.as_str()),
        "preview": cached.map(|message| message.preview.as_str()).unwrap_or(""),
        "body": cached.map(|message| message.body.as_str()).unwrap_or(""),
        "body_html": cached.and_then(|message| message.body_html.as_deref()).unwrap_or(""),
        // Where the plain body's quoted tail starts (UTF-16 offset), so clients
        // can fold it; null when there is none. HTML bodies mark theirs inline.
        "body_quote_start": cached.and_then(|message| quote::plain_quote_start(&message.body)),
        // No cached body means the on-demand IMAP fetch failed (auth/network)
        // or is still filling in the background — not that the message is
        // empty. Clients offer a retry / re-read for these.
        "body_missing": cached.is_none(),
        "date": cached.map(|message| message.date).unwrap_or(header.date),
        "unread": !header.seen,
        "starred": header.starred,
        "has_attachments": cached.is_some_and(|message| message.has_attachments()),
        "attachments": cached
            .map(|message| serde_json::to_value(&message.attachments).unwrap_or_else(|_| json!([])))
            .unwrap_or_else(|| json!([])),
        "reply": reply,
    })
}

#[cfg(test)]
mod tests;
