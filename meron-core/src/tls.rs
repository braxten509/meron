//! TLS client setup for the IMAP and SMTP transports, plus trust-on-first-use (TOFU) certificate pinning.
//!
//! Normal mail servers present a chain that webpki validates against the bundled Mozilla roots.
//!
//! Local bridges do not: Proton Mail Bridge, for one, serves a self-signed certificate that carries
//! `basicConstraints: CA:TRUE` and then presents it as the leaf, which rustls rejects outright
//! (`CaUsedAsEndEntity`) before trust anchors ever matter — installing that certificate as a root
//! would not help.
//!
//! The only way to reach such a server is to pin the exact certificate the user looked at and
//! accepted.
//!
//! So an account may carry a `cert_pin`: the hex SHA-256 of the leaf DER it is willing to talk to.
//! When the presented leaf hashes to that pin we accept it without chain or hostname checks;
//! anything else still goes through webpki, so a rotated bridge certificate re-prompts instead of
//! silently connecting. [`probe`] fetches the certificate for that prompt.

use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

/// Marker embedded in the error message of a handshake that failed on the
/// server's certificate rather than on the network. The account dialog keys off
/// it to offer the "inspect and trust this certificate" flow instead of showing
/// a raw rustls error.
pub const UNTRUSTED_CERT_MARKER: &str = "untrusted-certificate";

/// The same for the submission server, so the dialog knows which of the two
/// servers to probe and which pin to set. Contains [`UNTRUSTED_CERT_MARKER`] as
/// a substring: a caller that only asks "is this a certificate problem?" needs
/// to match one marker, not two.
pub const UNTRUSTED_SMTP_CERT_MARKER: &str = "smtp-untrusted-certificate";

/// A handshake the peer's certificate failed. Carried as a typed error so
/// callers can tell it apart from a network failure without matching strings;
/// its `Display` is what reaches the UI.
#[derive(Debug)]
pub struct UntrustedCertificate {
    detail: String,
}

impl std::fmt::Display for UntrustedCertificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tls handshake: {UNTRUSTED_CERT_MARKER}: {}", self.detail)
    }
}

impl std::error::Error for UntrustedCertificate {}

impl UntrustedCertificate {
    /// The failure without the marker, for a caller that re-tags it (the SMTP
    /// check reports the submission server's marker instead).
    pub fn detail(&self) -> &str {
        &self.detail
    }

    /// Wraps `err` when it is a certificate rejection, and returns it unchanged
    /// otherwise.
    pub fn from_io(error: std::io::Error) -> anyhow::Error {
        if is_cert_error(&error) {
            anyhow::Error::new(UntrustedCertificate {
                detail: error.to_string(),
            })
        } else {
            anyhow::Error::new(error).context("tls handshake")
        }
    }
}

/// What the user is asked to accept: the leaf certificate as the server sent it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CertInfo {
    /// Hex SHA-256 of the leaf DER — the value stored as the account's pin.
    pub fingerprint: String,
    pub subject: String,
    pub issuer: String,
    /// RFC 2822 validity bounds, or empty when the certificate does not parse.
    pub not_before: String,
    pub not_after: String,
    pub self_signed: bool,
}

pub fn fingerprint(der: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(der);
    let sum = hasher.finalize();
    let mut out = String::with_capacity(sum.len() * 2);
    for byte in sum {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn describe(der: &CertificateDer<'_>) -> CertInfo {
    let fingerprint = fingerprint(der);
    match x509_parser::parse_x509_certificate(der) {
        Ok((_, cert)) => CertInfo {
            fingerprint,
            subject: cert.subject().to_string(),
            issuer: cert.issuer().to_string(),
            not_before: cert.validity().not_before.to_rfc2822().unwrap_or_default(),
            not_after: cert.validity().not_after.to_rfc2822().unwrap_or_default(),
            self_signed: cert.subject() == cert.issuer(),
        },
        Err(_) => CertInfo {
            fingerprint,
            subject: String::new(),
            issuer: String::new(),
            not_before: String::new(),
            not_after: String::new(),
            self_signed: false,
        },
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn webpki_verifier() -> Result<Arc<WebPkiServerVerifier>> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider())
        .build()
        .context("build certificate verifier")
}

/// Accepts one specific leaf certificate by fingerprint; everything else falls
/// through to normal webpki validation. Handshake signature checks are always
/// the real ones — pinning replaces *identity* verification, not cryptography.
#[derive(Debug)]
struct PinnedVerifier {
    pin: String,
    inner: Arc<WebPkiServerVerifier>,
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if fingerprint(end_entity) == self.pin {
            return Ok(ServerCertVerified::assertion());
        }
        self.inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// Records the leaf certificate and accepts it, so [`probe`] can show the user
/// what the server actually presented. Only ever used for a handshake that is
/// dropped immediately afterwards — never to carry credentials.
#[derive(Debug)]
struct CapturingVerifier {
    captured: Mutex<Option<CertificateDer<'static>>>,
    inner: Arc<WebPkiServerVerifier>,
}

impl ServerCertVerifier for CapturingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        *self.captured.lock().unwrap() = Some(end_entity.clone().into_owned());
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn config_with_verifier(verifier: Arc<dyn ServerCertVerifier>) -> Result<rustls::ClientConfig> {
    Ok(rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .context("tls protocol versions")?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth())
}

/// The connector used by the mail transports. `pin`, when present, is the
/// account's accepted leaf fingerprint.
pub fn connector(pin: Option<&str>) -> Result<tokio_rustls::TlsConnector> {
    let inner = webpki_verifier()?;
    let config = match pin {
        Some(pin) if !pin.is_empty() => config_with_verifier(Arc::new(PinnedVerifier {
            pin: pin.to_ascii_lowercase(),
            inner,
        }))?,
        _ => rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .context("tls protocol versions")?
            .with_webpki_verifier(inner)
            .with_no_client_auth(),
    };
    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)))
}

/// True when a handshake failure was the peer's certificate (untrusted, expired,
/// wrong name, malformed) rather than a transport problem.
pub fn is_cert_error(error: &std::io::Error) -> bool {
    matches!(
        error
            .get_ref()
            .and_then(|e| e.downcast_ref::<rustls::Error>()),
        Some(rustls::Error::InvalidCertificate(_))
    )
}

/// Handshake with `host` over an already-connected socket purely to read back
/// the leaf certificate. The session is discarded; nothing is sent over it.
async fn capture(host: &str, tcp: TcpStream) -> Result<CertInfo> {
    let verifier = Arc::new(CapturingVerifier {
        captured: Mutex::new(None),
        inner: webpki_verifier()?,
    });
    let connector =
        tokio_rustls::TlsConnector::from(Arc::new(config_with_verifier(verifier.clone())?));
    // An unparseable server name (a bare IP is fine, garbage is not) would fail
    // the handshake before we ever see a certificate.
    let server_name = ServerName::try_from(host.to_string()).context("invalid server name")?;
    let handshake: Result<TlsStream<TcpStream>> = connector
        .connect(server_name, tcp)
        .await
        .context("tls handshake");
    let captured = verifier.captured.lock().unwrap().take();
    match captured {
        // The handshake may still fail after certificate selection (a signature
        // or version mismatch); the certificate we captured is what the user
        // needs to see either way.
        Some(der) => Ok(describe(&der)),
        None => Err(handshake
            .err()
            .unwrap_or_else(|| anyhow::anyhow!("server sent no certificate"))),
    }
}

/// Fetch the certificate a server presents, so the user can look at it and
/// decide whether to pin it. `protocol` is `"imap"` or `"smtp"` and `starttls`
/// selects the cleartext-then-upgrade form those protocols use on 143/587 (and
/// on Proton Bridge's 1143/1025).
pub async fn probe(
    host: &str,
    port: u16,
    protocol: &str,
    starttls: bool,
    proxy: Option<&crate::proxy::ProxyConfig>,
) -> Result<CertInfo> {
    probe_with_timeout(host, port, protocol, starttls, proxy, PROBE_TIMEOUT).await
}

async fn probe_with_timeout(
    host: &str,
    port: u16,
    protocol: &str,
    starttls: bool,
    proxy: Option<&crate::proxy::ProxyConfig>,
    timeout: std::time::Duration,
) -> Result<CertInfo> {
    tokio::time::timeout(timeout, async {
        let tcp = crate::imap::open_socket(host, port, proxy).await?;
        let tcp = if starttls {
            match protocol {
                "smtp" => crate::smtp::starttls_socket(tcp).await?,
                _ => crate::imap::starttls_socket(tcp).await?,
            }
        } else {
            tcp
        };
        capture(host, tcp).await
    })
    .await
    .map_err(|_| anyhow::anyhow!("certificate probe: timed out"))?
}

/// Matches the connect path's handshake cap: a probe runs while the user waits
/// on the account dialog.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probe_timeout_includes_starttls_greeting() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (_tcp, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });

        let started = tokio::time::Instant::now();
        let error = probe_with_timeout(
            "127.0.0.1",
            port,
            "imap",
            true,
            None,
            std::time::Duration::from_millis(50),
        )
        .await
        .unwrap_err();
        server.abort();

        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn fingerprint_is_lowercase_hex_sha256() {
        assert_eq!(
            fingerprint(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// A matching pin short-circuits chain building entirely — which is the
    /// whole point: Proton Bridge's leaf is a CA certificate and never gets past
    /// webpki, no matter which roots are configured.
    #[test]
    fn pinned_leaf_is_accepted_and_others_are_not() {
        let leaf = CertificateDer::from(b"not a real certificate".to_vec());
        let verifier = PinnedVerifier {
            pin: fingerprint(&leaf),
            inner: webpki_verifier().unwrap(),
        };
        let name = ServerName::try_from("127.0.0.1").unwrap();
        assert!(
            verifier
                .verify_server_cert(&leaf, &[], &name, &[], UnixTime::now())
                .is_ok()
        );

        let other = CertificateDer::from(b"a different certificate".to_vec());
        assert!(
            verifier
                .verify_server_cert(&other, &[], &name, &[], UnixTime::now())
                .is_err()
        );
    }

    /// The submission server's marker has to contain the general one, so a
    /// caller asking only "was this a certificate problem?" matches both.
    #[test]
    fn smtp_marker_is_recognized_as_a_certificate_failure() {
        assert!(UNTRUSTED_SMTP_CERT_MARKER.contains(UNTRUSTED_CERT_MARKER));
    }

    #[test]
    fn untrusted_certificate_errors_carry_the_marker_and_stay_typed() {
        let err = UntrustedCertificate::from_io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
                std::sync::Arc::new(std::io::Error::other("CaUsedAsEndEntity")),
            ))),
        ));
        assert!(err.to_string().contains(UNTRUSTED_CERT_MARKER));
        assert!(err.downcast_ref::<UntrustedCertificate>().is_some());

        // A transport failure keeps its own error, so the SMTP check can tell
        // "certificate rejected" from "server unreachable".
        let network = UntrustedCertificate::from_io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        ));
        assert!(network.downcast_ref::<UntrustedCertificate>().is_none());
    }

    #[test]
    fn certificate_failures_are_distinguished_from_transport_failures() {
        let cert_err = std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            // Exactly how a Proton Bridge handshake fails: webpki's
            // CaUsedAsEndEntity, wrapped by rustls as an opaque Other.
            rustls::Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
                std::sync::Arc::new(std::io::Error::other("CaUsedAsEndEntity")),
            ))),
        );
        assert!(is_cert_error(&cert_err));

        let net_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        assert!(!is_cert_error(&net_err));
    }
}
