//go:build !darwin && !linux && !windows

// The desktop updater selects an installer for the current operating
// system at build time. On platforms without a supported installer
// this implementation returns the error shown to the caller.

package main

import "fmt"

func applyUpdate(channel updateChannel, payload string) error {
	return fmt.Errorf("update: in-app updates are not supported on this platform")
}
