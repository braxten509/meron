//! Provider requests made by the inbox classifier.
//! The engine owns the shared client and model capability cache.
//! These helpers handle model discovery, request deadlines and retries.

use super::*;

pub(super) fn proxy_blocks_tinfoil() -> Option<AiError> {
    crate::proxy::global().is_some().then(|| {
        AiError::new(
            ErrorKind::Other,
            "Tinfoil can't be used while an app proxy is set.",
        )
    })
}

/// The verified Tinfoil client for `key`, verifying again past
/// [`ai::TINFOIL_TTL`] or after a failed check.
pub(crate) async fn tinfoil_client(
    engine: &Arc<Engine>,
    key: &str,
) -> Result<Arc<::tinfoil::Client>, AiError> {
    if let Some(error) = proxy_blocks_tinfoil() {
        return Err(error);
    }
    let hash = ai::key_hash(key);
    let mut slot = engine.ai_sort.tinfoil.lock().await;
    if let Some(cached) = slot.as_ref()
        && cached.key_hash == hash
        && cached.checked.elapsed() < ai::TINFOIL_TTL
    {
        return Ok(cached.client.clone());
    }
    let client = Arc::new(ai::tinfoil::connect(key).await?);
    *slot = Some(ai::TinfoilClient {
        client: client.clone(),
        checked: Instant::now(),
        key_hash: hash,
    });
    Ok(client)
}

/// Verify Tinfoil ahead of time, so new mail doesn't spend its 20 s on it.
pub(crate) fn warm_tinfoil(engine: &Arc<Engine>, key: &str) {
    let engine = engine.clone();
    let key = key.to_string();
    tokio::spawn(async move {
        {
            let slot = engine.ai_sort.tinfoil.lock().await;
            if slot.as_ref().is_some_and(|cached| {
                cached.key_hash == ai::key_hash(&key) && cached.checked.elapsed() < ai::TINFOIL_TTL
            }) {
                return;
            }
        }
        if let Err(error) = tinfoil_client(&engine, &key).await {
            warn(format_args!("tinfoil check: {error}"));
            if error.kind == ErrorKind::Verify {
                engine.ai_sort.set_error(&error);
            }
        }
    });
}

/// Whether `model` is known to take a JSON schema.
pub(super) fn model_structured(engine: &Engine, provider: &str, model: &str) -> bool {
    engine
        .ai_sort
        .cached_models(provider)
        .and_then(|models| models.into_iter().find(|it| it.id == model))
        .is_some_and(|it| it.structured)
}

/// One request, no retry.
async fn request(
    engine: &Arc<Engine>,
    provider: &str,
    key: &str,
    structured: bool,
    body: serde_json::Value,
) -> Result<String, AiError> {
    if provider == "tinfoil" {
        let client = tinfoil_client(engine, key).await?;
        let result = ai::tinfoil::chat(&client, body).await;
        if let Err(error) = &result
            && ai::tinfoil::needs_reverify(error)
        {
            *engine.ai_sort.tinfoil.lock().await = None;
        }
        return result;
    }
    let key = key.to_string();
    // The blocking call has its own timeout and never touches the store; a
    // caller that stopped waiting just drops its answer.
    tokio::task::spawn_blocking(move || ai::openrouter::chat(&key, body, structured))
        .await
        .map_err(|_| AiError::new(ErrorKind::Other, "The AI request stopped."))?
}

/// A request and, when it failed in a way worth it and time is left, one
/// retry, after the service's `Retry-After`.
pub(super) async fn request_with_retry(
    engine: &Arc<Engine>,
    provider: &str,
    key: &str,
    structured: bool,
    body: serde_json::Value,
    deadline: Instant,
) -> Result<String, AiError> {
    match request(engine, provider, key, structured, body.clone()).await {
        Ok(text) => Ok(text),
        Err(error) if error.retry => {
            let wait = error.retry_after.unwrap_or(Duration::from_millis(500));
            let left = deadline.saturating_duration_since(Instant::now());
            if left <= wait + Duration::from_secs(2) {
                return Err(error);
            }
            tokio::time::sleep(wait).await;
            request(engine, provider, key, structured, body).await
        }
        Err(error) => Err(error),
    }
}

/// How long a sort waits for the model list to learn if its model is a
/// decision model.
const MODEL_LIST_WAIT: Duration = Duration::from_secs(5);

/// Whether `model` is an OpenRouter decision model, from the day's full model
/// list (fetched when not cached). A list that can't be had fails the batch
/// rather than send a decision model to the chat endpoint.
pub(super) async fn model_decision(
    engine: &Arc<Engine>,
    provider: &str,
    model: &str,
) -> Result<bool, AiError> {
    if provider != "openrouter" {
        return Ok(false);
    }
    if engine.ai_sort.cached_models(provider).is_none() {
        let _ = tokio::time::timeout(MODEL_LIST_WAIT, models(engine, provider)).await;
    }
    let Some(list) = engine.ai_sort.cached_models(provider) else {
        return Err(AiError::network("Couldn't load the model list."));
    };
    Ok(list.iter().any(|it| it.id == model && it.decision))
}

/// One decision request, no retry.
async fn decide_request(key: &str, body: serde_json::Value) -> Result<serde_json::Value, AiError> {
    let key = key.to_string();
    tokio::task::spawn_blocking(move || ai::openrouter::decide(&key, body))
        .await
        .map_err(|_| AiError::new(ErrorKind::Other, "The AI request stopped."))?
}

/// A decision request and, as for chat, one retry when worth it and time is left.
async fn decide_with_retry(
    key: &str,
    body: serde_json::Value,
    deadline: Instant,
) -> Result<serde_json::Value, AiError> {
    match decide_request(key, body.clone()).await {
        Ok(value) => Ok(value),
        Err(error) if error.retry => {
            let wait = error.retry_after.unwrap_or(Duration::from_millis(500));
            let left = deadline.saturating_duration_since(Instant::now());
            if left <= wait + Duration::from_secs(2) {
                return Err(error);
            }
            tokio::time::sleep(wait).await;
            decide_request(key, body).await
        }
        Err(error) => Err(error),
    }
}

/// Ask a decision model about each email of a batch, [`ai::DECISION_PARALLEL`]
/// at a time. A failed request fails the batch, as a failed chat request does.
pub(super) async fn decide_batch_by_decision(
    config: &AiSortConfig,
    key: &str,
    folders: &[PromptFolder],
    batch: &[AiSortCandidate],
    emails: &[PromptEmail],
    spam: Option<&ai::SpamChoice>,
    deadline: Instant,
) -> Result<Vec<AiSortDecision>, AiError> {
    use futures::StreamExt;
    // Owned bodies and boxed futures, so the stream is `Send` for the spawned job.
    let key = key.to_string();
    let requests: Vec<_> = emails
        .iter()
        .map(|email| {
            let body = ai::decision_body_taught(&config.model, folders, email, spam);
            let key = key.clone();
            Box::pin(async move { decide_with_retry(&key, body, deadline).await })
        })
        .collect();
    let mut answers = futures::stream::iter(requests).buffered(ai::DECISION_PARALLEL);
    let mut picks = Vec::with_capacity(emails.len());
    let mut asked = emails.iter();
    while let Some(answer) = answers.next().await {
        // Dropping the stream drops the requests not yet answered.
        let email = asked.next();
        let automated = email.is_some_and(|email| email.automated.is_some());
        let spam = spam.filter(|_| email.is_some_and(|email| email.spam_backed));
        picks.push(ai::parse_decision_taught(
            &answer?, folders, automated, spam,
        ));
    }
    Ok(batch
        .iter()
        .zip(picks)
        .map(|(candidate, (pick, why))| AiSortDecision {
            candidate: candidate.clone(),
            target: pick.map(|pick| pick.target),
            why,
        })
        .collect())
}
