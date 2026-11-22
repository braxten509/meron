//go:build !windows

// The updater looks for a previous installation error during
// startup. Windows can leave a failure record from its
// installer helper. Other hosts return an empty status here.

package main

func pendingUpdateError() string { return "" }
