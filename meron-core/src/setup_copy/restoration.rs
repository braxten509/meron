//! Collects and restores a device's account setup and settings.
//! The receiving device records recovery information before replacing its configuration.
//! The setup screen can inspect or restore that saved configuration.

use super::*;
use base64::Engine as _;

pub(super) async fn counts(engine: &Arc<Engine>) -> Counts {
    let (accounts, folders) = {
        let db = engine.db.lock().unwrap();
        let accounts: u32 = db
            .query_row(
                "SELECT COUNT(*) FROM accounts WHERE engine <> 'local'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let folders = store::list_meron_folders(&db)
            .map(|folders| folders.len() as u32)
            .unwrap_or(0);
        (accounts, folders)
    };
    let keys = crate::engine::ai_sort::loaded_keys(engine).await.len() as u32;
    Counts {
        accounts,
        folders,
        keys,
    }
}

/// This device's setup as a copy carries it: accounts with their secrets,
/// shared settings, Meron folders, tasks, calendar, tags, and the AI keys.
pub async fn collect_payload(engine: &Arc<Engine>, device: &str) -> Result<SetupPayload> {
    let ids: Vec<String> = {
        let db = engine.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT id FROM accounts WHERE engine <> 'local'")?;
        stmt.query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    let keys = crate::engine::ai_sort::loaded_keys(engine).await;
    let engine = engine.clone();
    let device = device.to_string();
    tokio::task::spawn_blocking(move || {
        // Secrets first, without the store's lock (a keychain can be slow).
        // One that can't be read stops it: a copy (or safety copy) without
        // that password would lose it.
        let secrets: HashMap<String, Secrets> = ids
            .into_iter()
            .map(|id| {
                let secret = engine.host.load_account_secret(&engine.db, &id)?;
                anyhow::Ok((id, secret))
            })
            .collect::<Result<_>>()?;
        let mut backup = crate::backup::collect(&engine.db.lock().unwrap(), true, &|id| {
            secrets.get(id).cloned().unwrap_or_default()
        })?;
        backup.settings.retain(|key, _| is_shared_setting(key));
        let photo_root = engine.host.account_media_root();
        let mut photo_bytes = 0usize;
        let photos = backup
            .accounts
            .iter()
            .filter_map(|account| {
                let bytes = crate::account_photos::read_for_copy(&photo_root, &account.avatar_url)?;
                photo_bytes += bytes.len();
                (photo_bytes <= crate::account_photos::COPY_TOTAL_BYTES)
                    .then(|| (account.id.clone(), STANDARD.encode(bytes)))
            })
            .collect();
        Ok(SetupPayload {
            version: PAYLOAD_VERSION,
            device,
            backup,
            keys: keys.into_iter().collect(),
            photos,
        })
    })
    .await?
}

/// The erased side: check the copy, then replace this setup with it.
pub(super) async fn receive(engine: &Arc<Engine>, payload: Value, device: &str) -> Result<Value> {
    let payload: SetupPayload = serde_json::from_value(payload).map_err(|_| fail("bad_copy"))?;
    if payload.version != PAYLOAD_VERSION || payload.device == device {
        return Err(fail("bad_copy"));
    }
    if payload
        .backup
        .accounts
        .iter()
        .any(|account| account.id.trim().is_empty())
    {
        return Err(fail("bad_copy"));
    }
    replace_setup(engine, payload, device).await
}

/// Save a safety copy of this setup, wipe it and apply `payload`. Nothing is
/// erased when the safety copy can't be saved; the wipe and the apply are one
/// transaction.
pub async fn replace_setup(
    engine: &Arc<Engine>,
    payload: SetupPayload,
    device: &str,
) -> Result<Value> {
    save_safety(engine, device).await.map_err(|restore_error| {
        crate::mlog!(
            crate::log::Level::Warn,
            "setupCopy",
            "no safety copy, nothing erased: {restore_error:#}"
        );
        fail("safety_failed")
    })?;
    // Today's account secrets, read before anything changes, to put back if
    // the new ones can't all be saved. One that can't be read stops the copy
    // (nothing erased): putting "none" back would delete that password.
    let old_secrets: Vec<(String, Secrets)> = {
        let engine = engine.clone();
        tokio::task::spawn_blocking(move || {
            let ids: Vec<String> = {
                let db = engine.db.lock().unwrap();
                let mut statement =
                    db.prepare("SELECT id FROM accounts WHERE engine <> 'local'")?;
                let ids = statement
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                ids
            };
            ids.into_iter()
                .map(|id| {
                    let secret = engine.host.load_account_secret(&engine.db, &id)?;
                    anyhow::Ok((id, secret))
                })
                .collect::<Result<Vec<_>>>()
        })
        .await
        .map_err(anyhow::Error::from)
        .and_then(|read| read)
        .map_err(|restore_error| {
            crate::mlog!(
                crate::log::Level::Warn,
                "setupCopy",
                "could not read this device's passwords, nothing erased: {restore_error:#}"
            );
            fail("apply_failed")
        })?
    };
    let applied = {
        let engine = engine.clone();
        let backup = payload.backup.clone();
        tokio::task::spawn_blocking(move || {
            wipe_and_apply(
                &engine.db.lock().unwrap(),
                engine.host.as_ref(),
                &backup,
                &old_secrets,
            )
        })
        .await
    };
    // A crashed apply counts as failed: the transaction never committed.
    let (old_ids, summary) = applied
        .map_err(anyhow::Error::from)
        .and_then(|applied| applied)
        .map_err(|restore_error| {
            crate::mlog!(
                crate::log::Level::Warn,
                "setupCopy",
                "applying the copy failed, nothing changed: {restore_error:#}"
            );
            fail("apply_failed")
        })?;
    // The old accounts' warm state goes; same ids load their new settings.
    for id in &old_ids {
        engine.accounts.lock().await.remove(id);
        engine.clear_pool(id);
        crate::parse::remove_account_media(&engine.host.media_root(), id);
    }
    // The photos came along: save them here and point the accounts at them.
    for (id, data) in &payload.photos {
        if !payload
            .backup
            .accounts
            .iter()
            .any(|account| &account.id == id)
        {
            continue;
        }
        let saved = STANDARD
            .decode(data.as_bytes())
            .map_err(anyhow::Error::from)
            .and_then(|bytes| crate::account_photos::apply_copied(engine, id, &bytes));
        if let Err(restore_error) = saved {
            crate::mlog!(
                crate::log::Level::Warn,
                "setupCopy",
                "could not save the photo of {id}: {restore_error:#}"
            );
        }
    }
    // Load the copied accounts with their sign-ins now: the account list is
    // read right after the copy, and an account not loaded yet shows as
    // "needs reconnect" until the next restart.
    {
        let loaded = store::load_accounts(&engine.db.lock().unwrap()).unwrap_or_default();
        let mut accounts = engine.accounts.lock().await;
        for (id, mut creds) in loaded {
            engine
                .host
                .apply_secret(&engine.db.lock().unwrap(), &id, &mut creds);
            accounts.insert(id, creds);
        }
    }
    let mut secret_failures = 0;
    for provider in crate::ai_sort::PROVIDERS {
        let key = payload.keys.get(provider).cloned();
        if let Err(restore_error) = crate::engine::ai_sort::store_key(engine, provider, key).await {
            secret_failures += 1;
            crate::mlog!(
                crate::log::Level::Warn,
                "setupCopy",
                "could not save the {provider} key: {restore_error:#}"
            );
        }
    }
    {
        let db = engine.db.lock().unwrap();
        let _ = crate::proxy::load_global(&db);
    }
    // Photos that didn't come along (an older device sent the copy): ask
    // Google or Microsoft for them, behind the copy.
    {
        let engine = engine.clone();
        crate::account_photos::forget_tried();
        tokio::spawn(async move {
            crate::account_photos::fetch_missing(&engine).await;
        });
    }
    // The phone's screens listen for these (a no-op on the desktop).
    crate::ffi::emit_event("calendar.changed", json!({ "method": "setupCopy" }));
    crate::ffi::emit_event("tasks.changed", json!({ "method": "setupCopy" }));
    Ok(json!({
        "accounts": summary.accounts,
        "folders": summary.meron_folders,
        "keys": payload.keys.len(),
        "tasks": summary.tasks,
        "events": summary.events,
        "secret_failures": secret_failures,
    }))
}

/// Wipe this device's setup and apply `data`, in one transaction, with every account secret saved
/// before it commits (in the same transaction where the store keeps them, else in the keychain).
/// When a secret can't be saved, or the commit fails, the old secrets go back and nothing changes.
/// Answers with the old account ids.
fn wipe_and_apply(
    conn: &Connection,
    host: &dyn crate::engine::EngineHost,
    data: &BackupData,
    old_secrets: &[(String, Secrets)],
) -> Result<Applied> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let old_ids: Vec<String> = tx
        .prepare("SELECT id FROM accounts WHERE engine <> 'local'")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in &old_ids {
        store::delete_account_in(&tx, id)?;
    }
    let keys: Vec<String> = tx
        .prepare("SELECT key FROM settings")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for key in keys.iter().filter(|key| is_shared_setting(key)) {
        tx.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
    }
    for table in [
        "meron_folder_aliases",
        "meron_folder_uids",
        "meron_folder_entries",
        "meron_folders",
        "meron_marks",
        "tasks",
        "task_lists",
        "events",
    ] {
        tx.execute(&format!("DELETE FROM {table}"), [])?;
    }
    let mut data = data.clone();
    data.settings.retain(|key, _| is_shared_setting(key));
    let secrets: RefCell<Vec<(String, Secrets)>> = RefCell::new(Vec::new());
    let summary = crate::backup::apply_in(&tx, &data, &|_, id, secret| {
        secrets.borrow_mut().push((id.to_string(), secret.clone()));
        Ok(())
    })?;
    let secrets = secrets.into_inner();
    // From here on, every way out but a commit (an error, a panic) puts the
    // old secrets back; the transaction rolls back by itself.
    let mut restore = RestoreSecrets {
        host,
        conn,
        saved: Vec::new(),
        old: old_secrets,
        done: false,
    };
    for (id, secret) in &secrets {
        restore.saved.push(id.clone());
        host.store_secret(&tx, id, secret)
            .map_err(|restore_error| {
                restore_error.context("a password or sign-in token could not be saved")
            })?;
    }
    tx.commit()?;
    restore.done = true;
    // Old accounts the copy doesn't have: their secrets go.
    for id in old_ids
        .iter()
        .filter(|id| !secrets.iter().any(|(new, _)| new == *id))
    {
        host.delete_account_secret(id);
    }
    Ok((old_ids, summary))
}

/// Save this setup, locked with a fresh key kept with the other secrets. One
/// copy: the next replaces it.
async fn save_safety(engine: &Arc<Engine>, device: &str) -> Result<()> {
    let payload = collect_payload(engine, device).await?;
    let mine = counts(engine).await;
    let plain = serde_json::to_vec(&payload)?;
    let rng = SystemRandom::new();
    let mut key = [0u8; 32];
    let mut nonce = [0u8; 12];
    rng.fill(&mut key).map_err(|_| anyhow!("no randomness"))?;
    rng.fill(&mut nonce).map_err(|_| anyhow!("no randomness"))?;
    let sealing = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_256_GCM, &key).unwrap());
    let mut sealed = plain;
    sealing
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(b"meron-safety"),
            &mut sealed,
        )
        .map_err(|_| anyhow!("could not lock the safety copy"))?;
    // Two key slots, used in turn: the new key goes in the slot the saved copy
    // doesn't use, and the copy points at its slot. Until the new copy is
    // written (one row, one step) the old one still opens with its own key.
    let old_slot = safety_row(&engine.db.lock().unwrap())?.map(|row| safety_slot(&row));
    let slot = if old_slot.as_deref() == Some(SAFETY_KEY_NAME) {
        SAFETY_KEY_NAME_B
    } else {
        SAFETY_KEY_NAME
    };
    let engine2 = engine.clone();
    let encoded_key = STANDARD.encode(key);
    tokio::task::spawn_blocking(move || {
        engine2
            .host
            .store_named_secret(&engine2.db, slot, Some(&encoded_key))
    })
    .await??;
    let row = json!({
        "created_at": store::now_unix(),
        "counts": mine,
        "slot": slot,
        "nonce": STANDARD.encode(nonce),
        "data": STANDARD.encode(&sealed),
    });
    engine.db.lock().unwrap().execute(
        "INSERT INTO meta(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SAFETY_META_KEY, row.to_string()],
    )?;
    // The old copy is gone: so is its key. Best effort.
    if let Some(old) = old_slot.filter(|old| old != slot) {
        let engine2 = engine.clone();
        let _ = tokio::task::spawn_blocking(move || {
            engine2.host.store_named_secret(&engine2.db, &old, None)
        })
        .await;
    }
    Ok(())
}

/// The key slot a saved safety copy's key is in (older copies: the first).
fn safety_slot(row: &Value) -> String {
    match row.get("slot").and_then(Value::as_str) {
        Some(SAFETY_KEY_NAME_B) => SAFETY_KEY_NAME_B.to_string(),
        _ => SAFETY_KEY_NAME.to_string(),
    }
}

fn safety_row(conn: &Connection) -> Result<Option<Value>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![SAFETY_META_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(text.and_then(|text| serde_json::from_str(&text).ok()))
}

pub(super) fn safety_info(engine: &Arc<Engine>) -> Result<Value> {
    let row = safety_row(&engine.db.lock().unwrap())?;
    Ok(match row {
        Some(row) => json!({
            "exists": true,
            "created_at": row.get("created_at").cloned().unwrap_or(json!(0)),
            "counts": row.get("counts").cloned().unwrap_or(json!({})),
        }),
        None => json!({ "exists": false }),
    })
}

/// Put the safety copy back. The setup it replaces becomes the safety copy,
/// so this can be undone the same way.
pub(super) async fn restore_safety(engine: &Arc<Engine>, device: &str) -> Result<Value> {
    if current().is_some() {
        anyhow::bail!("Finish or cancel the copy first.");
    }
    let row = safety_row(&engine.db.lock().unwrap())?
        .ok_or_else(|| anyhow!("There is no safety copy."))?;
    let engine2 = engine.clone();
    let slot = safety_slot(&row);
    let key =
        tokio::task::spawn_blocking(move || engine2.host.load_named_secret(&engine2.db, &slot))
            .await??
            .ok_or_else(|| anyhow!("The safety copy's key is missing."))?;
    let key = STANDARD.decode(key)?;
    let nonce: [u8; 12] = b64_field(&row, "nonce")?
        .try_into()
        .map_err(|_| anyhow!("The safety copy is damaged."))?;
    let mut sealed = b64_field(&row, "data")?;
    let opening = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &key)
            .map_err(|_| anyhow!("The safety copy is damaged."))?,
    );
    let plain = opening
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(b"meron-safety"),
            &mut sealed,
        )
        .map_err(|_| anyhow!("The safety copy is damaged."))?;
    let payload: SetupPayload = serde_json::from_slice(plain)?;
    let result = replace_setup(engine, payload, device).await?;
    Ok(json!({ "ok": true, "replaced": true, "result": result }))
}
