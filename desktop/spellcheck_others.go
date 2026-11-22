//go:build !linux || (linux && (!webkit2_41 || bindings))

// Provides the spell-check startup hook for other desktop builds.
// The shared lifecycle calls the same entry point on every platform.
// WebKit-specific configuration belongs to the Linux implementation.

package main

func setupNativeSpellChecking() {}
