# Meron's copy of tinfoil-rs

Upstream: https://github.com/tinfoilsh/tinfoil-rs at e33eea56ebecc9e4bbcd79158ab4d3db534d1e19 (v0.2.2).
Used through `[patch."https://github.com/tinfoilsh/tinfoil-rs"]` in meron-core/Cargo.toml.

One change: `plain_http_builder()` in src/lib.rs makes the unpinned fetches
(src/verifier/util.rs, src/ehbp_transport.rs) trust the bundled webpki roots.
reqwest's default rustls-platform-verifier panics on Android ("Expect
rustls-platform-verifier to be initialized") without a JNI init. The enclave
attestation and the TLS key pin are unchanged.

To update: copy the new upstream tag's Cargo.toml, LICENSE, README.md, src
and assets over this folder, re-apply the change above, and update the rev.
