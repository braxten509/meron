//! Commands used by the mobile mail bridge.
//!
//! Exposes typed operations for the protocol dispatcher and groups handlers by feature.
//! Request and response formats remain owned by the individual handlers.

use super::*;
mod synchronization;
use synchronization::DRAFT_SYNC_LIMIT;
pub(crate) use synchronization::sync_mobile_mail;
mod outgoing;
pub(crate) use outgoing::{
    discard_mobile_draft, respond_mobile_invite, save_mobile_draft, send_mobile_message,
};
mod folders;
pub(crate) use folders::{
    create_mobile_folder, delete_mobile_folder, list_mobile_folders, rename_mobile_folder,
    set_mobile_folder_notify,
};
mod thread_listing;
pub(crate) use thread_listing::{list_mobile_contact_threads, list_mobile_threads};
mod thread_reading;
pub use thread_reading::messages_still_unread;
pub(crate) use thread_reading::{
    list_mobile_starred_items, mobile_messages_still_unread, read_mobile_thread,
};
mod deletion;
use deletion::cache_server_move;
pub(crate) use deletion::{archive_mobile_thread, delete_mobile_thread, delete_mobile_threads};
mod transfers;
pub(crate) use transfers::{copy_mobile_thread, move_mobile_thread};
mod read_flags;
pub(crate) use read_flags::{
    empty_mobile_folder, mark_mobile_folder_all_read, mark_mobile_thread_read,
};
mod tags;
pub(crate) use tags::{
    list_mobile_tags, mark_mobile_thread_starred, replace_mobile_tag, set_mobile_thread_tags,
};
mod rules;
pub(crate) use rules::{
    mobile_ai_sort, mobile_meron_folders, mobile_rule_matches, mobile_rule_status, mobile_rules,
    mobile_setup_copy, run_mobile_rule, set_mobile_rules, stop_mobile_rule,
};
mod message_identity;
use message_identity::requests_empty_message_ids;
pub(crate) use message_identity::{
    ParsedThreadId, cached_thread_uids, canon_folder, find_cached_archive_folder, format_thread_id,
    has_requested_mobile_message_ids, parse_thread_id, parse_thread_id_with_request_folder,
    requested_mobile_uids, update_mobile_read_state,
};
