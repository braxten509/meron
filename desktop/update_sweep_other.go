//go:build !windows

// The desktop startup path checks for files left over from
// replacing a running Windows executable. Unix systems
// replace the target atomically and need no startup cleanup.

package main

// sweepReplacedExecutable only has work to do on Windows, where a running exe
// must be renamed rather than overwritten. Elsewhere the rename over the target
// leaves nothing behind.
func sweepReplacedExecutable() {}
