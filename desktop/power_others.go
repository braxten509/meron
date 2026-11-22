//go:build !linux && !darwin && !windows

// Supplies the desktop lifecycle hooks on platforms without
// a native wake signal. Mailbox watches there recover through
// their connection keepalive and ordinary IMAP IDLE timeout.

package main

// No OS resume signal on this platform; IDLE recovery falls back to TCP
// keepalive and the IDLE timeout.
func (a *App) setupResumeListener() {}

func (a *App) closeResumeListener() {}
