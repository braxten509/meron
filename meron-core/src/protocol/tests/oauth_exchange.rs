//! Exercises protocol behavior for oauth exchange.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn mobile_protocol_exchanges_oauth_code_and_persists_account() {
    let data_dir = unique_data_dir("oauth-code");
    let token_url = one_shot_oauth_token_server();
    let picture_url = one_shot_server("/avatar", "image/png", "png");
    let userinfo_url = one_shot_json_server(
        "/userinfo",
        &format!(
            r#"{{"email":"google.user@gmail.com","name":"Google User","picture":"{picture_url}"}}"#
        ),
    );
    let unique = unique_test_suffix();
    let email = format!("me+{unique}@gmail.com");
    let request = format!(
        r#"{{"id":66,"method":"account.exchangeOAuthCode","params":{{"email":"{email}","provider":"gmail","display_name":"","sender_name":"","code":"auth-code","client_id":"client","client_secret":"secret","redirect_uri":"jp.nonbili.meron.oauth://oauth","code_verifier":"verifier","token_url":"{token_url}","userinfo_url":"{userinfo_url}"}}}}"#
    );

    let exchanged = invoke_mobile_protocol_json(&request, Some(data_dir.to_str().unwrap()));
    assert_eq!(exchanged["id"], 66, "{exchanged}");
    assert_eq!(
        exchanged["result"]["account"]["id"],
        email.as_str(),
        "{exchanged}"
    );
    assert_eq!(
        exchanged["result"]["account"]["provider"], "gmail",
        "{exchanged}"
    );
    assert_eq!(
        exchanged["result"]["account"]["auth_type"], "gmail_oauth",
        "{exchanged}"
    );
    assert_eq!(
        exchanged["result"]["account"]["display_name"], "Google User",
        "{exchanged}"
    );
    assert_eq!(
        exchanged["result"]["account"]["sender_name"], "Google User",
        "{exchanged}"
    );
    // The picture is stored locally, like a chosen avatar.
    let avatar_url = exchanged["result"]["account"]["avatar_url"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        avatar_url.starts_with("/media/avatars/") && avatar_url.ends_with(".png"),
        "{exchanged}"
    );
    assert_eq!(
        std::fs::read(data_dir.join(avatar_url.trim_start_matches('/'))).unwrap(),
        b"png"
    );
    assert_eq!(
        exchanged["result"]["account"]["needs_reconnect"], false,
        "{exchanged}"
    );

    let listed = invoke_mobile_protocol_json(
        r#"{"id":67,"method":"account.list","params":{}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(
        listed["result"]["accounts"][0]["id"],
        email.as_str(),
        "{listed}"
    );
    assert_eq!(
        listed["result"]["accounts"][0]["needs_reconnect"], false,
        "{listed}"
    );
    let conn = store::open_at(data_dir.join("meron.db")).unwrap();
    let config: String = conn
        .query_row(
            "SELECT config FROM accounts WHERE id = ?1",
            [&email],
            |row| row.get(0),
        )
        .unwrap();
    let config: serde_json::Value = serde_json::from_str(&config).unwrap();
    assert_eq!(config["oauth_client_id"], "client", "{config}");
    assert_eq!(config["oauth_client_secret"], "secret", "{config}");
    assert_eq!(config["oauth_token_url"], token_url.as_str(), "{config}");

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_exchanges_oauth_code_uses_id_token_claims() {
    let data_dir = unique_data_dir("oauth-code-claims");
    let token_url =
        one_shot_oauth_token_server_with_id_token("outlook.user@example.com", "Outlook User");
    let request = format!(
        r#"{{"id":68,"method":"account.exchangeOAuthCode","params":{{"email":"","provider":"outlook","display_name":"","sender_name":"","code":"auth-code","client_id":"client","redirect_uri":"msauth://jp.nonbili.meron/example","code_verifier":"verifier","token_url":"{token_url}"}}}}"#
    );

    let exchanged = invoke_mobile_protocol_json(&request, Some(data_dir.to_str().unwrap()));
    assert_eq!(exchanged["id"], 68, "{exchanged}");
    let account = &exchanged["result"]["account"];
    assert_eq!(account["id"], "outlook.user@example.com", "{exchanged}");
    assert_eq!(account["provider"], "outlook", "{exchanged}");
    assert_eq!(account["display_name"], "Outlook User", "{exchanged}");
    assert_eq!(account["sender_name"], "Outlook User", "{exchanged}");
    let conn = store::open_at(data_dir.join("meron.db")).unwrap();
    let config: String = conn
        .query_row(
            "SELECT config FROM accounts WHERE id = ?1",
            ["outlook.user@example.com"],
            |row| row.get(0),
        )
        .unwrap();
    let config: serde_json::Value = serde_json::from_str(&config).unwrap();
    assert_eq!(config["oauth_client_id"], "client", "{config}");
    assert_eq!(config["oauth_token_url"], token_url.as_str(), "{config}");

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_exchanges_outlook_code_saves_graph_photo() {
    use base64::Engine;
    use std::io::Write;
    use std::net::TcpListener;

    let data_dir = unique_data_dir("oauth-code-photo");
    let claims = serde_json::json!({ "preferred_username": "photo.user@example.com" });
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
    let code_body = format!(
        r#"{{"access_token":"imap","refresh_token":"refresh","expires_in":3600,"id_token":"h.{payload}.s"}}"#
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    // Code exchange, Graph token redemption, then the photo itself.
    std::thread::spawn(move || {
        for step in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let request = crate::test_support::read_http_request(&mut stream);
            let (content_type, body): (&str, Vec<u8>) = match step {
                0 => ("application/json", code_body.clone().into_bytes()),
                1 => ("application/json", br#"{"access_token":"graph"}"#.to_vec()),
                _ => {
                    assert!(request.starts_with("GET /photo "), "{request}");
                    assert!(request.contains("Bearer graph"), "{request}");
                    ("image/jpeg", b"\xff\xd8\xff\xe0photo".to_vec())
                }
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    let request = format!(
        r#"{{"id":69,"method":"account.exchangeOAuthCode","params":{{"email":"","provider":"outlook","code":"auth-code","client_id":"client","code_verifier":"verifier","token_url":"http://{addr}/token","photo_url":"http://{addr}/photo"}}}}"#
    );

    let exchanged = invoke_mobile_protocol_json(&request, Some(data_dir.to_str().unwrap()));
    let avatar_url = exchanged["result"]["account"]["avatar_url"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        avatar_url.starts_with("/media/avatars/photo_user_example_com/")
            && avatar_url.ends_with(".jpg"),
        "{exchanged}"
    );
    let saved = std::fs::read(data_dir.join(avatar_url.trim_start_matches('/'))).unwrap();
    assert_eq!(saved, b"\xff\xd8\xff\xe0photo");

    let _ = std::fs::remove_dir_all(data_dir);
}
