//! Local folders in the sidecar: requests for the local account, and moves between it and server accounts.
//!
//! The local account's mail lives in the archive (`store::archive`), behind an [`Engine`] of its
//! own that has no server accounts. Reads go through the same list and thread code as server mail,
//! pointed at that engine; writes never leave the machine, except an upload back to a server
//! account.

mod mailbox;
use mailbox::local_mail;

use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

use meron_core::engine::{Engine, EngineHost};
use meron_core::protocol::Request;
use meron_core::store::LOCAL_ACCOUNT_ID;
use meron_core::{engine, imap, mail_model, parse, secrets, store, thread_list, thread_read};

use crate::Writer;
use crate::sidecar::params::*;

/// Messages fetched, archived and removed from the server per round, so a large
/// move holds a bounded amount of mail in memory and an interruption leaves
/// every finished round complete.
const ARCHIVE_BATCH: usize = 25;

/// Methods that act on one account's mail. Addressed to the local account,
/// they are served here; everything else about it (preferences, order) is an
/// ordinary account row in the store.
const LOCAL_MAIL_METHODS: &[&str] = &[
    "folders.list",
    "folders.create",
    "folders.delete",
    "folders.rename",
    "folders.setNotify",
    "folders.archive",
    "messages.emptyFolder",
    "messages.recent",
    "messages.sync",
    "messages.read",
    "messages.thread",
    "messages.threadHeaders",
    "messages.markRead",
    "messages.markStarred",
    "messages.setTags",
    "tags.list",
    "messages.markAllRead",
    "messages.delete",
    "messages.move",
    "messages.copy",
    "messages.exportUids",
    "messages.saveRaw",
    "messages.saveRawBatch",
    "watch.start",
    "watch.stop",
];

struct ArchiveHost;

impl EngineHost for ArchiveHost {
    fn open_db(&self) -> anyhow::Result<rusqlite::Connection> {
        store::open_archive()
    }

    fn apply_secret(&self, _: &rusqlite::Connection, _: &str, _: &mut imap::Creds) {}

    fn store_secret(
        &self,
        _: &rusqlite::Connection,
        _: &str,
        _: &secrets::Secrets,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

/// The archive engine, opened on first use so a profile without local folders
/// never creates `archive.db`.
static ARCHIVE: Mutex<Option<Arc<Engine>>> = Mutex::new(None);

fn archive(main: &Arc<Engine>) -> anyhow::Result<Arc<Engine>> {
    let mut slot = ARCHIVE.lock().unwrap();
    if let Some(archive) = slot.as_ref() {
        return Ok(archive.clone());
    }
    // Checked under the lock removal holds, so a request racing the removal of
    // local folders cannot bring back an empty archive behind it.
    if !store::has_local_account(&main.db.lock().unwrap())? {
        anyhow::bail!("Local folders are not set up");
    }
    let archive = Arc::new(Engine::for_archive(Box::new(ArchiveHost))?);
    {
        let db = archive.db.lock().unwrap();
        match store::repair_local_index(&db, &parse::media_root()) {
            Ok(0) => {}
            Ok(repaired) => eprintln!("meron-core: reindexed {repaired} archived messages"),
            Err(error) => eprintln!("meron-core: archive index repair failed: {error:#}"),
        }
        store::sync_local_identity(&main.db.lock().unwrap(), &db)?;
    }
    *slot = Some(archive.clone());
    Ok(archive)
}

/// Whether this request is for local folders.
pub(crate) fn handles(request: &Request) -> bool {
    let parameters = &request.params;
    let account = parameters.get("account").and_then(Value::as_str);
    match request.method.as_str() {
        "account.addLocal"
        | "account.localStats"
        | "account.localBackup"
        | "account.localRestore" => true,
        "account.remove" => {
            account.or_else(|| parameters.get("id").and_then(Value::as_str))
                == Some(LOCAL_ACCOUNT_ID)
        }
        "messages.move" | "messages.copy"
            if parameters.get("target_account").and_then(Value::as_str)
                == Some(LOCAL_ACCOUNT_ID) =>
        {
            true
        }
        method => account == Some(LOCAL_ACCOUNT_ID) && LOCAL_MAIL_METHODS.contains(&method),
    }
}

pub(crate) async fn dispatch(
    main: &Arc<Engine>,
    command_request: &Request,
    out: &Writer,
) -> anyhow::Result<Value> {
    let parameters = &command_request.params;
    match command_request.method.as_str() {
        // Create the local account (once) and its archive.
        "account.addLocal" => {
            let name = required_string(parameters, "name")
                .ok()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Local Folders".to_string());
            let created = store::add_local_account(&main.db.lock().unwrap(), &name)?;
            let archive = archive(main)?;
            // Start with one folder, so there is somewhere to move mail and a
            // folder to open. The UI names it in the user's language.
            if created {
                let folder = required_string(parameters, "folder")
                    .ok()
                    .filter(|folder| !folder.trim().is_empty())
                    .unwrap_or_else(|| "Archive".to_string());
                let db = archive.db.lock().unwrap();
                if store::get_folders(&db, LOCAL_ACCOUNT_ID)?.is_empty() {
                    store::create_local_folder(&db, &folder)?;
                }
            }
            Ok(json!({ "ok": true, "created": created, "id": LOCAL_ACCOUNT_ID }))
        }

        "account.localStats" => {
            let archive = archive(main)?;
            let mut stats = store::local_storage_stats(&archive.db.lock().unwrap())?;
            stats["path"] = json!(archive_file().display().to_string());
            Ok(stats)
        }

        "account.localBackup" => {
            let path = required_string(parameters, "path")?;
            let passphrase = required_string(parameters, "passphrase")?;
            let archive = archive(main)?;
            let db = archive.db.lock().unwrap();
            store::backup_archive_to(&db, std::path::Path::new(&path), &passphrase)?;
            Ok(json!({ "ok": true, "path": path }))
        }

        // Merge a backup into the local folders, creating them if needed.
        "account.localRestore" => {
            let path = required_string(parameters, "path")?;
            let passphrase = required_string(parameters, "passphrase")?;
            store::add_local_account(&main.db.lock().unwrap(), "Local Folders")?;
            let archive = archive(main)?;
            let restored = {
                let archive = archive.clone();
                tokio::task::spawn_blocking(move || {
                    store::restore_local_backup(
                        &archive.db.lock().unwrap(),
                        std::path::Path::new(&path),
                        &passphrase,
                        &parse::media_root(),
                    )
                })
                .await??
            };
            Ok(json!({ "ok": true, "restored": restored }))
        }

        // Deletes the archive and every message in it. The UI asks first, and
        // says it cannot be undone. The account goes only once the file is
        // gone: otherwise adding local folders again would bring the mail back.
        "account.remove" => {
            // Held to the end, so nothing reopens the archive while it goes.
            let mut slot = ARCHIVE.lock().unwrap();
            // A request still holding the engine (a long move) keeps the
            // database open and could write to it after it is deleted.
            if slot
                .as_ref()
                .is_some_and(|archive| Arc::strong_count(archive) > 1)
            {
                anyhow::bail!("Local folders are busy. Try again once the current move finishes.");
            }
            drop(slot.take());
            let path = archive_file();
            for suffix in ["", "-wal", "-shm"] {
                let mut file = path.clone().into_os_string();
                file.push(suffix);
                match std::fs::remove_file(&file) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(anyhow::anyhow!(
                            "Could not delete {}: {error}",
                            std::path::Path::new(&file).display()
                        ));
                    }
                }
            }
            store::delete_account(&main.db.lock().unwrap(), LOCAL_ACCOUNT_ID)?;
            drop(slot);
            parse::remove_account_media(&parse::media_root(), LOCAL_ACCOUNT_ID);
            Ok(json!({ "ok": true }))
        }

        "messages.move" | "messages.copy"
            if required_string(parameters, "account").ok().as_deref() != Some(LOCAL_ACCOUNT_ID) =>
        {
            archive_from_server(main, command_request).await
        }

        _ => {
            let archive = archive(main)?;
            local_mail(main, &archive, command_request, out).await
        }
    }
}

fn archive_file() -> std::path::PathBuf {
    std::env::var("MERON_ARCHIVE_DB")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| store::application_directory().join("archive.db"))
}

/// The UIDs a mail request names: explicit `uids`, one `uid`, or every message
/// of `thread_key` in `folder`.
fn request_uids(
    conn: &rusqlite::Connection,
    parameters: &Value,
    folder: &str,
) -> anyhow::Result<Vec<u32>> {
    let (thread_key, subject_filter) =
        store::split_thread_key(&required_string(parameters, "thread_key").unwrap_or_default());
    let uid = optional_message_uid(parameters)?;
    let explicit_uids = explicit_message_uids(parameters)?;
    store::resolve_message_uids(
        conn,
        parameters
            .get("account")
            .and_then(Value::as_str)
            .unwrap_or(LOCAL_ACCOUNT_ID),
        folder,
        &thread_key,
        subject_filter.as_deref(),
        uid,
        &explicit_uids,
    )
}

/// The thread id a mutation reports as changed, built as the server path does.
fn changed_thread_id(parameters: &Value, account: &str, folder: &str) -> String {
    let (thread_key, subject_filter) =
        store::split_thread_key(&required_string(parameters, "thread_key").unwrap_or_default());
    if thread_key.is_empty() {
        return parameters
            .get("uid")
            .and_then(Value::as_u64)
            .map(|uid| format!("{account}#{folder}#{uid}"))
            .unwrap_or_default();
    }
    let key = subject_filter
        .as_deref()
        .map(|subject| store::branch_compound_key(&thread_key, subject))
        .unwrap_or(thread_key);
    mail_model::format_thread_id(account, folder, &key)
}

fn request_folder(parameters: &Value) -> anyhow::Result<String> {
    let folder =
        required_string(parameters, "folder").or_else(|_| required_string(parameters, "name"))?;
    store::normalize_local_folder_name(&folder)
}

/// Write an exported message readable by the user only, as the server export does.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut output = options
        .open(path)
        .map_err(|error| anyhow::anyhow!("open message export {}: {error}", path.display()))?;
    output
        .write_all(bytes)
        .map_err(|error| anyhow::anyhow!("write message export {}: {error}", path.display()))
}

/// The folder a local request names, in the form local folders are stored
/// under (see `store::normalize_local_folder_name`).
fn canon_local(parameters: &Value) -> String {
    store::canon_local_folder(&required_string(parameters, "folder").unwrap_or_default())
}

/// Tag counts in local folders, for the tag list.
pub(crate) fn tag_counts(main: &Arc<Engine>) -> std::collections::BTreeMap<String, u32> {
    let counts = archive(main)
        .and_then(|archive| store::tag_counts(&archive.db.lock().unwrap(), Some(LOCAL_ACCOUNT_ID)));
    counts.unwrap_or_default()
}

/// Rename or delete a tag in local folders. See [`store::replace_tag`]. Nothing
/// to do without local folders, and the archive is not created for it.
pub(crate) fn replace_tag(main: &Arc<Engine>, tag: &str, to: Option<&str>) -> anyhow::Result<()> {
    if !store::has_local_account(&main.db.lock().unwrap())? {
        return Ok(());
    }
    let archive = archive(main)?;
    store::replace_tag(&archive.db.lock().unwrap(), tag, to, false)?;
    Ok(())
}

/// Starred threads kept in local folders, for the cross-account Starred view.
/// Empty when there are no local folders; the archive is not created for it.
pub(crate) fn starred_cards(main: &Arc<Engine>, max_threads: u32) -> Vec<Value> {
    let has_local = store::has_local_account(&main.db.lock().unwrap()).unwrap_or(false);
    if !has_local {
        return Vec::new();
    }
    let cards = archive(main).and_then(|archive| {
        mail_model::starred_thread_cards(&archive.db.lock().unwrap(), max_threads)
    });
    cards.unwrap_or_else(|error| {
        eprintln!("meron-core: local starred threads: {error:#}");
        Vec::new()
    })
}

/// Keep the archive's view of the user's addresses and the local account's
/// preferences current before a thread read renders with them.
fn refresh_identity(main: &Arc<Engine>, archive: &Arc<Engine>) {
    let result = store::sync_local_identity(&main.db.lock().unwrap(), &archive.db.lock().unwrap());
    if let Err(error) = result {
        eprintln!("meron-core: local identity sync failed: {error:#}");
    }
}

/// Move (or copy) server mail into a local folder: download each original, keep
/// it in the archive and read it back, and only then move the server copy to
/// Trash. Runs in rounds of [`ARCHIVE_BATCH`], each complete before the next.
async fn archive_from_server(main: &Arc<Engine>, request: &Request) -> anyhow::Result<Value> {
    let parameters = &request.params;
    let moving = request.method == "messages.move";
    let account = required_string(parameters, "account")?;
    let folder = engine::canon_folder(
        &required_string(parameters, "folder").unwrap_or_else(|_| "INBOX".to_string()),
    );
    let target_folder =
        store::normalize_local_folder_name(&required_string(parameters, "target_folder")?)?;
    let thread_id = changed_thread_id(parameters, &account, &folder);
    let archive = archive(main)?;
    if !store::folder_exists(
        &archive.db.lock().unwrap(),
        LOCAL_ACCOUNT_ID,
        &target_folder,
    )? {
        return Err(anyhow::anyhow!("Folder not found: {target_folder}"));
    }
    let uids = request_uids(&main.db.lock().unwrap(), parameters, &folder)?;
    let source = store::archive_source_label(&main.db.lock().unwrap(), &account, &folder);
    let media_root = parse::media_root();

    let mut target_uids = Vec::with_capacity(uids.len());
    let mut trash_folder = None;
    for chunk in uids.chunks(ARCHIVE_BATCH) {
        let messages = main
            .with_read_session(&account, |session| {
                let folder = folder.clone();
                let uids = chunk.to_vec();
                Box::pin(
                    async move { imap::fetch_raw_messages_for_copy(session, &folder, &uids).await },
                )
            })
            .await?;
        engine::require_complete_fetch(chunk, &messages)?;
        let inputs = {
            let db = main.db.lock().unwrap();
            messages
                .iter()
                .map(|message| {
                    let (thread_key, date) =
                        store::cached_thread_key_and_date(&db, &account, &folder, message.uid)?;
                    Ok((message, thread_key, date))
                })
                .collect::<anyhow::Result<Vec<_>>>()?
        };
        let stored = {
            let archive = archive.clone();
            let target_folder = target_folder.clone();
            let source = source.clone();
            let media_root = media_root.clone();
            let inputs = inputs
                .into_iter()
                .map(|(message, thread_key, date)| {
                    (
                        message.raw.clone(),
                        message.seen,
                        message.starred,
                        thread_key,
                        date,
                    )
                })
                .collect::<Vec<_>>();
            tokio::task::spawn_blocking(move || {
                let inputs = inputs
                    .iter()
                    .map(
                        |(raw, seen, starred, thread_key, date)| store::ArchiveInput {
                            raw,
                            seen: *seen,
                            starred: *starred,
                            thread_key: thread_key.clone(),
                            date_fallback: *date,
                            source: source.clone(),
                        },
                    )
                    .collect::<Vec<_>>();
                store::archive_messages(
                    &archive.db.lock().unwrap(),
                    &target_folder,
                    &inputs,
                    &media_root,
                )
            })
            .await??
        };
        target_uids.extend(stored);
        let copies = std::collections::BTreeMap::from([(folder.clone(), chunk.to_vec())]);
        let tags = store::carried_tags(&main.db.lock().unwrap(), &account, &copies)?;
        store::apply_carried_tags(&archive.db.lock().unwrap(), LOCAL_ACCOUNT_ID, tags, false)?;
        if moving {
            let removed = engine::delete_to_trash(main, &account, &folder, chunk)
                .await
                .map_err(|error| {
                    error.context("Kept the messages in the local folder but could not remove them from the server")
                })?;
            store::delete_messages_by_uid(&main.db.lock().unwrap(), &account, &folder, chunk)?;
            if let Some(trash) = removed {
                trash_folder = Some(trash.folder);
            }
        }
    }

    // The Trash copies are on the server now; show them without a full sync.
    if let Some(trash) = &trash_folder
        && let Some(refresh) =
            engine::refresh_written_folder(main, &account, trash, 50.max(uids.len() as u32)).await
    {
        let batch = &refresh.batch;
        let db = main.db.lock().unwrap();
        let _ = store::ensure_folder(&db, &account, trash)
            .and_then(|_| store::upsert_messages(&db, &account, trash, &batch.messages))
            .and_then(|_| {
                store::set_folder_state(&db, &account, trash, batch.uidvalidity, batch.uid_next)
            });
    }

    let count = target_uids.len();
    let mut result = mail_model::mutation_result(
        json!({
            "ok": true,
            "moved": if moving { count } else { 0 },
            "copied": if moving { 0 } else { count },
            "source_folder": folder,
            "target_account": LOCAL_ACCOUNT_ID,
            "target_folder": target_folder,
        }),
        &main.db.lock().unwrap(),
        &account,
        &thread_id,
        &folder,
        trash_folder.as_deref(),
        None,
        None,
        moving,
    )?;
    let local_unread = store::get_folder_unread(
        &archive.db.lock().unwrap(),
        LOCAL_ACCOUNT_ID,
        &target_folder,
    )?;
    result["folder_unreads"][LOCAL_ACCOUNT_ID] = json!({ target_folder.clone(): local_unread });
    if let Some(counts) = result["folder_counts"].as_array_mut() {
        counts.push(json!({
            "account_id": LOCAL_ACCOUNT_ID,
            "folder_id": target_folder,
            "unread": local_unread,
        }));
    }
    Ok(result)
}
