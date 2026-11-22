//go:build !windows

// Supplies the process setup hook for the sidecar on
// non-Windows hosts. These operating systems can launch
// the child process using the ordinary command settings.

package main

import "os/exec"

// hideSidecarConsole is a no-op off Windows: no other platform gives a spawned
// child a window of its own.
func hideSidecarConsole(*exec.Cmd) {}
