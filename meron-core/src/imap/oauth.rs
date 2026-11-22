//! Token refresh and rejected grants.
//!
//! Exchanges a refresh token for the credentials needed by a session.
//! Distinguishes a revoked grant from a temporary provider or connection failure.

use super::*;

#[derive(serde::Deserialize)]
pub(super) struct TokenResponse {
    pub(super) access_token: String,
    pub(super) expires_in: Option<i64>,
    pub(super) refresh_token: Option<String>,
}

/// What a successful [`refresh_oauth_token`] hands back.
pub struct RefreshedToken {
    pub access_token: String,
    /// Seconds until `access_token` expires.
    pub expires_in: i64,
    /// A replacement refresh token, when the provider rotates them. Microsoft issues one on every
    /// refresh and each has its own fixed lifetime, so the caller must keep this one: holding on to the
    /// token it sent would see the account's sign-in expire that long after it was first connected,
    /// however often it refreshed. Google usually sends none.
    pub refresh_token: Option<String>,
}

impl From<TokenResponse> for RefreshedToken {
    fn from(response: TokenResponse) -> Self {
        RefreshedToken {
            access_token: response.access_token,
            expires_in: response.expires_in.unwrap_or(3600),
            refresh_token: response
                .refresh_token
                .filter(|token| !token.trim().is_empty()),
        }
    }
}

/// A token endpoint's refusal of the refresh token itself: its `error` field
/// is `invalid_grant`, which RFC 6749 §5.2 reserves for a grant that is
/// expired, revoked or otherwise no longer valid. Retrying cannot succeed.
/// Displays like any other failed refresh.
#[derive(Debug)]
pub struct OAuthGrantRejected {
    status: String,
    body: String,
}

impl std::fmt::Display for OAuthGrantRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "oauth refresh failed ({}): {}", self.status, self.body)
    }
}

impl std::error::Error for OAuthGrantRejected {}

/// Whether a failed [`refresh_oauth_token`] means the grant itself is dead
/// rather than the request having failed in transit or on the server's side.
pub fn oauth_grant_rejected(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| cause.is::<OAuthGrantRejected>())
}

/// The error for a non-2xx token response. Only a body whose own `error` field
/// says `invalid_grant` is a rejection; the text turning up anywhere else (a
/// description, a gateway's error page) is not.
pub(super) fn oauth_refresh_error(status: &str, body: &str) -> anyhow::Error {
    let code = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|json| json["error"].as_str().map(str::to_string));
    if code.as_deref() == Some("invalid_grant") {
        return anyhow::Error::new(OAuthGrantRejected {
            status: status.to_string(),
            body: body.to_string(),
        });
    }
    anyhow!("oauth refresh failed ({status}): {body}")
}

/// Exchange a refresh token for a fresh access token via an OAuth token endpoint.
///
/// Uses `ureq` (a minimal blocking HTTP client over the same rustls stack as IMAP/SMTP) on a
/// blocking task, so we get correct HTTP framing — status codes, Content-Length/chunked, gzip —
/// instead of hand-parsing bytes.
///
/// `token_url` is the provider's token endpoint. `client_secret` may be empty (Microsoft public
/// clients use PKCE with no secret), in which case it's omitted. `scope`, when present, requests an
/// access token for that resource — Microsoft binds tokens to a resource, so the IMAP/SMTP scopes
/// are passed on refresh; Google ignores it.
pub async fn refresh_oauth_token(
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
    scope: Option<&str>,
    proxy: Option<crate::proxy::ProxyConfig>,
) -> Result<RefreshedToken> {
    let token_url = token_url.to_string();
    let client_id = client_id.to_string();
    let client_secret = client_secret.to_string();
    let refresh_token = refresh_token.to_string();
    let scope = scope.map(|s| s.to_string());

    let parsed = tokio::task::spawn_blocking(move || -> Result<TokenResponse> {
        let mut form: Vec<(&str, &str)> = vec![
            ("client_id", client_id.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ];
        if !client_secret.is_empty() {
            form.push(("client_secret", client_secret.as_str()));
        }
        if let Some(scope) = scope.as_deref() {
            form.push(("scope", scope));
        }

        // Keep non-2xx as a response (not a transport error) so we can surface
        // the JSON error body the provider returns for an invalid/revoked token.
        let mut resp = crate::proxy::agent_for(proxy.as_ref())?
            .post(&token_url)
            .config()
            .http_status_as_error(false)
            .build()
            .send_form(form)
            .context("oauth refresh request")?;

        let status = resp.status();
        let body = resp
            .body_mut()
            .read_to_string()
            .context("read oauth response")?;
        if !status.is_success() {
            return Err(oauth_refresh_error(&status.to_string(), &body));
        }
        serde_json::from_str(&body).context("parse oauth response JSON")
    })
    .await
    .context("oauth refresh task")??;

    Ok(parsed.into())
}
