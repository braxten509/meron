//! IMAP read path: TLS connect + login (async-imap over tokio-rustls), folder listing, recent-message envelopes, and full message reads.
//!
//! `connect` performs a fresh TLS + auth handshake. The request path does not call it directly per
//! operation — `Engine::with_session` (in `main.rs`) pools warm authenticated sessions and reuses
//! them, falling back to `connect` only when no live session is available. IMAP IDLE watchers hold
//! their own dedicated long-lived connections, separate from that pool.

use anyhow::{Context, Result, anyhow};
use futures::StreamExt;
use mailparse::MailHeaderMap;
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use crate::parse;
use crate::search_query::{SearchQuery, close_open_terms};

#[cfg(test)]
mod append_tests;
mod bodies;
mod checked_mutations;
mod connection;
mod fetch;
mod flags;
mod folders;
mod headers;
#[cfg(test)]
mod idle_tests;
mod listing;
mod models;
#[cfg(test)]
mod mutation_tests;
mod oauth;
mod roles;
mod search;
mod session;
#[cfg(test)]
mod tests;
mod transfer;
#[cfg(test)]
use bodies::civil_from_days;
use bodies::imap_quote;
pub use bodies::{
    BODY_FETCH_BATCH, fetch_bodies, fetch_full_message, prefetch_search_criteria,
    search_prefetch_uids,
};
pub use checked_mutations::{expunge_uids_checked, move_to_folder_checked};
use checked_mutations::{store_deleted, uid_expunge};
#[cfg(test)]
use connection::{MIN_PROTOCOL_TIMEOUT, interleave_address_families, protocol_timeout_for};
pub use connection::{XOAuth2Simple, connect, connect_stream, upgrade_to_tls};
pub(crate) use connection::{connect_tcp, open_socket, starttls_socket};
#[cfg(test)]
use fetch::message_id_search_criteria;
use fetch::uid_fetch_headers;
pub use fetch::{
    FetchedMessage, fetch_bulk_signals, fetch_by_message_ids, fetch_dates_by_uid,
    fetch_headers_by_uid, header_from_raw,
};
pub use flags::{
    AI_CLAIM_PREFIX, AI_SORTED_KEYWORD, AiClaimState, AiClaims, FlagSync, ai_claim_states,
    claim_for_ai_sort, fetch_all_tags, mark_ai_sorted, prepare_flag_update, prepare_tag_update,
    store_meron_marks, store_seen, store_starred, store_tag, sync_flags, uids_with_keyword,
    unclaim_for_ai_sort,
};
use flags::{MAX_UID_SET_LEN, fetch_ai_sorted, fetch_meron_mark, fetch_tags, uid_set_chunks};
pub use folders::{
    PartialFolderDelete, create_folder, delete_folders, folder_status, is_server_refusal,
    is_unparseable_response, is_unselectable_mailbox, list_folders, prepare_folder_delete,
    rename_folder,
};
use headers::{
    attachment_files, fetch_items, header_fields, message_date, supports_condstore,
    supports_gmail_ext, thread_key,
};
#[cfg(test)]
use headers::{body_parameter, first_message_id, normalize_message_id};
pub use listing::{
    FolderListing, IncrementalBatch, fetch_flags_by_uid, fetch_recent, fetch_recent_incremental,
    folder_listing, recent_uids,
};
pub use models::{
    Creds, Folder, MessageHeader, OmittedSettings, RecentBatch, Recipient, grant_fingerprint,
};
pub use oauth::{OAuthGrantRejected, RefreshedToken, oauth_grant_rejected, refresh_oauth_token};
#[cfg(test)]
use oauth::{TokenResponse, oauth_refresh_error};
pub use roles::{
    find_archive_folder, find_drafts_folder, find_sent_folder, find_trash_folder,
    looks_like_archive, looks_like_drafts, looks_like_junk, looks_like_sent, looks_like_trash,
};
#[cfg(test)]
use search::{ATTACHMENT_CANDIDATES, gmail_raw, search_criteria};
pub use search::{list_all_uids, list_unseen_uids, search_starred_uids, search_uids, uids_from};
#[cfg(test)]
use session::tcp_looks_open;
pub use session::{
    ConnState, IDLE_SETTLE_CAP, IDLE_SETTLE_QUIET, Session, Stream, session_looks_open, settle_idle,
};
pub(crate) use session::{ReadProgress, capabilities_once};
#[cfg(test)]
use transfer::append_message;
pub use transfer::{
    MoveOutcome, RawMessageCopy, append_copied_message, append_copied_messages, append_to_sent,
    discard_draft, empty_folder, expunge_uids, fetch_raw_messages_for_copy, folder_holds_message,
    folder_uidvalidity, move_to_folder, move_to_folder_once, replace_draft,
};
