//! Checks how the engine selects companion, archive and draft folders.
//! Fixtures cover localized server attributes, naming fallbacks and duplicate roles.
//! The timeout preference shares this suite with the background synchronization policy.

use crate::engine::{
    cached_archive_folder_from_folders, companion_folders, find_role_folder,
    parse_background_sync_timeout, thread_gap_search_folders,
};
use std::time::Duration;

fn role_folder(name: &str, special_use: Option<&str>) -> crate::imap::Folder {
    crate::imap::Folder {
        name: name.to_string(),
        special_use: special_use.map(str::to_string),
        ..Default::default()
    }
}

#[test]
fn find_role_folder_prefers_server_attribute_over_name() {
    // A localized drafts folder carries the \Drafts attribute; a folder
    // that merely *looks* like drafts by name must lose to it.
    let folders = vec![
        role_folder("Drafts", None),
        role_folder("Mail/Entwürfe", Some("drafts")),
    ];
    assert_eq!(
        find_role_folder(folders, "drafts", crate::imap::looks_like_drafts, "INBOX"),
        Some("Mail/Entwürfe".to_string())
    );
}

#[test]
fn find_role_folder_falls_back_to_name_heuristic() {
    // No special-use attributes recorded (server without RFC 6154, or rows
    // synced before the column existed): the name heuristic still works.
    let folders = vec![role_folder("INBOX", None), role_folder("Drafts", None)];
    assert_eq!(
        find_role_folder(folders, "drafts", crate::imap::looks_like_drafts, "INBOX"),
        Some("Drafts".to_string())
    );
}

#[test]
fn find_role_folder_never_returns_the_excluded_folder() {
    let folders = vec![role_folder("Drafts", Some("drafts"))];
    assert_eq!(
        find_role_folder(folders, "drafts", crate::imap::looks_like_drafts, "drafts"),
        None
    );
}

#[test]
fn cached_archive_folder_prefers_server_attribute_over_name() {
    let folders = vec![
        role_folder("Archive", None),
        role_folder("Mail/Archiv", Some("archive")),
    ];
    assert_eq!(
        cached_archive_folder_from_folders(folders, "INBOX"),
        Some("Mail/Archiv".to_string())
    );
}

#[test]
fn cached_archive_folder_resolves_localized_attribute_only_name() {
    let folders = vec![
        role_folder("INBOX", None),
        role_folder("Mail/Archiv", Some("archive")),
    ];
    assert_eq!(
        cached_archive_folder_from_folders(folders, "INBOX"),
        Some("Mail/Archiv".to_string())
    );
}

#[test]
fn cached_archive_folder_accepts_gmail_all_attribute() {
    // Gmail advertises All Mail as \All (recorded as "all"), and localized
    // accounts defeat the name heuristic — the attribute alone must win.
    let folders = vec![
        role_folder("INBOX", None),
        role_folder("[Gmail]/Alle Nachrichten", Some("all")),
    ];
    assert_eq!(
        cached_archive_folder_from_folders(folders, "INBOX"),
        Some("[Gmail]/Alle Nachrichten".to_string())
    );
}

#[test]
fn cached_archive_folder_falls_back_to_name_heuristic() {
    let folders = vec![role_folder("INBOX", None), role_folder("All Mail", None)];
    assert_eq!(
        cached_archive_folder_from_folders(folders, "INBOX"),
        Some("All Mail".to_string())
    );
}

#[test]
fn companion_folders_lists_sent_before_drafts() {
    assert_eq!(
        companion_folders(
            Some("[Gmail]/Sent Mail".to_string()),
            Some("[Gmail]/Drafts".to_string())
        ),
        vec![
            ("Sent", "[Gmail]/Sent Mail".to_string()),
            ("Drafts", "[Gmail]/Drafts".to_string()),
        ]
    );
}

#[test]
fn companion_folders_skips_drafts_matching_sent_case_insensitively() {
    // A misconfigured server can advertise both roles on one mailbox;
    // syncing it twice in the same tail would be wasted I/O.
    assert_eq!(
        companion_folders(Some("Sent".to_string()), Some("sent".to_string())),
        vec![("Sent", "Sent".to_string())]
    );
}

#[test]
fn companion_folders_empty_when_neither_resolves() {
    assert_eq!(companion_folders(None, None), Vec::new());
}

#[test]
fn background_sync_timeout_rejects_unsupported_values() {
    assert_eq!(
        parse_background_sync_timeout(Some("300")),
        Duration::from_secs(300)
    );
    for value in [None, Some("0"), Some("invalid"), Some("86401")] {
        assert_eq!(
            parse_background_sync_timeout(value),
            Duration::from_secs(30)
        );
    }
    assert_eq!(
        parse_background_sync_timeout(Some("18446744073709551615")),
        Duration::from_secs(30)
    );
}

#[test]
fn thread_gap_search_folders_include_drafts_once_before_archive() {
    let folders = thread_gap_search_folders(
        Some("Sent".to_string()),
        Some("Drafts".to_string()),
        Some("[Gmail]/All Mail".to_string()),
    );
    assert_eq!(folders, vec!["INBOX", "Sent", "Drafts", "[Gmail]/All Mail"]);
}

#[test]
fn thread_gap_search_folders_dedup_case_insensitively() {
    let folders = thread_gap_search_folders(
        Some("inbox".to_string()),
        Some("Drafts".to_string()),
        Some("drafts".to_string()),
    );
    assert_eq!(folders, vec!["INBOX", "Drafts"]);
}
