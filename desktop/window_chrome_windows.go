//go:build windows && !bindings

// Windows uses a frameless Wails window for the integrated titlebar.
// The desktop host reads these capabilities before creating the native
// window and when deciding whether a preference needs a restart.

package main

// The integrated title bar on Windows is Wails' frameless window: no caption,
// but DWM keeps the shadow, the Windows 11 rounded corners and the resize
// borders (WS_THICKFRAME; Wails' runtime resizes from the page's edges), and a
// --wails-draggable press is a real caption drag, so Aero Snap works. What it
// can't offer is the Windows 11 Snap Layouts flyout on the maximize button:
// that needs WM_NCHITTEST to answer HTMAXBUTTON, and the WebView2 child window
// takes the mouse before the frame sees it. Win+Z still opens it.
const integratedTitlebarSupported = true

const framelessTitlebar = true

// Frameless is a creation-time option in Wails v2, so a switch applies on the
// next launch.
var titlebarSwitchesLive = func() bool { return false }

func installWindowChrome(integrated bool) {}

func setNativeTitlebarIntegrated(integrated bool) {}

// closeNativeWindow posts WM_CLOSE, so the close runs through Wails'
// OnBeforeClose and close-to-tray exactly like the system close button.
func closeNativeWindow() {
	if hwnd := findMainWindow(); hwnd != 0 {
		procPostMessage.Call(hwnd, wmClose, 0, 0)
	}
}

// nativeChromeSettings: Windows has one button layout, on the right, and a
// double-click on the caption maximizes.
func nativeChromeSettings() (layout, doubleClick string) {
	return ":minimize,maximize,close", "toggle-maximize"
}

func nativeWindowTiled() bool { return false }

// DWM draws the frame around a frameless window too.
func nativeDrawsFrame() bool { return true }
