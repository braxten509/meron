//! The AI sorting commands, shared by the desktop sidecar (`aiSort.<name>`) and
//! the phone (`mail.<name>` names, mapped in `protocol`). Keys never come back
//! out: only whether one is saved and its last 4 characters.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{AiError, ErrorKind, PROVIDERS, key_hint};
use crate::engine::Engine;
use crate::engine::ai_sort as pipeline;
use crate::store;

fn provider_param(params: &Value) -> anyhow::Result<String> {
    let provider = params
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    anyhow::ensure!(
        PROVIDERS.contains(&provider.as_str()),
        "unknown AI service: {provider}"
    );
    Ok(provider)
}

fn key_state(key: Option<&String>) -> Value {
    match key {
        Some(key) => json!({ "has_key": true, "key_hint": key_hint(key) }),
        None => json!({ "has_key": false, "key_hint": Value::Null }),
    }
}

fn check_result(result: Result<(), AiError>) -> Value {
    match result {
        Ok(()) => json!({ "ok": true }),
        Err(err) => json!({ "ok": false, "kind": err.kind, "message": err.message }),
    }
}

/// Run one command: `get`, `set`, `setKey`, `checkKey`, `models`, `estimate`,
/// `run`, `stop`, `status`, `moveBack`, `lessons` or `forgetLesson`.
pub async fn dispatch(engine: &Arc<Engine>, command: &str, params: &Value) -> anyhow::Result<Value> {
    match command {
        "get" => {
            let config = store::ai_sort_config(&engine.db.lock().unwrap())?;
            let keys = pipeline::loaded_keys(engine).await;
            Ok(json!({
                "config": config,
                "keys": {
                    "openrouter": key_state(keys.get("openrouter")),
                    "tinfoil": key_state(keys.get("tinfoil")),
                },
                "last_error": engine.ai_sort.last_error(),
            }))
        }
        "set" => {
            let config: store::AiSortConfig =
                serde_json::from_value(params.get("config").cloned().unwrap_or_default())?;
            let keys = pipeline::loaded_keys(engine).await;
            let config = {
                let db = engine.db.lock().unwrap();
                let mut config = store::clean_ai_sort_config(&db, config)?;
                // Can't be on without a key for the chosen service.
                if !keys.contains_key(&config.provider) {
                    config.enabled = false;
                }
                store::set_ai_sort_config(&db, config)?
            };
            if config.enabled
                && config.provider == "tinfoil"
                && let Some(key) = keys.get("tinfoil")
            {
                pipeline::warm_tinfoil(engine, key);
            }
            Ok(json!({ "config": config }))
        }
        "setKey" => {
            let provider = provider_param(params)?;
            let key = params
                .get("key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let key = (!key.is_empty()).then_some(key);
            pipeline::store_key(engine, &provider, key.clone()).await?;
            engine.ai_sort.clear_error();
            if key.is_none() {
                // Sorting can't stay on without the key it runs on.
                let db = engine.db.lock().unwrap();
                let mut config = store::ai_sort_config(&db)?;
                if config.enabled && config.provider == provider {
                    config.enabled = false;
                    store::set_ai_sort_config(&db, config)?;
                }
            }
            Ok(key_state(key.as_ref()))
        }
        "checkKey" => {
            let provider = provider_param(params)?;
            let Some(key) = pipeline::loaded_keys(engine).await.get(&provider).cloned() else {
                return Ok(check_result(Err(AiError::new(
                    ErrorKind::Key,
                    "Add an API key first.",
                ))));
            };
            let result = if provider == "tinfoil" {
                check_tinfoil(engine, &key).await
            } else {
                tokio::task::spawn_blocking(move || super::openrouter::check_key(&key))
                    .await
                    .unwrap_or_else(|_| Err(AiError::new(ErrorKind::Other, "The check stopped.")))
            };
            match &result {
                Ok(()) => engine.ai_sort.clear_error(),
                Err(err) if err.kind != ErrorKind::Other => engine.ai_sort.set_error(err),
                Err(_) => {}
            }
            Ok(check_result(result))
        }
        "models" => {
            let provider = provider_param(params)?;
            let models = pipeline::models(engine, &provider)
                .await
                .map_err(|err| anyhow::anyhow!("{}", err.message))?;
            // No default: the user picks a model.
            Ok(json!({ "models": models, "default": Value::Null }))
        }
        "estimate" => pipeline::estimate(engine).await,
        "run" => {
            let count = pipeline::run_existing(engine).await?;
            Ok(json!({ "count": count }))
        }
        "stop" => {
            pipeline::stop_existing(engine)?;
            Ok(json!({ "ok": true }))
        }
        "status" => {
            let log = store::ai_sort_log(&engine.db.lock().unwrap())?;
            let running = *engine.ai_sort.job.lock().unwrap();
            Ok(json!({
                "log": log,
                "running": running,
                "last_error": engine.ai_sort.last_error(),
            }))
        }
        "moveBack" => {
            let id = params
                .get("id")
                .and_then(|id| id.as_i64().or_else(|| id.as_str()?.parse().ok()))
                .ok_or_else(|| anyhow::anyhow!("missing id"))?;
            pipeline::move_back(engine, id).await?;
            Ok(json!({ "ok": true }))
        }
        "lessons" => lessons(engine),
        "forgetLesson" => {
            let id = params
                .get("id")
                .and_then(|id| id.as_i64().or_else(|| id.as_str()?.parse().ok()))
                .ok_or_else(|| anyhow::anyhow!("missing id"))?;
            store::ai_lesson_forget(&engine.db.lock().unwrap(), id)?;
            // The email loses its lesson tag on the server too, so the
            // reader's other devices forget it as well.
            pipeline::push_lesson_tags(engine);
            lessons(engine)
        }
        other => Err(anyhow::anyhow!("unknown AI sorting command: {other}")),
    }
}

/// What the reader taught by moving mail by hand, newest first. Each lesson:
/// `sender`, `subject`, `place` (`spam`, `inbox` or `meron`), the Meron
/// folder's current name in `place_name`, and `used`: false while the AI
/// can't follow it (its Meron folder isn't on the AI's list, or the account
/// has no Spam folder).
fn lessons(engine: &Arc<Engine>) -> anyhow::Result<Value> {
    let db = engine.db.lock().unwrap();
    let config = store::ai_sort_config(&db)?;
    let listed = config.meron_ids();
    let mut out = Vec::new();
    for lesson in store::ai_lessons(&db)? {
        let Some(place) = store::LessonPlace::from_code(&lesson.place) else {
            continue;
        };
        let (kind, place_name, used) = match &place {
            store::LessonPlace::Spam => (
                "spam",
                String::new(),
                store::folder_for_role(&db, &lesson.account, "junk")?.is_some(),
            ),
            store::LessonPlace::Inbox => ("inbox", String::new(), true),
            store::LessonPlace::Meron(id) => {
                let name = store::meron_folder_name(&db, id)?;
                let used = name.is_some() && listed.contains(id);
                ("meron", name.unwrap_or_default(), used)
            }
        };
        let sender = match (lesson.from_name.trim(), lesson.from_addr.trim()) {
            ("", addr) => addr.to_string(),
            (name, "") => name.to_string(),
            (name, addr) => format!("{name} <{addr}>"),
        };
        out.push(json!({
            "id": lesson.id,
            "account": lesson.account,
            "sender": sender,
            "subject": lesson.subject,
            "place": kind,
            "place_name": place_name,
            "used": used,
            "at": lesson.at,
        }));
    }
    Ok(json!({ "lessons": out }))
}

/// Verify the enclave, then spend one token through it.
async fn check_tinfoil(engine: &Arc<Engine>, key: &str) -> Result<(), AiError> {
    let client = pipeline::tinfoil_client(engine, key).await?;
    let config = store::ai_sort_config(&engine.db.lock().unwrap())
        .map_err(|_| AiError::new(ErrorKind::Other, "Couldn't read the setting."))?;
    let model = if config.provider == "tinfoil" && !config.model.is_empty() {
        config.model
    } else {
        pipeline::models(engine, "tinfoil")
            .await?
            .into_iter()
            .find(|model| !model.reasoning)
            .map(|model| model.id)
            .ok_or_else(|| AiError::new(ErrorKind::Other, "Tinfoil lists no models."))?
    };
    super::tinfoil::chat(&client, super::probe_body(&model))
        .await
        .map(|_| ())
}
