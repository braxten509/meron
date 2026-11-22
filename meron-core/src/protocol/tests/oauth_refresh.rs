//! OAuth account refresh and reconnect.
//! Reconnect must retain the account's existing avatar.
//! The Outlook refresh uses the account's client identity and IMAP scopes.

use super::*;

#[test]
fn mobile_protocol_oauth_reconnect_keeps_the_stored_avatar() {
    let data_directory = unique_data_directory("oauth-keeps-avatar");
    let directory_text = data_directory.to_str().unwrap();
    let email = format!("keep+{}@gmail.com", unique_test_suffix());
    let add = |avatar: &str| {
        invoke_mobile_protocol_json(
            &format!(
                r#"{{"id":1,"method":"account.addOAuth","params":{{"email":"{email}","provider":"gmail","avatar_url":"{avatar}","access_token":"access","refresh_token":"refresh"}}}}"#
            ),
            Some(directory_text),
        )
    };
    let first = add("https://example.com/provider.png");
    assert_eq!(
        first["result"]["account"]["avatar_url"], "https://example.com/provider.png",
        "{first}"
    );
    let set = invoke_mobile_protocol_json(
        &format!(
            r#"{{"id":2,"method":"account.setAvatar","params":{{"id":"{email}","avatar_url":"/media/avatars/custom.png"}}}}"#
        ),
        Some(directory_text),
    );
    assert!(set.get("error").is_none(), "{set}");

    // A reconnect offers the provider's photo again; the chosen one stays.
    let again = add("https://example.com/provider.png");
    assert_eq!(
        again["result"]["account"]["avatar_url"], "/media/avatars/custom.png",
        "{again}"
    );

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn engine_refreshes_outlook_with_the_accounts_client_and_imap_scopes() {
    use std::io::Write;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let token_url = format!("http://{}/token", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = crate::test_support::read_http_request(&mut stream);
        tx.send(request).unwrap();
        let body = r#"{"access_token":"fresh","refresh_token":"refresh-2","expires_in":3600}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });

    let data_directory = unique_data_directory("engine-outlook-refresh");
    let directory_text = data_directory.to_str().unwrap();
    let email = format!("refresh+{}@outlook.com", unique_test_suffix());
    let added = invoke_mobile_protocol_json(
        &format!(
            r#"{{"id":1,"method":"account.addOAuth","params":{{"email":"{email}","provider":"outlook","client_id":"own-client","token_url":"{token_url}","access_token":"stale","refresh_token":"refresh","token_expires_at":1}}}}"#
        ),
        Some(directory_text),
    );
    assert!(added.get("error").is_none(), "{added}");

    let engine = crate::engine::Engine::new(Box::new(MobileHost {
        data_dir: directory_text.to_string(),
    }))
    .expect("engine builds");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let creds = rt
        .block_on(engine.ensure_valid_creds(&email))
        .expect("refreshes");
    assert_eq!(creds.access_token.as_deref(), Some("fresh"));

    let request = rx.recv().unwrap();
    assert!(request.contains("client_id=own-client"), "{request}");
    assert!(request.contains("IMAP.AccessAsUser.All"), "{request}");
    assert!(!request.contains("client_secret"), "{request}");

    let _ = std::fs::remove_dir_all(data_directory);
}
