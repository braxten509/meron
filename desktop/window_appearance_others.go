//go:build (!linux && !darwin && !windows) || (linux && bindings)

// The desktop frontend sends its appearance to the host when the
// theme changes. This fallback implements the native window tint
// callbacks on targets where the host supplies no native tinting.

package main

// No native chrome to tint here.
func setNativeWindowDark(dark bool) {}

func setNativeTitlebarColors(bg, fg string) {}

func refreshNativeTitlebarCss() {}
