//go:build !linux && !darwin

// The tray renderer passes unread state and a style preference
// to each platform. This backend selects the color icon for its
// unread state and sends it directly to the native tray.

package main

// Windows keeps the colour icon: the setting isn't offered there.
func applyPlatformTrayIcon(unread, monochrome bool) error {
	traySetIcon(trayIcon(unread))
	return nil
}
