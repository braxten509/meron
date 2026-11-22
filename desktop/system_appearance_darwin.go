//go:build darwin

// Listens for macOS system appearance changes. Startup
// registers the native callback, which reports the light or
// dark setting used when the desktop theme follows the system.

package main

/*
#cgo CFLAGS: -x objective-c
#cgo LDFLAGS: -framework Foundation -framework AppKit

void setupAppearanceObserver();
void teardownAppearanceObserver();
*/
import "C"

//export goSystemAppearanceChanged
func goSystemAppearanceChanged(dark C.int) {
	if globalApp == nil {
		return
	}
	// Called on the main thread; the event emit must not block AppKit.
	go globalApp.setSystemAppearance(dark != 0)
}

func (a *App) setupAppearanceListener() {
	C.setupAppearanceObserver()
}

func (a *App) closeAppearanceListener() {
	C.teardownAppearanceObserver()
}
