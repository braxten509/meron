//go:build darwin

// Desktop startup must create the macOS status item on the
// main thread. A Foundation callback dispatches the request
// there and then calls the shared Go tray initializer.

package main

/*
#cgo CFLAGS: -x objective-c
#cgo LDFLAGS: -framework Foundation

#import <Foundation/Foundation.h>

extern void goStartTray();

static inline void startTrayOnMainThread() {
    dispatch_async(dispatch_get_main_queue(), ^{
        goStartTray();
    });
}
*/
import "C"

func (a *App) startTray() {
	C.startTrayOnMainThread()
}

//export goStartTray
func goStartTray() {
	if globalApp != nil {
		globalApp.startTrayPhysical()
	}
}
