//! Tinfoil: a client that first verifies the enclave (AMD SEV-SNP report plus
//! the Sigstore record of its code) and then pins TLS to the attested key, so
//! the request is readable only inside the enclave. The chat path only; the
//! app proxy is not supported, so callers refuse Tinfoil while one is set.

use serde_json::Value;

use super::{AiError, ErrorKind, HTTP_TIMEOUT, Model};

/// Verifying fetches several documents; give it more than one request's time.
pub const VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn classify(err: &::tinfoil::Error) -> AiError {
    use ::tinfoil::async_openai::error::OpenAIError;
    if let ::tinfoil::Error::Api(OpenAIError::ApiError(response)) = err {
        let status = response.status_code.as_u16();
        return AiError::from_status(status);
    }
    if err.is_attestation() {
        return AiError::new(
            ErrorKind::Verify,
            "Tinfoil's privacy check failed, so nothing was sent.",
        );
    }
    if err.is_configuration() {
        return AiError::new(ErrorKind::Other, "Tinfoil isn't set up right.");
    }
    if err.is_retryable() {
        return AiError::network("Couldn't reach Tinfoil.");
    }
    AiError::new(ErrorKind::Other, "Tinfoil didn't answer.")
}

/// Whether `err` means the verified client should be checked again.
pub fn needs_reverify(err: &AiError) -> bool {
    err.kind == ErrorKind::Verify
}

/// A client for `key`, after verifying the enclave.
pub async fn connect(key: &str) -> Result<::tinfoil::Client, AiError> {
    match tokio::time::timeout(
        VERIFY_TIMEOUT,
        ::tinfoil::Client::new_default_with_api_key(key.to_string()),
    )
    .await
    {
        Ok(Ok(client)) => Ok(client),
        Ok(Err(err)) => {
            let mut out = classify(&err);
            // Not reaching the attestation documents is not a failed check.
            if err.is_fetch() {
                out = AiError::network("Couldn't reach Tinfoil to check it.");
            }
            Err(out)
        }
        Err(_) => Err(AiError::network("Checking Tinfoil took too long.")),
    }
}

/// Send one chat request through the verified client and answer with the text.
pub async fn chat(client: &::tinfoil::Client, body: Value) -> Result<String, AiError> {
    match tokio::time::timeout(HTTP_TIMEOUT, client.chat_relaxed().create(body)).await {
        Ok(Ok(response)) => {
            if response.raw().get("error").is_some() {
                return Err(AiError::network("Tinfoil sent an error."));
            }
            Ok(response.content().unwrap_or_default().to_string())
        }
        Ok(Err(err)) => Err(classify(&err)),
        Err(_) => Err(AiError::network("Tinfoil didn't answer in time.")),
    }
}

fn model_of(item: &Value) -> Option<Model> {
    let id = item.get("id")?.as_str()?.to_string();
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("chat");
    if kind != "chat" {
        return None;
    }
    let price = |name: &str| {
        item.get("pricing")
            .and_then(|pricing| pricing.get(name))
            .and_then(|value| match value {
                Value::Number(number) => number.as_f64(),
                Value::String(text) => text.trim().parse().ok(),
                _ => None,
            })
    };
    Some(Model {
        name: item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&id)
            .to_string(),
        price_in_per_m: price("inputTokenPricePer1M"),
        price_out_per_m: price("outputTokenPricePer1M"),
        structured: true,
        decision: false,
        reasoning: item
            .get("reasoning")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        id,
    })
}

/// The chat models, cheapest first, read through the verified client.
pub async fn models(client: &::tinfoil::Client) -> Result<Vec<Model>, AiError> {
    let http = client.http_client().map_err(|err| classify(&err))?;
    let url = format!("{}/v1/models", client.secure_client().base_url());
    let response = http
        .get(&url)
        .timeout(HTTP_TIMEOUT)
        .send()
        .await
        .map_err(|_| AiError::network("Couldn't reach Tinfoil."))?;
    let status = response.status().as_u16();
    let text = response
        .text()
        .await
        .map_err(|_| AiError::network("Tinfoil's answer broke off."))?;
    if !(200..300).contains(&status) {
        return Err(AiError::from_status(status));
    }
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| AiError::network("Tinfoil's model list wasn't readable."))?;
    let mut out: Vec<Model> = value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(model_of)
        .collect();
    super::sort_models(&mut out);
    Ok(out)
}
