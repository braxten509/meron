//! Account credentials, folder metadata and message summaries.
//!
//! Defines account connection settings and the records returned by folder reads.
//! These types cross the boundary between IMAP sessions, the engine and local storage.

#[derive(Clone)]
pub struct Creds {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// Implicit TLS on connect. False = plaintext (no encryption).
    pub tls: bool,
    /// Upgrade a plaintext IMAP connection to TLS via STARTTLS after the
    /// greeting. Takes precedence over `tls`: when true we connect plaintext
    /// and issue STARTTLS rather than wrapping the socket in implicit TLS.
    pub starttls: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: bool,
    /// STARTTLS for the SMTP submission connection (typically port 587).
    pub smtp_starttls: bool,
    pub auth_type: String,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: i64,
    pub oauth_client_id: String,
    pub oauth_client_secret: String,
    pub oauth_token_url: String,
    pub oauth_scope: String,
    /// Whether this account follows the app-wide proxy, forces a direct
    /// connection, or uses its own.
    pub proxy: crate::proxy::ProxyChoice,
    /// Hex SHA-256 of an IMAP server certificate the user inspected and
    /// accepted, for servers whose certificate webpki cannot validate (local
    /// bridges with a self-signed leaf). See [`crate::tls`].
    pub cert_pin: Option<String>,
    /// The same, for the submission server. Separate because the two can be
    /// different daemons with different certificates — a local bridge serves one
    /// certificate on both ports, but nothing guarantees that in general.
    pub smtp_cert_pin: Option<String>,
    /// Fingerprint ([`grant_fingerprint`]) of a refresh token the provider
    /// refused outright (an expired or revoked OAuth grant). Persisted, so the
    /// account reads as needing a reconnect across restarts instead of failing
    /// the same refresh on every sync. It names the token rather than the
    /// account on purpose: once the account holds any other token — however
    /// and whenever a reconnect's writes land — the verdict no longer applies.
    /// See [`Creds::auth_rejected`].
    pub rejected_grant: Option<String>,
}

/// A stable, non-reversible name for a refresh token, to remember which one a
/// provider refused without storing the token outside the secret store.
pub fn grant_fingerprint(refresh_token: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(refresh_token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Which settings a save left out of its parameters, and so must be carried
/// over from the stored account instead of reset.
///
/// Saving an account resends what the setup form holds — servers, ports,
/// credentials. Everything else about a connection lives elsewhere in the UI (a
/// per-account proxy) or is answered by a prompt (an accepted certificate), so a
/// reconnect that only re-enters a password must not clear them.
#[derive(Clone, Copy, Debug)]
pub struct OmittedSettings {
    pub proxy: bool,
    pub cert_pin: bool,
    pub smtp_cert_pin: bool,
    /// Editing an existing account's servers must not require retyping its
    /// password, and the UI never holds one to resend: an absent `password`
    /// means "keep the stored one" rather than "set it to empty".
    pub password: bool,
}

impl OmittedSettings {
    pub fn any(&self) -> bool {
        self.proxy || self.cert_pin || self.smtp_cert_pin || self.password
    }
}

impl Creds {
    /// Whether the provider has refused the refresh token these credentials
    /// hold, so that nothing short of signing in again will work.
    pub fn auth_rejected(&self) -> bool {
        match (
            self.rejected_grant.as_deref(),
            self.refresh_token.as_deref(),
        ) {
            (Some(rejected), Some(token)) if !token.is_empty() => {
                rejected == grant_fingerprint(token)
            }
            _ => false,
        }
    }

    /// Copy the omitted settings across from the account as it is stored today.
    pub fn carry_over(&mut self, stored: &Creds, omitted: OmittedSettings) {
        if omitted.proxy {
            self.proxy = stored.proxy.clone();
        }
        if omitted.cert_pin {
            self.cert_pin = stored.cert_pin.clone();
        }
        if omitted.smtp_cert_pin {
            self.smtp_cert_pin = stored.smtp_cert_pin.clone();
        }
        if omitted.password {
            self.password = stored.password.clone();
        }
    }

    /// Whether this account authenticates via OAuth2 (XOAUTH2) rather than a
    /// password. Covers every provider — `gmail_oauth`, `outlook_oauth`, … — so
    /// new providers don't need to be enumerated at each auth site.
    pub fn is_oauth(&self) -> bool {
        self.auth_type.ends_with("_oauth")
    }
}

#[derive(serde::Serialize, Default)]
pub struct Folder {
    /// Canonical wire name, modified UTF-7 when the server speaks it. Everything
    /// that addresses the mailbox — SELECT, cached message rows, saved Kanban
    /// columns — uses this; only `display_name` is decoded for humans.
    pub name: String,
    /// `name` decoded from modified UTF-7. Equal to `name` for ASCII folders.
    #[serde(default)]
    pub display_name: String,
    pub delimiter: Option<String>,
    /// Count of unseen messages cached for this folder. Populated by
    /// `store::get_folders`; the IMAP LIST path leaves it at 0.
    #[serde(default)]
    pub unread: u32,
    /// RFC 6154 special-use role ("drafts", "sent", "trash", "junk",
    /// "archive", "all") as advertised by LIST, when the server supports the
    /// extension. Cached in the store so role lookups (e.g. which folder holds
    /// drafts) don't depend on name heuristics alone.
    #[serde(default)]
    pub special_use: Option<String>,
    /// UI role computed from `special_use` plus core name heuristics.
    #[serde(default)]
    pub role: String,
    /// The user opted this folder in to live sync and new-mail notifications.
    /// Populated by `store::get_folders`; INBOX is always watched regardless.
    #[serde(default)]
    pub notify: bool,
}

/// One addressee parsed from an envelope `To`/`Cc` list.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
pub struct Recipient {
    pub name: String,
    pub addr: String,
}

#[derive(serde::Serialize, Default, Clone)]
pub struct MessageHeader {
    pub uid: u32,
    /// IMAP folder this UID lives in. Populated only by cross-folder queries
    /// (thread views that merge Inbox + Sent); single-folder paths leave it
    /// empty and the caller supplies the folder out-of-band.
    #[serde(default)]
    pub folder: String,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    /// Send time as Unix epoch seconds (0 when unknown). Formatted for display
    /// in local time by the frontend.
    pub date: i64,
    pub seen: bool,
    pub starred: bool,
    pub thread_key: String,
    /// Tags among the message's keywords, when this header came from a FETCH
    /// that read its FLAGS. `None` says nothing about them.
    #[serde(skip)]
    pub server_tags: Option<Vec<String>>,
    /// The message carries [`AI_SORTED_KEYWORD`]: some device's AI sorting
    /// already decided on it. Only known from a FETCH that read FLAGS.
    #[serde(skip)]
    pub ai_sorted: bool,
    /// The Meron folder id its `$MeronF_<id>` mark names (see
    /// [`crate::meron_marks`]): some device filed its conversation there.
    /// `None`: not known (the FETCH had no FLAGS); `Some(None)`: no mark.
    #[serde(skip)]
    pub meron_mark: Option<Option<String>>,
    /// Listed in the Inbox from its own (other) folder: mail AI sorting left
    /// there and shows in the Inbox. Set only by the mixed Inbox list reads;
    /// such rows group into cards of their own folder, never the Inbox's.
    #[serde(skip)]
    pub ai_shown: bool,
    /// Normalized RFC Message-ID from the envelope, when available.
    #[serde(default)]
    pub message_id: String,
    /// Gmail's stable per-message id (`X-GM-MSGID`), when the server exposes it.
    #[serde(default)]
    pub gmail_msg_id: Option<u64>,
    /// Normalized RFC In-Reply-To from the envelope, when available.
    #[serde(default)]
    pub in_reply_to: String,
    /// Envelope `To`/`Cc` addressees. Populated by the envelope-fetch paths and
    /// persisted for recipient autocomplete; the cached-row SELECT paths that
    /// don't need them leave these empty.
    #[serde(default)]
    pub to: Vec<Recipient>,
    #[serde(default)]
    pub cc: Vec<Recipient>,
    /// For the thread-card projection only: when an outbound message's identity is
    /// rewritten to the recipient (see `store::apply_card_identity`), this holds the
    /// number of *additional* recipients beyond the one shown, for a "+N" hint.
    #[serde(default)]
    pub recipient_overflow: u32,
    /// Attachment names from BODYSTRUCTURE; None means metadata was not fetched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<crate::store::CardFile>>,
}

/// A batch of recent messages plus the folder's UID sync markers, so the caller
/// can detect UIDVALIDITY resets and persist `uid_next`.
#[derive(Clone)]
pub struct RecentBatch {
    pub uidvalidity: u32,
    pub uid_next: u32,
    pub messages: Vec<MessageHeader>,
}
