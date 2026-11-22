//! Regression tests for accounts.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;

    #[test]
    fn mobile_autoconfig_parser_expands_username_and_socket_defaults() {
        let xml = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="example.com">
    <displayName>Example Mail</displayName>
    <incomingServer type="imap">
      <hostname>mail.example.com</hostname>
      <socketType>STARTTLS</socketType>
      <username>%EMAILLOCALPART%</username>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.com</hostname>
      <socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

        let settings = parse_mobile_autoconfig(xml, "alice@example.com").unwrap();
        assert_eq!(settings.provider_name, "Example Mail");
        assert_eq!(settings.imap_host, "mail.example.com");
        assert_eq!(settings.imap_port, 143);
        assert_eq!(settings.smtp_host, "smtp.example.com");
        assert_eq!(settings.smtp_port, 587);
        assert_eq!(settings.username, "alice");
    }

    #[test]
    fn mobile_autoconfig_parser_requires_imap_and_smtp() {
        let xml = r#"<clientConfig><emailProvider><incomingServer type="imap"><hostname>imap.example.com</hostname></incomingServer></emailProvider></clientConfig>"#;
        assert!(parse_mobile_autoconfig(xml, "alice@example.com").is_none());
    }
