//go:build darwin

// Aligns the native macOS titlebar controls in the desktop window.
// The shared titlebar path calls this small Go-to-AppKit adapter.
// Its native counterpart positions the traffic-light buttons.

package main

/*
#cgo CFLAGS: -x objective-c
#cgo LDFLAGS: -framework AppKit

void alignTrafficLights(void);
*/
import "C"

func alignNativeTitlebar() {
	C.alignTrafficLights()
}
