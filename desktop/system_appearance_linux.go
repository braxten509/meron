//go:build linux

// Reads the Linux color-scheme preference through the desktop
// portal. The listener supplies the initial appearance, observes
// changes and disconnects when the desktop host shuts down.

package main

import (
	"context"
	"sync"
	"time"

	"github.com/godbus/dbus/v5"
)

const (
	portalDestination     = "org.freedesktop.portal.Desktop"
	portalPath            = "/org/freedesktop/portal/desktop"
	portalSettings        = "org.freedesktop.portal.Settings"
	appearanceNamespace   = "org.freedesktop.appearance"
	appearanceColorScheme = "color-scheme"
)

var (
	appearanceBus *dbus.Conn
	appearanceMu  sync.Mutex
)

// setupAppearanceListener reads the desktop's color scheme from the XDG Settings portal and follows its SettingChanged signal.
//
// GNOME, KDE and the other portal backends all publish the preference there. Without a portal the
// preference stays unknown and the frontend falls back to prefers-color-scheme.
//
// Runs in the background: the first portal call can wait on D-Bus activation, and startup must not.
// The initial value reaches the frontend as an event.
func (a *App) setupAppearanceListener() {
	go func() {
		defer systemAppearanceSettled()
		conn, err := dbus.ConnectSessionBus()
		if err != nil {
			a.logf("appearance: connect session bus: %v", err)
			return
		}
		if err := conn.AddMatchSignal(
			dbus.WithMatchInterface(portalSettings),
			dbus.WithMatchMember("SettingChanged"),
			dbus.WithMatchArg(0, appearanceNamespace),
		); err != nil {
			a.logf("appearance: add SettingChanged match: %v", err)
			_ = conn.Close()
			return
		}

		appearanceMu.Lock()
		appearanceBus = conn
		appearanceMu.Unlock()

		c := make(chan *dbus.Signal, 4)
		conn.Signal(c)

		if scheme, err := readPortalColorScheme(conn); err != nil {
			a.logf("appearance: read color-scheme: %v", err)
		} else {
			a.setSystemAppearance(scheme == 1)
		}
		systemAppearanceSettled()

		for s := range c {
			if s.Name != portalSettings+".SettingChanged" || len(s.Body) != 3 {
				continue
			}
			if key, _ := s.Body[1].(string); key != appearanceColorScheme {
				continue
			}
			if value, ok := s.Body[2].(dbus.Variant); ok {
				if scheme, ok := value.Value().(uint32); ok {
					a.setSystemAppearance(scheme == 1)
				}
			}
		}
	}()
}

// readPortalColorScheme returns the portal's color-scheme: 0 no preference,
// 1 prefer dark, 2 prefer light.
func readPortalColorScheme(conn *dbus.Conn) (uint32, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	portal := conn.Object(portalDestination, portalPath)
	var value dbus.Variant
	err := portal.CallWithContext(ctx, portalSettings+".ReadOne", 0, appearanceNamespace, appearanceColorScheme).Store(&value)
	if err != nil {
		// Portals older than version 2 only have the deprecated Read, which
		// wraps the value in a second variant.
		if err := portal.CallWithContext(ctx, portalSettings+".Read", 0, appearanceNamespace, appearanceColorScheme).Store(&value); err != nil {
			return 0, err
		}
		if inner, ok := value.Value().(dbus.Variant); ok {
			value = inner
		}
	}
	scheme, _ := value.Value().(uint32)
	return scheme, nil
}

func (a *App) closeAppearanceListener() {
	appearanceMu.Lock()
	if appearanceBus != nil {
		_ = appearanceBus.Close()
		appearanceBus = nil
	}
	appearanceMu.Unlock()
}
