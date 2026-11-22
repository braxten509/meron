//! Provider credentials used by inbox sorting.
//! The host owns keychain access. Its first load continues if a sync stops waiting.
//! Saving a key updates the cached provider state used by subsequent requests.

use super::*;

/// The keys, loaded from the host once (off the store's lock, as keychain
/// calls can hang). The load goes on in its own task, so a caller that stops
/// waiting doesn't lose it.
pub async fn loaded_keys(engine: &Arc<Engine>) -> HashMap<String, String> {
    let slot = engine.ai_sort.keys.clone();
    let engine = engine.clone();
    let task = tokio::spawn(async move {
        let mut guard = slot.lock().await;
        if let Some(keys) = guard.as_ref() {
            return keys.clone();
        }
        let (keys, complete) = tokio::task::spawn_blocking(move || {
            let mut keys = HashMap::new();
            let mut complete = true;
            for provider in ai::PROVIDERS {
                match engine
                    .host
                    .load_named_secret(&engine.db, &ai::key_name(provider))
                {
                    Ok(Some(key)) => {
                        keys.insert(provider.to_string(), key);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        complete = false;
                        warn(format_args!("could not load the {provider} key: {error:#}"));
                    }
                }
            }
            (keys, complete)
        })
        .await
        .unwrap_or_default();
        // A store that failed to answer is asked again next time.
        if complete {
            *guard = Some(keys.clone());
        }
        keys
    });
    task.await.unwrap_or_default()
}

/// Save or remove (`None`) a provider's key.
pub async fn store_key(
    engine: &Arc<Engine>,
    provider: &str,
    key: Option<String>,
) -> anyhow::Result<()> {
    let host_engine = engine.clone();
    let name = ai::key_name(provider);
    let value = key.clone();
    tokio::task::spawn_blocking(move || {
        host_engine
            .host
            .store_named_secret(&host_engine.db, &name, value.as_deref())
    })
    .await??;
    let _ = loaded_keys(engine).await;
    {
        let mut guard = engine.ai_sort.keys.lock().await;
        let keys = guard.get_or_insert_with(HashMap::new);
        match key {
            Some(key) => keys.insert(provider.to_string(), key),
            None => keys.remove(provider),
        };
    }
    if provider == "tinfoil" {
        *engine.ai_sort.tinfoil.lock().await = None;
    }
    Ok(())
}
