//! Folder and message sync against the server, including companion folders.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use crate::{imap, store};

use super::*;

mod folders;
pub use folders::{mark_starred_copies, push_pending_tags, spawn_tag_push, sync_folders};
mod transfers;
pub use transfers::{
    MovedCopies, Trashed, delete_to_trash, move_on_server, move_within_account,
    move_within_account_with, moved_message_ids, sender_delete_uids,
};
pub(crate) use transfers::{delete_to_trash_alone, move_on_server_alone};
mod destination;
#[cfg(test)]
pub(super) use destination::moved_copy_uids;
pub(crate) use destination::{CopiesAsk, refresh_moved_copies_shared};
pub use destination::{refresh_moved_copies, refresh_moved_copies_with};
mod snapshot;
pub use snapshot::SyncMessagesResult;
pub(super) use snapshot::{FolderSyncState, sync_state_isolating_unparseable};
use snapshot::{cached_in_window, read_flag_state};
mod recovery;
pub use recovery::fetch_recent_resilient;
pub(super) use recovery::{fetch_headers_isolating_unparseable, recover_recent_batch};
use recovery::{fetch_headers_isolating_until, read_isolating};
mod messages;
#[cfg(test)]
use messages::sync_messages_with_policy;
pub use messages::{
    MessageSyncGuard, WrittenFolderRefresh, refresh_written_folder, sync_background_messages,
    sync_messages,
};
mod sorting;
pub(super) use sorting::sort_carried_mail;
mod policy;
pub use policy::{CompanionSync, background_sync_timeout, sync_companion_folders};
use policy::{RULE_ACTIONS_BUDGET, fetch_uncached_marked};
#[cfg(test)]
pub(super) use policy::{companion_folders, parse_background_sync_timeout};
#[cfg(test)]
mod guard_tests;
