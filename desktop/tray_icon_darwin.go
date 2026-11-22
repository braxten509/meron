//go:build darwin

// The macOS tray renderer updates its menu bar item when unread
// state or appearance changes. Its template image follows the
// system appearance while the unread indicator is handled
// separately.

package main

import "fyne.io/systray"

var traySetTemplateIconWithBadge = systray.SetTemplateIconWithBadge

func applyPlatformTrayIcon(unread, monochrome bool) error {
	if monochrome {
		// Template only the base: AppKit would recolour a dot in this image
		// too. The badge is a separate view over the status item's button.
		traySetTemplateIconWithBadge(trayIconMonoPNG, unread)
	} else {
		traySetIcon(trayIcon(unread))
	}
	return nil
}
