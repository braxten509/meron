//! Resolves mobile mail server settings from the supplied address.
//! Built-in provider data, configuration XML and DNS supply connection candidates.
//! Account setup receives host names, ports and supported username templates.

use super::*;

pub(crate) fn autodiscover_mobile_account(params: &Value) -> Result<Value, String> {
    let email = required_string(params, "email")?.trim().to_string();
    let Some((_, domain)) = email.rsplit_once('@') else {
        return Err("invalid email".to_string());
    };
    let domain = domain.trim().to_ascii_lowercase();
    if domain.is_empty() {
        return Err("invalid email".to_string());
    }

    let discovered = mobile_provider_mail_settings(&email, &domain)
        .or_else(|| discover_mobile_autoconfig(&email, &domain))
        .or_else(|| discover_mobile_srv(&email, &domain))
        .unwrap_or_else(|| DiscoveredMailSettings {
            imap_host: format!("imap.{domain}"),
            imap_port: 993,
            smtp_host: format!("smtp.{domain}"),
            smtp_port: 465,
            username: email.clone(),
            source: "guess".to_string(),
            ..Default::default()
        });
    let mut result = json!({
        "imap_host": discovered.imap_host,
        "imap_port": discovered.imap_port,
        "smtp_host": discovered.smtp_host,
        "smtp_port": discovered.smtp_port,
        "username": discovered.username,
        "source": discovered.source,
    });
    if !discovered.provider_name.is_empty() {
        result["provider_name"] = json!(discovered.provider_name);
    }
    if let Some((provider, url)) =
        mobile_app_password_hint(&domain, result["imap_host"].as_str().unwrap_or_default())
    {
        result["app_password_hint"] = json!({ "provider": provider, "url": url });
    }
    Ok(result)
}

fn mobile_provider_mail_settings(email: &str, domain: &str) -> Option<DiscoveredMailSettings> {
    let settings = match domain {
        "gmail.com" | "googlemail.com" => ("Gmail", "imap.gmail.com", 993, "smtp.gmail.com", 465),
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => (
            "Outlook",
            "outlook.office365.com",
            993,
            "smtp.office365.com",
            587,
        ),
        "icloud.com" | "me.com" | "mac.com" => {
            ("iCloud", "imap.mail.me.com", 993, "smtp.mail.me.com", 587)
        }
        "yahoo.com" | "ymail.com" | "rocketmail.com" => (
            "Yahoo",
            "imap.mail.yahoo.com",
            993,
            "smtp.mail.yahoo.com",
            465,
        ),
        "aol.com" => ("AOL", "imap.aol.com", 993, "smtp.aol.com", 465),
        "fastmail.com" | "fastmail.fm" => (
            "Fastmail",
            "imap.fastmail.com",
            993,
            "smtp.fastmail.com",
            465,
        ),
        "proton.me" | "protonmail.com" => {
            ("Proton Mail Bridge", "127.0.0.1", 1143, "127.0.0.1", 1025)
        }
        _ => return None,
    };
    Some(DiscoveredMailSettings {
        provider_name: settings.0.to_string(),
        imap_host: settings.1.to_string(),
        imap_port: settings.2,
        smtp_host: settings.3.to_string(),
        smtp_port: settings.4,
        username: email.to_string(),
        source: "known".to_string(),
    })
}

fn discover_mobile_autoconfig(email: &str, domain: &str) -> Option<DiscoveredMailSettings> {
    let encoded_email: String = form_urlencoded::byte_serialize(email.as_bytes()).collect();
    let urls = [
        (
            format!(
                "https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={encoded_email}"
            ),
            "autoconfig",
        ),
        (
            format!(
                "https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress={encoded_email}"
            ),
            "autoconfig",
        ),
        (
            format!("https://autoconfig.thunderbird.net/v1.1/{domain}"),
            "thunderbird",
        ),
    ];
    for (url, source) in urls {
        let Some(xml) = fetch_mobile_autoconfig(&url) else {
            continue;
        };
        let Some(mut settings) = parse_mobile_autoconfig(&xml, email) else {
            continue;
        };
        settings.source = source.to_string();
        return Some(settings);
    }
    None
}

fn fetch_mobile_autoconfig(url: &str) -> Option<String> {
    let mut response = crate::proxy::agent()
        .ok()?
        .get(url)
        .header("User-Agent", "Meron-Mail-Autoconfig")
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(6)))
        .build()
        .call()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response
        .body_mut()
        .with_config()
        .limit(1 << 20)
        .read_to_string()
        .ok()
}

pub(super) fn parse_mobile_autoconfig(xml: &str, email: &str) -> Option<DiscoveredMailSettings> {
    let mut reader = Reader::from_str(xml);
    let decoder = reader.decoder();
    let mut buffer = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut display_name = String::new();
    let mut incoming: Vec<AutoconfigServer> = Vec::new();
    let mut outgoing: Vec<AutoconfigServer> = Vec::new();
    let mut current: Option<AutoconfigServer> = None;

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "incomingServer" || name == "outgoingServer" {
                    let mut server = AutoconfigServer::default();
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"type" {
                            server.server_type =
                                attr.decode_and_unescape_value(decoder).ok()?.to_string();
                        }
                    }
                    current = Some(server);
                }
                path.push(name);
            }
            Ok(Event::Text(e)) => {
                let text = e.unescape().ok()?.trim().to_string();
                if text.is_empty() {
                    buffer.clear();
                    continue;
                }
                match path.last().map(String::as_str) {
                    Some("displayName") if current.is_none() => display_name = text,
                    Some("hostname") => {
                        if let Some(server) = current.as_mut() {
                            server.hostname = text;
                        }
                    }
                    Some("port") => {
                        if let Some(server) = current.as_mut() {
                            server.port = text.parse::<u16>().unwrap_or(0);
                        }
                    }
                    Some("socketType") => {
                        if let Some(server) = current.as_mut() {
                            server.socket_type = text;
                        }
                    }
                    Some("username") => {
                        if let Some(server) = current.as_mut() {
                            server.username = text;
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name().as_ref().to_vec();
                if name.as_slice() == b"incomingServer" {
                    if let Some(server) = current.take() {
                        incoming.push(server);
                    }
                } else if name.as_slice() == b"outgoingServer" {
                    if let Some(server) = current.take() {
                        outgoing.push(server);
                    }
                }
                path.pop();
            }
            Ok(_) => {}
            Err(_) => return None,
        }
        buffer.clear();
    }

    let imap = incoming.into_iter().find(|server| {
        server.server_type.eq_ignore_ascii_case("imap") && !server.hostname.is_empty()
    })?;
    let smtp = outgoing.into_iter().find(|server| {
        server.server_type.eq_ignore_ascii_case("smtp") && !server.hostname.is_empty()
    })?;
    Some(DiscoveredMailSettings {
        imap_host: imap.hostname,
        imap_port: port_for_mobile_autoconfig(imap.port, &imap.socket_type, true),
        smtp_host: smtp.hostname,
        smtp_port: port_for_mobile_autoconfig(smtp.port, &smtp.socket_type, false),
        username: expand_mobile_autoconfig_username(&imap.username, email)
            .unwrap_or_else(|| email.to_string()),
        provider_name: display_name,
        source: String::new(),
    })
}

fn discover_mobile_srv(email: &str, domain: &str) -> Option<DiscoveredMailSettings> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .ok()?;
    let (imap_host, imap_port) = lookup_mobile_srv(&runtime, "imaps", domain)
        .or_else(|| lookup_mobile_srv(&runtime, "imap", domain))?;
    let (smtp_host, smtp_port) = lookup_mobile_srv(&runtime, "submissions", domain)
        .or_else(|| lookup_mobile_srv(&runtime, "submission", domain))?;
    Some(DiscoveredMailSettings {
        imap_host,
        imap_port,
        smtp_host,
        smtp_port,
        username: email.to_string(),
        source: "srv".to_string(),
        ..Default::default()
    })
}

fn lookup_mobile_srv(
    runtime: &tokio::runtime::Runtime,
    service: &str,
    domain: &str,
) -> Option<(String, u16)> {
    runtime.block_on(async {
        let resolver = Resolver::builder_tokio().ok()?.build().ok()?;
        let name = format!("_{service}._tcp.{domain}.");
        let response = tokio::time::timeout(Duration::from_secs(5), resolver.srv_lookup(name))
            .await
            .ok()?
            .ok()?;
        let mut records = response
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                RData::SRV(srv) => Some(srv),
                _ => None,
            })
            .collect::<Vec<_>>();
        records.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| b.weight.cmp(&a.weight))
        });
        let best = records.first()?;
        let target = best.target.to_utf8().trim_end_matches('.').to_string();
        if target.is_empty() || target == "." {
            return None;
        }
        Some((target, best.port))
    })
}

fn port_for_mobile_autoconfig(port: u16, socket_type: &str, imap: bool) -> u16 {
    if port != 0 {
        return port;
    }
    match socket_type.to_ascii_uppercase().as_str() {
        "STARTTLS" => {
            if imap {
                143
            } else {
                587
            }
        }
        "PLAIN" => {
            if imap {
                143
            } else {
                25
            }
        }
        _ => {
            if imap {
                993
            } else {
                465
            }
        }
    }
}

fn expand_mobile_autoconfig_username(template: &str, email: &str) -> Option<String> {
    if template.is_empty() {
        return None;
    }
    let local = email
        .split_once('@')
        .map(|(local, _)| local)
        .unwrap_or(email);
    Some(
        template
            .replace("%EMAILADDRESS%", email)
            .replace("%EMAILLOCALPART%", local),
    )
}

pub(crate) fn mobile_app_password_hint(
    domain: &str,
    imap_host: &str,
) -> Option<(&'static str, &'static str)> {
    let host = imap_host.to_ascii_lowercase();
    let contains = |needle: &str| host.contains(needle) || domain.contains(needle);
    if contains("mail.me.com")
        || contains("icloud.com")
        || contains("me.com")
        || contains("mac.com")
    {
        Some(("iCloud", "https://account.apple.com/account/manage"))
    } else if contains("fastmail") {
        Some((
            "Fastmail",
            "https://app.fastmail.com/settings/security/apppassword",
        ))
    } else if contains("yahoo") || contains("ymail") || contains("rocketmail") {
        Some((
            "Yahoo",
            "https://login.yahoo.com/account/security/app-passwords",
        ))
    } else if contains("aol.com") {
        Some((
            "AOL",
            "https://login.aol.com/account/security/app-passwords",
        ))
    } else {
        None
    }
}
