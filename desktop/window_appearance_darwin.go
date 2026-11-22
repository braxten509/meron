//go:build darwin

// Applies the chosen desktop theme to native AppKit controls.
// The frontend draws the titlebar while menus, sheets
// and window controls follow the configured appearance.

package main

/*
#cgo CFLAGS: -x objective-c
#cgo LDFLAGS: -framework AppKit

void setAppAppearanceDark(int dark);
*/
import "C"

// setNativeWindowDark pins NSApp's appearance to the app's own theme.
//
// The title bar is hidden here (see main.go), but the appearance still drives
// the traffic lights, the native context menus, sheets and scroll bars, which
// would otherwise follow the system while the window paints the opposite theme.
func setNativeWindowDark(dark bool) {
	value := C.int(0)
	if dark {
		value = 1
	}
	C.setAppAppearanceDark(value)
}

// macOS hides the native title bar; the frontend's MacTitleBar strip already
// paints the side nav color.
func setNativeTitlebarColors(bg, fg string) {}

func refreshNativeTitlebarCss() {}
