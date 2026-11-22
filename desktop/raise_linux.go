//go:build linux

// Notification clicks and second-instance launches need to
// bring the main window forward. On Linux this involves
// remapping the GTK window so the window manager can activate
// it.

package main

import "context"

// showAndRaiseMainWindow brings the window to the front, focused.
//
// The Wails runtime maps onto three GTK calls here: WindowShow is gtk_widget_show, WindowHide is
// gtk_widget_hide, WindowUnminimise is gtk_window_present.
//
// Neither show nor present raises a window that is merely behind another one: mutter refuses
// activation requests from a process that does not hold the focus and has no activation token to pass,
// on Wayland and on X11 alike, and a notification click is exactly that case. What mutter does honour
// is a freshly mapped window, so unmapping and mapping again is the only route back to the front.
//
// GTK reapplies the maximised state across the remap, so a maximised window returns maximised.
func showAndRaiseMainWindow(requestContext context.Context) {
	windowHide(requestContext)
	windowShow(requestContext)
	windowUnminimise(requestContext)
}
