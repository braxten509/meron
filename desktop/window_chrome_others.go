//go:build (!linux && !windows) || bindings

// The desktop host queries these capabilities before offering
// integrated window controls. Platforms using this backend rely on
// their default window frame and report which titlebar options are
// available.

package main

// macOS always hides its title bar (main.go); elsewhere there is no frame to
// keep without one.
const integratedTitlebarSupported = false

// See window_chrome_linux.go.
const framelessTitlebar = false

var titlebarSwitchesLive = func() bool { return false }

func installWindowChrome(integrated bool) {}

func setNativeTitlebarIntegrated(integrated bool) {}

func closeNativeWindow() {}

func nativeChromeSettings() (layout, doubleClick string) { return "", "" }

func nativeWindowTiled() bool { return false }

func nativeDrawsFrame() bool { return false }
