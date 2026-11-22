//go:build embed_sidecar

// Supplies the bundled core binary for desktop release builds.
// The embed_sidecar tag selects the executable staged by the build script.
// The host uses those bytes when starting the Rust sidecar.

package main

import _ "embed"

// embeddedSidecar holds the Rust core engine sidecar binary, baked into release
// builds via `wails build -tags embed_sidecar`. The build script stages the
// freshly built release binary at build/sidecar/meron-core before the Go compile.
//
//go:embed build/sidecar/meron-core
var embeddedSidecar []byte
