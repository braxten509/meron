//go:build !darwin

// Desktop startup calls this entry point to create the tray on
// Linux and Windows. Those platforms enter the shared tray
// initialization directly and select their native event loop
// there.

package main

func (a *App) startTray() {
	a.startTrayPhysical()
}
