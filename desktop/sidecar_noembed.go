//go:build !embed_sidecar

// Selects an external Rust core during desktop development.
// Without the embed_sidecar tag the host starts with an empty embedded image.
// The sidecar launcher then resolves the configured or locally built executable.

package main

// embeddedSidecar is empty in dev builds; the sidecar is found via the
// MERON_CORE_SERVER env var or the relative debug path. See sidecar_embed.go
// for the release build that bakes the binary in.
var embeddedSidecar []byte
