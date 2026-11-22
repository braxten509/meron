//go:build !linux

// The tray renderer periodically asks whether its cached icon needs
// repair. Linux stores icons where the shell can remove them. Other
// backends supply the icon directly and report a healthy cache.

package main

func platformTrayIconNeedsRepair() bool { return false }
