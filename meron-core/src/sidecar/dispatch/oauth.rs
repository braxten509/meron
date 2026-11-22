//! Desktop OAuth sign-in. The Go host runs the browser and loopback redirect;
//! these build the authorization URL, exchange the code and fetch the profile
//! photo with the shared `meron_core::oauth` used by mobile.

use base64::Engine as _;
use serde_json::{Value, json};

use meron_core::engine::default_oauth_client;
use meron_core::oauth;
use meron_core::protocol::Request;

pub(crate) async fn dispatch(request: &Request) -> anyhow::Result<Value> {
    let parameters = request.params.clone();
    let method = request.method.clone();
    tokio::task::spawn_blocking(move || match method.as_str() {
        "oauth.authorizeUrl" => authorize_url(&parameters),
        "oauth.exchangeCode" => exchange_code(&parameters),
        "oauth.fetchPhoto" => fetch_photo(&parameters),
        _ => Err(format!("unknown method: {method}")),
    })
    .await?
    .map_err(anyhow::Error::msg)
}

fn string_parameter(parameters: &Value, key: &str) -> String {
    parameters
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn client(parameters: &Value) -> Result<oauth::Client, String> {
    match parameters
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("")
    {
        provider @ ("gmail" | "outlook") => Ok(default_oauth_client(provider)),
        _ => Err("unsupported oauth provider".to_string()),
    }
}

/// `{provider, redirect_uri, state, code_challenge}` → `{url}`.
fn authorize_url(parameters: &Value) -> Result<Value, String> {
    let url = oauth::authorize_url(
        &client(parameters)?,
        &string_parameter(parameters, "redirect_uri"),
        &string_parameter(parameters, "state"),
        &string_parameter(parameters, "code_challenge"),
        "",
    )?;
    Ok(json!({ "url": url }))
}

/// `{provider, code, code_verifier, redirect_uri}` → the tokens and profile.
fn exchange_code(parameters: &Value) -> Result<Value, String> {
    let client = client(parameters)?;
    let tokens = oauth::exchange_code(
        &client,
        &string_parameter(parameters, "code"),
        &string_parameter(parameters, "redirect_uri"),
        &string_parameter(parameters, "code_verifier"),
    )?;
    let refresh_token = tokens.refresh_token.clone().unwrap_or_default();
    if refresh_token.is_empty() {
        return Err("OAuth response did not include a refresh token".to_string());
    }
    let profile = oauth::fetch_profile(&client, &tokens)?;
    Ok(json!({
        "email": profile.email,
        "display_name": profile.name,
        "avatar_url": profile.picture,
        "access_token": tokens.access_token,
        "refresh_token": refresh_token,
        "expires_in": tokens.expires_in.unwrap_or(3600),
    }))
}

/// `{provider, access_token, refresh_token}` → `{data, ext}`, the base64 photo.
fn fetch_photo(parameters: &Value) -> Result<Value, String> {
    let photo = oauth::fetch_photo(
        &client(parameters)?,
        &string_parameter(parameters, "access_token"),
        &string_parameter(parameters, "refresh_token"),
        "",
    )?;
    Ok(json!({
        "data": base64::engine::general_purpose::STANDARD.encode(photo.bytes),
        "ext": photo.ext,
    }))
}
