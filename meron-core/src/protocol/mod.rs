//! Defines the request and event entry points shared by Meron's hosts.
//! The dispatcher decodes command parameters and selects the responsible feature.
//! Replies and startup events are serialized for the host transport.

pub(crate) use crate::imap::{self, Creds, MessageHeader};
pub(crate) use crate::rss;
pub(crate) use crate::secrets::Secrets;
pub(crate) use crate::smtp::{self, AttachmentInput};
pub(crate) use crate::store::{self, AccountMeta};
pub(crate) use crate::thread_list;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
pub(crate) use std::path::{Component, Path};
pub(crate) use std::sync::Mutex;
pub(crate) use std::time::{SystemTime, UNIX_EPOCH};

mod accounts;
mod backup;
mod calendar;
mod feeds;
mod helpers;
mod mail;
mod media;
mod oauth;
mod prefs;
mod tasks;

pub(crate) use accounts::*;
pub(crate) use backup::*;
pub(crate) use calendar::*;
pub(crate) use feeds::*;
pub(crate) use helpers::*;
pub use mail::messages_still_unread;
pub(crate) use mail::*;
pub(crate) use media::*;
pub(crate) use oauth::*;
pub(crate) use prefs::*;
pub(crate) use tasks::*;

#[cfg(test)]
mod tests;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Version of the Meron core request/response/event protocol.
///
/// Bump this on any breaking change to request, response, or event shapes. The
/// desktop Go bridge checks it on the `ready` event, and mobile hosts should
/// assert it during core startup before issuing commands.
pub const PROTOCOL_VERSION: u32 = 1;
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

pub fn ready_event() -> Value {
    json!({ "version": VERSION, "protocol": PROTOCOL_VERSION })
}

pub fn ping_response() -> Value {
    json!({ "pong": true, "version": VERSION, "protocol": PROTOCOL_VERSION })
}

pub fn dispatch_protocol_request(request: &Request) -> Result<Value, String> {
    match request.method.as_str() {
        "ping" => Ok(ping_response()),
        "account.list" => Ok(json!({ "accounts": [] })),
        "account.autodiscover" => autodiscover_mobile_account(&request.params),
        "oauth.authorizeUrl" => mobile_oauth_authorize_url(&request.params),
        "account.addOAuth" => Ok(json!({ "account": Value::Null })),
        "account.exchangeOAuthCode" => Ok(json!({ "account": Value::Null })),
        "account.updateOAuthToken" => Ok(json!({ "ok": true, "account": Value::Null })),
        "account.addRss" | "account.addRSS" => Ok(json!({ "account": Value::Null })),
        "account.remove" => Ok(json!({ "ok": true })),
        "account.setName" => Ok(json!({ "ok": true })),
        "account.setSenderName" => Ok(json!({ "ok": true })),
        "account.setAvatar" => Ok(json!({ "ok": true })),
        "account.writeAvatarFile" => Ok(json!({ "url": "" })),
        "account.setChatWallpaper" => Ok(json!({ "ok": true })),
        "account.writeChatWallpaperFile" => Ok(json!({ "url": "" })),
        "account.setImages" => Ok(json!({ "ok": true })),
        "account.setConversationHtml" => Ok(json!({ "ok": true })),
        "account.setUnified" => Ok(json!({ "ok": true })),
        "account.setMuted" => Ok(json!({ "ok": true })),
        "account.setPaused" => Ok(json!({ "ok": true })),
        "account.setSaveSentCopy" => Ok(json!({ "ok": true })),
        "account.setRSSSyncInterval" => Ok(json!({ "ok": true })),
        "account.setAliases" => Ok(json!({ "ok": true })),
        "account.setProxy" => Ok(json!({ "ok": true })),
        "app.prefsGet" => Ok(json!({ "prefs": {} })),
        "app.prefsSet" => Ok(json!({ "ok": true })),
        "app.proxyGet" => Ok(json!({ "proxy": Value::Null })),
        "app.proxySet" => Ok(json!({ "ok": true })),
        "account.reorder" => Ok(json!({ "ok": true })),
        "contacts.suggest" | "mail.suggestContacts" => Ok(json!({ "contacts": [] })),
        "contacts.threads" | "mail.contactThreads" => {
            Ok(json!({ "threads": [], "next_cursor": null }))
        }
        "feed.add" | "rss.addFeed" => Ok(json!({ "ok": true })),
        "feed.remove" | "rss.removeFeed" => Ok(json!({ "ok": true })),
        "feed.move" | "rss.moveFeed" => Ok(json!({ "ok": true, "moved": 0 })),
        "backup.export" => Ok(json!({ "backup": "" })),
        "backup.import" => {
            Ok(json!({ "accounts": 0, "skipped": 0, "feeds": 0, "settings": 0, "secrets": 0 }))
        }
        "rss.exportOpml" => Ok(json!({ "opml": "" })),
        "rss.importOpml" => Ok(json!({ "imported": 0 })),
        "mail.folderList" => Ok(json!({ "folders": [] })),
        "mail.folderCreate" => Ok(json!({ "folders": [] })),
        "mail.folderDelete" => Ok(json!({ "ok": true, "deleted": 0, "folders": [] })),
        "mail.folderRename" => Ok(json!({ "ok": true, "renamed": [], "folders": [] })),
        "mail.folderSetNotify" => Ok(json!({ "folders": [] })),
        "mail.threadList" => Ok(json!({ "threads": [] })),
        "mail.threadRead" => Ok(json!({ "messages": [] })),
        "mail.attachmentRead" | "mail.readAttachment" => Ok(json!({ "data": "" })),
        "storage.usage" => Ok(json!({ "cacheBytes": 0, "dbBytes": 0 })),
        "storage.clearCache" => Ok(json!({ "cacheBytes": 0, "dbBytes": 0 })),
        "mail.starredItems" => Ok(json!({ "items": [] })),
        "mail.allocateIdentity" => Ok(json!({ "message_id": "" })),
        "mail.send" => Ok(json!({ "ok": true })),
        "invite.respond" => Ok(json!({ "ok": true })),
        "mail.saveDraft" => Ok(json!({ "ok": true })),
        "mail.discardDraft" => Ok(json!({ "ok": true, "deleted": 0, "permanent": true })),
        "mail.archive" => Ok(json!({ "ok": true, "moved": 0 })),
        "mail.delete" => Ok(json!({ "ok": true, "deleted": 0 })),
        "mail.deleteMany" => Ok(json!({ "ok": true, "results": [] })),
        "mail.move" => Ok(json!({ "ok": true, "moved": 0 })),
        "mail.copy" => Ok(json!({ "ok": true, "copied": 0 })),
        "mail.markRead" => Ok(json!({ "ok": true })),
        "mail.markAllRead" => Ok(json!({ "ok": true })),
        "mail.emptyFolder" => Ok(json!({ "ok": true, "deleted": 0, "expunged": 0 })),
        "mail.markStarred" => Ok(json!({ "ok": true })),
        "mail.setTags" => Ok(json!({ "ok": true })),
        "mail.tagList" => Ok(json!({ "tags": [] })),
        "mail.replaceTag" => Ok(json!({ "ok": true, "rules": [], "queries": [] })),
        "mail.rules" | "mail.setRules" => Ok(json!({ "rules": [] })),
        "mail.ruleMatches" | "mail.runRule" => Ok(json!({ "count": 0 })),
        "mail.ruleStatus" => Ok(json!({ "log": [], "running": {} })),
        "mail.stopRule" => Ok(json!({ "ok": true })),
        method if crate::ai_sort::mobile_command(method).is_some() => Ok(
            crate::ai_sort::mobile_stub(crate::ai_sort::mobile_command(method).unwrap_or_default()),
        ),
        method if crate::meron_folders::mobile_command(method).is_some() => {
            Ok(crate::meron_folders::mobile_stub(
                crate::meron_folders::mobile_command(method).unwrap_or_default(),
            ))
        }
        "rss.thread" => Ok(json!({ "messages": [] })),
        "rss.markRead" => Ok(json!({ "ok": true })),
        "rss.markStarred" => Ok(json!({ "ok": true })),
        method => match stub_mobile_tasks(method).or_else(|| stub_mobile_calendar(method)) {
            Some(value) => Ok(value),
            None => Err(format!("unknown method: {method}")),
        },
    }
}

pub fn dispatch_mobile_protocol_request(
    command_request: &Request,
    data_directory: &str,
) -> Result<Value, String> {
    match command_request.method.as_str() {
        "reminders.list" | "reminders.claim" | "reminders.skipPast" | "reminders.acknowledge" => {
            with_mobile_db(data_directory, |conn| {
                crate::reminders::dispatch(&conn, &command_request.method, &command_request.params)
                    .map_err(|error| format!("{error:#}"))
            })
        }
        "search.parse" | "search.build" => {
            crate::search_query::handle_request(&command_request.method, &command_request.params)
                .ok_or_else(|| format!("bad {} request", command_request.method))
        }
        "avatar.resolve" => serde_json::to_value(crate::avatar::resolve(&command_request.params))
            .map_err(|error| error.to_string()),
        "changelog.fetch" => {
            let variant = crate::changelog::Variant::parse(
                command_request
                    .params
                    .get("variant")
                    .and_then(Value::as_str)
                    .unwrap_or("android"),
            );
            crate::changelog::fetch(variant).map_err(|error| format!("{error:#}"))
        }
        "account.list" => list_mobile_accounts(data_directory),
        "account.addPassword" => {
            add_mobile_password_account(data_directory, &command_request.params)
        }
        "account.probeCert" => probe_mobile_cert(&command_request.params),
        "account.setCertPin" => {
            set_mobile_account_cert_pin(data_directory, &command_request.params)
        }
        "account.autodiscover" => autodiscover_mobile_account(&command_request.params),
        "oauth.authorizeUrl" => mobile_oauth_authorize_url(&command_request.params),
        "account.addOAuth" => add_mobile_oauth_account(data_directory, &command_request.params),
        "account.exchangeOAuthCode" => {
            exchange_mobile_oauth_code(data_directory, &command_request.params)
        }
        "account.updateOAuthToken" => {
            update_mobile_oauth_token(data_directory, &command_request.params)
        }
        "account.addRss" | "account.addRSS" => {
            add_mobile_rss_account(data_directory, &command_request.params)
        }
        "account.remove" => remove_mobile_account(data_directory, &command_request.params),
        "account.setName" => set_mobile_account_name(data_directory, &command_request.params),
        "account.setSenderName" => {
            set_mobile_account_sender_name(data_directory, &command_request.params)
        }
        "account.setAvatar" => set_mobile_account_avatar(data_directory, &command_request.params),
        "account.writeAvatarFile" => {
            write_mobile_account_media_file(data_directory, &command_request.params, "avatars")
        }
        "account.setChatWallpaper" => {
            set_mobile_account_chat_wallpaper(data_directory, &command_request.params)
        }
        "account.writeChatWallpaperFile" => {
            write_mobile_account_media_file(data_directory, &command_request.params, "wallpapers")
        }
        "account.setImages" => set_mobile_account_images(data_directory, &command_request.params),
        "account.setSignature" => {
            set_mobile_account_signature(data_directory, &command_request.params)
        }
        "account.setConversationHtml" => set_mobile_account_bool_pref(
            data_directory,
            &command_request.params,
            "conversation_html",
        ),
        "account.setUnified" => set_mobile_account_bool_pref(
            data_directory,
            &command_request.params,
            "included_in_unified",
        ),
        "account.setMuted" => {
            set_mobile_account_bool_pref(data_directory, &command_request.params, "muted")
        }
        "account.setPaused" => {
            set_mobile_account_bool_pref(data_directory, &command_request.params, "paused")
        }
        "account.setSaveSentCopy" => {
            set_mobile_account_save_sent_copy(data_directory, &command_request.params)
        }
        "account.setRSSSyncInterval" => {
            set_mobile_account_rss_sync_interval(data_directory, &command_request.params)
        }
        "account.setAliases" => set_mobile_account_aliases(data_directory, &command_request.params),
        "account.setProxy" => set_mobile_account_proxy(data_directory, &command_request.params),
        "app.prefsGet" => get_mobile_prefs(data_directory, &command_request.params),
        "app.prefsSet" => set_mobile_pref(data_directory, &command_request.params),
        "app.proxyGet" => get_mobile_proxy(data_directory),
        "app.proxySet" => set_mobile_proxy(data_directory, &command_request.params),
        "account.reorder" => reorder_mobile_accounts(data_directory, &command_request.params),
        "contacts.suggest" | "mail.suggestContacts" => {
            suggest_mobile_contacts(data_directory, &command_request.params)
        }
        "contacts.threads" | "mail.contactThreads" => {
            list_mobile_contact_threads(data_directory, &command_request.params)
        }
        "feed.add" | "rss.addFeed" => add_mobile_rss_feed(data_directory, &command_request.params),
        "feed.remove" | "rss.removeFeed" => {
            remove_mobile_rss_feed(data_directory, &command_request.params)
        }
        "feed.move" | "rss.moveFeed" => {
            move_mobile_rss_feed(data_directory, &command_request.params)
        }
        "backup.export" => export_mobile_backup(data_directory, &command_request.params),
        "backup.import" => import_mobile_backup(data_directory, &command_request.params),
        "rss.exportOpml" => export_mobile_opml(data_directory, &command_request.params),
        "rss.importOpml" => import_mobile_opml(data_directory, &command_request.params),
        "mail.folderList" => list_mobile_folders(data_directory, &command_request.params),
        "mail.folderCreate" => create_mobile_folder(data_directory, &command_request.params),
        "mail.folderDelete" => delete_mobile_folder(data_directory, &command_request.params),
        "mail.folderRename" => rename_mobile_folder(data_directory, &command_request.params),
        "mail.folderSetNotify" => set_mobile_folder_notify(data_directory, &command_request.params),
        "mail.threadList" => list_mobile_threads(data_directory, &command_request.params),
        "mail.threadRead" => read_mobile_thread(data_directory, &command_request.params),
        "mail.attachmentRead" | "mail.readAttachment" => {
            read_mobile_attachment_file(data_directory, &command_request.params)
        }
        "storage.usage" => mobile_storage_usage(data_directory),
        "storage.clearCache" => clear_mobile_storage_cache(data_directory),
        "mail.starredItems" => list_mobile_starred_items(data_directory, &command_request.params),
        "mail.stillUnread" => mobile_messages_still_unread(data_directory, &command_request.params),
        "mail.allocateIdentity" => {
            let account_id = command_request
                .params
                .get("account_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let draft = command_request
                .params
                .get("draft")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Ok(json!({ "message_id": crate::mail_model::allocate_message_id(account_id, draft) }))
        }
        "mail.sync" | "messages.sync" => sync_mobile_mail(data_directory, &command_request.params),
        "mail.send" => send_mobile_message(data_directory, &command_request.params),
        "invite.respond" => respond_mobile_invite(data_directory, &command_request.params),
        "mail.saveDraft" => save_mobile_draft(data_directory, &command_request.params),
        "mail.discardDraft" => discard_mobile_draft(data_directory, &command_request.params),
        "mail.archive" => archive_mobile_thread(data_directory, &command_request.params),
        "mail.delete" => delete_mobile_thread(data_directory, &command_request.params),
        "mail.deleteMany" => delete_mobile_threads(data_directory, &command_request.params),
        "mail.move" => move_mobile_thread(data_directory, &command_request.params),
        "mail.copy" => copy_mobile_thread(data_directory, &command_request.params),
        "mail.markRead" => mark_mobile_thread_read(data_directory, &command_request.params),
        "mail.markAllRead" => mark_mobile_folder_all_read(data_directory, &command_request.params),
        "mail.emptyFolder" => empty_mobile_folder(data_directory, &command_request.params),
        "mail.markStarred" => mark_mobile_thread_starred(data_directory, &command_request.params),
        "mail.setTags" => set_mobile_thread_tags(data_directory, &command_request.params),
        "mail.tagList" => list_mobile_tags(data_directory),
        "mail.replaceTag" => replace_mobile_tag(data_directory, &command_request.params),
        "mail.rules" => mobile_rules(data_directory),
        "mail.setRules" => set_mobile_rules(data_directory, &command_request.params),
        "mail.ruleMatches" => mobile_rule_matches(data_directory, &command_request.params),
        "mail.runRule" => run_mobile_rule(data_directory, &command_request.params),
        "mail.ruleStatus" => mobile_rule_status(data_directory),
        "mail.stopRule" => stop_mobile_rule(data_directory, &command_request.params),
        method if crate::ai_sort::mobile_command(method).is_some() => mobile_ai_sort(
            data_directory,
            crate::ai_sort::mobile_command(method).unwrap_or_default(),
            &command_request.params,
        ),
        method if method.starts_with("setupCopy.") => mobile_setup_copy(
            data_directory,
            &method["setupCopy.".len()..],
            &command_request.params,
        ),
        method if crate::meron_folders::mobile_command(method).is_some() => mobile_meron_folders(
            data_directory,
            crate::meron_folders::mobile_command(method).unwrap_or_default(),
            &command_request.params,
        ),
        "rss.sync" => sync_mobile_rss(data_directory, &command_request.params),
        "rss.thread" => read_mobile_rss_thread(data_directory, &command_request.params),
        "rss.markRead" => mark_mobile_rss_thread_read(data_directory, &command_request.params),
        "rss.markStarred" => {
            mark_mobile_rss_thread_starred(data_directory, &command_request.params)
        }
        method => match dispatch_mobile_tasks(data_directory, method, &command_request.params)
            .or_else(|| dispatch_mobile_calendar(data_directory, method, &command_request.params))
        {
            Some(result) => result,
            None => dispatch_protocol_request(command_request),
        },
    }
}

pub fn invoke_protocol_json(request_json: &str) -> Value {
    match serde_json::from_str::<Request>(request_json) {
        Ok(command_request) => match dispatch_protocol_request(&command_request) {
            Ok(result) => json!({ "id": command_request.id, "result": result }),
            Err(message) => json!({ "id": command_request.id, "error": { "message": message } }),
        },
        Err(err) => json!({ "error": { "message": format!("bad request: {err}") } }),
    }
}

pub fn invoke_mobile_protocol_json(request_json: &str, data_directory: Option<&str>) -> Value {
    match serde_json::from_str::<Request>(request_json) {
        Ok(command_request) => {
            let result = match data_directory {
                Some(data_directory) => {
                    dispatch_mobile_protocol_request(&command_request, data_directory)
                }
                None => dispatch_protocol_request(&command_request),
            };
            match result {
                Ok(result) => json!({ "id": command_request.id, "result": result }),
                Err(message) => {
                    json!({ "id": command_request.id, "error": { "message": message } })
                }
            }
        }
        Err(err) => json!({ "error": { "message": format!("bad request: {err}") } }),
    }
}
