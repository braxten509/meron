//go:build windows

// Reads the Windows app color preference and monitors its
// registry key. Changes update the desktop appearance until
// shutdown signals the listener to release its resources.

package main

import (
	"sync"

	"golang.org/x/sys/windows"
	"golang.org/x/sys/windows/registry"
)

const personalizeKey = `Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`

var (
	appearanceMu   sync.Mutex
	appearanceStop windows.Handle
)

// setupAppearanceListener reads the "Choose your app mode" setting
// (AppsUseLightTheme) and watches its registry key for changes on a
// background goroutine, until closeAppearanceListener signals the stop event.
func (a *App) setupAppearanceListener() {
	// Every early return leaves the preference unknown; the reading below
	// settles it otherwise.
	defer systemAppearanceSettled()
	key, err := registry.OpenKey(registry.CURRENT_USER, personalizeKey, registry.QUERY_VALUE|registry.NOTIFY)
	if err != nil {
		a.logf("appearance: open personalize key: %v", err)
		return
	}
	changed, err := windows.CreateEvent(nil, 0, 0, nil)
	if err != nil {
		a.logf("appearance: create event: %v", err)
		key.Close()
		return
	}
	stop, err := windows.CreateEvent(nil, 1, 0, nil)
	if err != nil {
		a.logf("appearance: create event: %v", err)
		windows.CloseHandle(changed)
		key.Close()
		return
	}
	appearanceMu.Lock()
	appearanceStop = stop
	appearanceMu.Unlock()

	report := func() {
		if light, _, err := key.GetIntegerValue("AppsUseLightTheme"); err == nil {
			a.setSystemAppearance(light == 0)
		}
	}
	// The first reading is taken here, before startup moves on, so the boot
	// query has it.
	report()

	go func() {
		defer key.Close()
		defer windows.CloseHandle(changed)
		for {
			if err := windows.RegNotifyChangeKeyValue(windows.Handle(key), false, windows.REG_NOTIFY_CHANGE_LAST_SET, changed, true); err != nil {
				a.logf("appearance: watch personalize key: %v", err)
				return
			}
			event, err := windows.WaitForMultipleObjects([]windows.Handle{changed, stop}, false, windows.INFINITE)
			if err != nil || event != windows.WAIT_OBJECT_0 {
				return
			}
			report()
		}
	}()
}

func (a *App) closeAppearanceListener() {
	appearanceMu.Lock()
	stop := appearanceStop
	appearanceStop = 0
	appearanceMu.Unlock()
	if stop != 0 {
		// The watcher goroutine wakes on this and returns; the handle itself is
		// left to process exit, since the goroutine may still be waiting on it.
		windows.SetEvent(stop)
	}
}
