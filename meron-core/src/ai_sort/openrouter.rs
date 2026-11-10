//! OpenRouter: chat, model list and key check. Blocking (ureq), so callers run
//! these in `spawn_blocking`; through [`crate::proxy::agent`], so the app proxy
//! applies. Each request has its own [`super::HTTP_TIMEOUT`].

use serde_json::{Value, json};

use super::{AiError, ErrorKind, HTTP_TIMEOUT, Model};

const BASE: &str = "https://openrouter.ai/api/v1";

fn agent() -> Result<ureq::Agent, AiError> {
    crate::proxy::agent().map_err(|_| AiError::new(ErrorKind::Other, "The app proxy setting is invalid."))
}

/// Read a response: its status, `Retry-After` and body.
fn read(
    response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<(u16, Option<String>, String), AiError> {
    let mut response = response.map_err(|err| match err {
        ureq::Error::Timeout(_) => AiError::network("The AI service didn't answer in time."),
        _ => AiError::network("Couldn't reach the AI service."),
    })?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|_| AiError::network("The AI service's answer broke off."))?;
    Ok((status, retry_after, body))
}

/// The body is not read: only the status decides, and the text stays local.
fn http_error(status: u16, retry_after: Option<String>, _body: &str) -> AiError {
    let mut err = AiError::from_status(status);
    // 403: moderation or a key that may not do this. Neither gets better by asking again.
    if status == 403 {
        err.retry = false;
    }
    err.retry_after = super::parse_retry_after(retry_after.as_deref());
    err
}

/// A 200 that carries an `error` object instead of an answer.
fn body_error(value: &Value) -> Option<AiError> {
    let error = value.get("error")?;
    let code = error
        .get("code")
        .and_then(Value::as_u64)
        .map(|code| code as u16)
        .unwrap_or(502);
    let mut err = AiError::from_status(code);
    // Mid-answer failures are worth the one retry unless it's the key or credit.
    err.retry = !matches!(err.kind, ErrorKind::Key | ErrorKind::Credit) && code != 403;
    Some(err)
}

/// Send one chat request and answer with the model's text. `structured`: the
/// model is known to take a schema, so require providers that honour it.
pub fn chat(key: &str, mut body: Value, structured: bool) -> Result<String, AiError> {
    body["provider"] = json!({
        "data_collection": "deny",
        "require_parameters": structured,
    });
    let response = agent()?
        .post(&format!("{BASE}/chat/completions"))
        .header("Authorization", &format!("Bearer {key}"))
        .header("X-Title", "Meron")
        .header("Content-Type", "application/json")
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(HTTP_TIMEOUT))
        .build()
        .send(body.to_string());
    let (status, retry_after, text) = read(response)?;
    if !(200..300).contains(&status) {
        return Err(http_error(status, retry_after, &text));
    }
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| AiError::network("The AI service's answer wasn't readable."))?;
    if let Some(err) = body_error(&value) {
        return Err(err);
    }
    if let Some(err) = value.pointer("/choices/0/error").and_then(|error| body_error(&json!({ "error": error }))) {
        return Err(err);
    }
    Ok(value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

/// Ask a decision model about one email (`/systemone`) and answer with the
/// whole reply. 401 and 403 are the key, 402 the credit.
pub fn decide(key: &str, body: Value) -> Result<Value, AiError> {
    let response = agent()?
        .post(&format!("{BASE}/systemone"))
        .header("Authorization", &format!("Bearer {key}"))
        .header("X-Title", "Meron")
        .header("Content-Type", "application/json")
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(HTTP_TIMEOUT))
        .build()
        .send(body.to_string());
    let (status, retry_after, text) = read(response)?;
    if !(200..300).contains(&status) {
        let mut err = http_error(status, retry_after, &text);
        if status == 403 {
            err.kind = ErrorKind::Key;
            err.message = super::kind_message(ErrorKind::Key).to_string();
        }
        return Err(err);
    }
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| AiError::network("The AI service's answer wasn't readable."))?;
    if let Some(err) = body_error(&value) {
        return Err(err);
    }
    Ok(value)
}

/// Whether the key works, free of charge.
pub fn check_key(key: &str) -> Result<(), AiError> {
    let response = agent()?
        .get(&format!("{BASE}/key"))
        .header("Authorization", &format!("Bearer {key}"))
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(HTTP_TIMEOUT))
        .build()
        .call();
    let (status, retry_after, text) = read(response)?;
    if !(200..300).contains(&status) {
        return Err(http_error(status, retry_after, &text));
    }
    let value: Value = serde_json::from_str(&text).unwrap_or_default();
    // A key that ran out: `limit_remaining` at or below zero.
    if value
        .pointer("/data/limit_remaining")
        .and_then(Value::as_f64)
        .is_some_and(|left| left <= 0.0)
    {
        return Err(AiError::new(ErrorKind::Credit, "This key has no credit left."));
    }
    Ok(())
}

fn per_million(value: Option<&Value>) -> Option<f64> {
    let per_token = match value? {
        Value::String(text) => text.trim().parse::<f64>().ok()?,
        Value::Number(number) => number.as_f64()?,
        _ => return None,
    };
    (per_token >= 0.0).then_some(per_token * 1_000_000.0)
}

fn model_of(item: &Value) -> Option<Model> {
    let id = item.get("id")?.as_str()?.to_string();
    let params: Vec<&str> = item
        .get("supported_parameters")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    Some(Model {
        name: item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&id)
            .to_string(),
        price_in_per_m: per_million(item.pointer("/pricing/prompt")),
        price_out_per_m: per_million(item.pointer("/pricing/completion")),
        structured: params.contains(&"structured_outputs") || params.contains(&"response_format"),
        reasoning: params.contains(&"reasoning") || params.contains(&"include_reasoning"),
        decision: false,
        id,
    })
}

/// The models that take a JSON schema and the decision models, cheapest first,
/// and whether the decision models are in it: a decision list that fails
/// leaves the chat models listed rather than none.
pub fn models(key: Option<&str>) -> Result<(Vec<Model>, bool), AiError> {
    let mut out = list(&format!("{BASE}/models?supported_parameters=structured_outputs"), key)?;
    let decisions = list(&format!("{BASE}/models?output_modalities=decisions"), key);
    let complete = decisions.is_ok();
    for mut model in decisions.unwrap_or_default() {
        // Asked through `/systemone`, which needs no schema; no output billed.
        model.decision = true;
        model.structured = true;
        model.reasoning = false;
        model.price_out_per_m = Some(0.0);
        out.retain(|known| known.id != model.id);
        out.push(model);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    super::sort_models(&mut out);
    Ok((out, complete))
}

/// One model list. It is paginated: `links.next` is followed.
fn list(first: &str, key: Option<&str>) -> Result<Vec<Model>, AiError> {
    let mut url = first.to_string();
    let mut out = Vec::new();
    for _ in 0..20 {
        let mut request = agent()?.get(&url);
        if let Some(key) = key.filter(|key| !key.is_empty()) {
            request = request.header("Authorization", &format!("Bearer {key}"));
        }
        let response = request
            .config()
            .http_status_as_error(false)
            .timeout_global(Some(HTTP_TIMEOUT))
            .build()
            .call();
        let (status, retry_after, text) = read(response)?;
        if !(200..300).contains(&status) {
            return Err(http_error(status, retry_after, &text));
        }
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| AiError::network("The model list wasn't readable."))?;
        out.extend(
            value
                .get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(model_of),
        );
        match value.pointer("/links/next").and_then(Value::as_str) {
            Some(next) if !next.is_empty() => {
                url = if next.starts_with("http") {
                    next.to_string()
                } else {
                    format!("https://openrouter.ai{}", if next.starts_with('/') { next.to_string() } else { format!("/{next}") })
                };
            }
            _ => break,
        }
    }
    Ok(out)
}
