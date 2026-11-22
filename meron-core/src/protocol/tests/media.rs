//! Account media files and attachment cache commands.
//! The fixture writes media, then reads a cached attachment.
//! Storage reporting and clearing exercise the same cache.

use super::*;
use base64::Engine as _;

#[test]
fn mobile_protocol_writes_account_media_files() {
    let data_directory = unique_data_directory("account-media");
    let unique = unique_test_suffix();
    let email = format!("media+{unique}@example.com");
    let directory_text = data_directory.to_str().unwrap();
    let add_request = format!(
        r#"{{"id":85,"method":"account.addPassword","params":{{"email":"{email}","display_name":"Media","imap_host":"imap.example.com","smtp_host":"smtp.example.com","username":"{email}","password":"secret"}}}}"#
    );
    assert!(
        invoke_mobile_protocol_json(&add_request, Some(directory_text))
            .get("error")
            .is_none()
    );

    let avatar_request = format!(
        r#"{{"id":86,"method":"account.writeAvatarFile","params":{{"id":"{email}","filename":"avatar.png","mime":"image/png","data":"aGVsbG8="}}}}"#
    );
    let avatar = invoke_mobile_protocol_json(&avatar_request, Some(directory_text));
    let avatar_url = avatar["result"]["url"].as_str().unwrap();
    assert!(avatar_url.starts_with("/media/avatars/"), "{avatar}");
    let avatar_path = data_directory.join(avatar_url.trim_start_matches('/'));
    assert!(avatar_path.exists(), "{avatar_url}");

    let wallpaper_request = format!(
        r#"{{"id":87,"method":"account.writeChatWallpaperFile","params":{{"id":"{email}","filename":"wallpaper.webp","mime":"image/webp","data":"d2FsbA=="}}}}"#
    );
    let wallpaper = invoke_mobile_protocol_json(&wallpaper_request, Some(directory_text));
    let wallpaper_url = wallpaper["result"]["url"].as_str().unwrap();
    assert!(
        wallpaper_url.starts_with("/media/wallpapers/"),
        "{wallpaper}"
    );
    let wallpaper_path = data_directory.join(wallpaper_url.trim_start_matches('/'));
    assert!(wallpaper_path.exists(), "{wallpaper_url}");

    let attachment_directory = data_directory
        .join("attachments")
        .join(crate::parse::sanitize_segment(&email));
    std::fs::create_dir_all(&attachment_directory).unwrap();
    std::fs::write(attachment_directory.join("note.txt"), b"cached").unwrap();

    let set_wallpaper = format!(
        r#"{{"id":88,"method":"account.setChatWallpaper","params":{{"id":"{email}","wallpaper":{{"kind":"custom","url":"{wallpaper_url}"}}}}}}"#
    );
    assert!(
        invoke_mobile_protocol_json(&set_wallpaper, Some(directory_text))
            .get("error")
            .is_none()
    );

    let listed = invoke_mobile_protocol_json(
        r#"{"id":89,"method":"account.list","params":{}}"#,
        Some(directory_text),
    );
    assert_eq!(
        listed["result"]["accounts"][0]["chat_wallpaper"]["url"],
        wallpaper_url
    );

    let remove = format!(r#"{{"id":90,"method":"account.remove","params":{{"id":"{email}"}}}}"#);
    assert!(
        invoke_mobile_protocol_json(&remove, Some(directory_text))
            .get("error")
            .is_none()
    );
    assert!(!attachment_directory.exists());
    assert!(!avatar_path.parent().unwrap().exists());
    assert!(!wallpaper_path.parent().unwrap().exists());

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_reads_cached_attachment_file() {
    let data_directory = unique_data_directory("attachment-read");
    let directory_text = data_directory.to_str().unwrap();
    let attachment_directory = data_directory
        .join("attachments")
        .join("acc")
        .join("INBOX")
        .join("1");
    std::fs::create_dir_all(&attachment_directory).unwrap();
    std::fs::write(attachment_directory.join("note.txt"), b"Hi").unwrap();

    let read = invoke_mobile_protocol_json(
        r#"{"id":90,"method":"mail.attachmentRead","params":{"key":"acc/INBOX/1/note.txt"}}"#,
        Some(directory_text),
    );
    assert_eq!(read["id"], 90);
    assert_eq!(read["result"]["data"], STANDARD.encode(b"Hi"));

    let desktop_alias_read = invoke_mobile_protocol_json(
        r#"{"id":92,"method":"mail.readAttachment","params":{"key":"acc/INBOX/1/note.txt"}}"#,
        Some(directory_text),
    );
    assert_eq!(desktop_alias_read["id"], 92);
    assert_eq!(desktop_alias_read["result"]["data"], STANDARD.encode(b"Hi"));

    let rejected = invoke_mobile_protocol_json(
        r#"{"id":91,"method":"mail.attachmentRead","params":{"key":"../meron.db"}}"#,
        Some(directory_text),
    );
    assert!(
        rejected["error"]["message"]
            .as_str()
            .unwrap()
            .contains("invalid attachment key")
    );

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_reports_and_clears_storage_cache() {
    let data_directory = unique_data_directory("storage-cache");
    let directory_text = data_directory.to_str().unwrap();
    seed_mobile_account(&data_directory, "me@example.com");
    let attachment_directory = data_directory
        .join("attachments")
        .join("acc")
        .join("INBOX")
        .join("1");
    std::fs::create_dir_all(&attachment_directory).unwrap();
    std::fs::write(attachment_directory.join("one.txt"), b"Hi").unwrap();
    std::fs::write(attachment_directory.join("two.txt"), b"There").unwrap();

    let usage = invoke_mobile_protocol_json(
        r#"{"id":92,"method":"storage.usage"}"#,
        Some(directory_text),
    );
    assert_eq!(usage["id"], 92);
    assert_eq!(usage["result"]["cacheBytes"], 7);
    assert!(usage["result"]["dbBytes"].as_u64().unwrap() > 0, "{usage}");

    let cleared = invoke_mobile_protocol_json(
        r#"{"id":93,"method":"storage.clearCache"}"#,
        Some(directory_text),
    );
    assert_eq!(cleared["id"], 93);
    assert_eq!(cleared["result"]["cacheBytes"], 0);
    assert!(
        cleared["result"]["dbBytes"].as_u64().unwrap() > 0,
        "{cleared}"
    );
    assert!(data_directory.join("attachments").exists());
    assert!(data_directory.join("meron.db").exists());

    let _ = std::fs::remove_dir_all(data_directory);
}
