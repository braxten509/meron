// The host reports window capabilities before the frontend
// displays titlebar settings. Tests cover platform-specific
// controls and applying a saved choice live or after restart.

package main

import (
	"context"
	"os"
	"testing"
)

func stubWindowChrome(t *testing.T) *[]bool {
	t.Helper()
	applied := &[]bool{}
	apply, read, drawsFrame, live, was := applyNativeTitlebar, readChromeSettings, windowDrawsFrame, titlebarSwitchesLive, integratedTitlebar.Load()
	applyNativeTitlebar = func(integrated bool) { *applied = append(*applied, integrated) }
	readChromeSettings = func() (string, string) { return "menu:close", "toggle-maximize" }
	windowDrawsFrame = func() bool { return true }
	titlebarSwitchesLive = func() bool { return true }
	t.Cleanup(func() {
		applyNativeTitlebar, readChromeSettings, windowDrawsFrame, titlebarSwitchesLive = apply, read, drawsFrame, live
		integratedTitlebar.Store(was)
	})
	integratedTitlebar.Store(false)
	return applied
}

// The choice is saved straight away (the next launch reads it before the
// window exists), as an opt-out from the default, and applied to the window
// only when it changes.
func TestWindowSetTitlebarSavesAndApplies(t *testing.T) {
	applied := stubWindowChrome(t)
	app := newWindowStateApp(t)

	for _, integrated := range []bool{true, true, false} {
		if _, err := app.windowSetTitlebar(map[string]any{"integrated": integrated}); err != nil {
			t.Fatal(err)
		}
		want := ""
		if !integrated {
			want = titlebarSystem
		}
		if got := loadWindowState(app.windowStatePath).Titlebar; got != want {
			t.Fatalf("integrated=%v: saved titlebar %q, want %q", integrated, got, want)
		}
	}
	want := []bool{}
	if integratedTitlebarSupported {
		want = []bool{true, false}
	}
	if len(*applied) != len(want) || (len(want) == 2 && ((*applied)[0] != true || (*applied)[1] != false)) {
		t.Fatalf("applied %v, want %v", *applied, want)
	}
}

func TestLoadWindowStateKeepsOnlyTheSystemTitlebarOptOut(t *testing.T) {
	app := newWindowStateApp(t)
	for saved, want := range map[string]string{"system": titlebarSystem, "integrated": "", "fancy": ""} {
		state := `{"width":900,"height":700,"titlebar":"` + saved + `"}`
		if err := os.WriteFile(app.windowStatePath, []byte(state), 0o600); err != nil {
			t.Fatal(err)
		}
		if got := loadWindowState(app.windowStatePath).Titlebar; got != want {
			t.Fatalf("saved %q: titlebar %q, want %q", saved, got, want)
		}
	}
}

func TestWindowChromeReportsDesktopSettings(t *testing.T) {
	stubWindowChrome(t)
	integratedTitlebar.Store(true)
	app := newWindowStateApp(t)

	result, err := app.windowChrome()
	if err != nil {
		t.Fatal(err)
	}
	got := result.(map[string]any)
	if got["integrated"] != true || got["layout"] != "menu:close" || got["doubleClick"] != "toggle-maximize" {
		t.Fatalf("got %v", got)
	}
	if got["supported"] != integratedTitlebarSupported {
		t.Fatalf("supported = %v", got["supported"])
	}
}

// The close control goes through the window's own close, so close-to-tray
// applies to it exactly as to the title bar's close button.
func TestWindowControlsRouteToTheWindow(t *testing.T) {
	var calls []string
	minimise, toggle, close := windowMinimise, windowToggleMaximise, windowClose
	windowMinimise = func(context.Context) { calls = append(calls, "minimise") }
	windowToggleMaximise = func(context.Context) { calls = append(calls, "toggleMaximise") }
	windowClose = func() { calls = append(calls, "close") }
	t.Cleanup(func() { windowMinimise, windowToggleMaximise, windowClose = minimise, toggle, close })
	app := newWindowStateApp(t)

	for _, command := range []string{"window.minimise", "window.toggleMaximise", "window.close"} {
		if _, err := app.windowControl(command); err != nil {
			t.Fatal(err)
		}
	}
	if len(calls) != 3 || calls[0] != "minimise" || calls[1] != "toggleMaximise" || calls[2] != "close" {
		t.Fatalf("calls %v", calls)
	}
}

// KDE's native frame does not prevent selecting the custom titlebar. The
// choice is saved while the current window stays native until restart.
func TestIntegratedTitlebarOfferedOnADesktopFrame(t *testing.T) {
	applied := stubWindowChrome(t)
	windowDrawsFrame = func() bool { return false }
	titlebarSwitchesLive = func() bool { return false }
	app := newWindowStateApp(t)
	app.window.Titlebar = titlebarSystem
	app.flushWindowState()

	if _, err := app.windowSetTitlebar(map[string]any{"integrated": true}); err != nil {
		t.Fatal(err)
	}
	result, err := app.windowChrome()
	if err != nil {
		t.Fatal(err)
	}
	got := result.(map[string]any)
	if got["supported"] != integratedTitlebarSupported || got["integrated"] != false || got["wanted"] != integratedTitlebarSupported {
		t.Fatalf("got %v", got)
	}
	if got := loadWindowState(app.windowStatePath).Titlebar; got != "" {
		t.Fatalf("saved titlebar %q, want empty", got)
	}
	if len(*applied) != 0 || integratedTitlebarActive() {
		t.Fatalf("applied %v: want unchanged native titlebar until restart", *applied)
	}
}

// A custom titlebar can remain active without transparent GTK shadows
// (solid-csd). Rounded corners are reported separately by windowResized.
func TestIntegratedTitlebarActiveWithoutTransparentGtkFrame(t *testing.T) {
	stubWindowChrome(t)
	windowDrawsFrame = func() bool { return false }
	integratedTitlebar.Store(true)
	if integratedTitlebarActive() != integratedTitlebarSupported {
		t.Fatal("want active on supported platforms regardless of frame ownership")
	}
}

// Where the switch can't apply to the live window (Windows' frameless option
// is fixed at creation), the choice is saved and reported as wanted, and the
// title bar in effect stays until the next launch.
func TestWindowSetTitlebarWaitsForRestartWhereNotLive(t *testing.T) {
	applied := stubWindowChrome(t)
	titlebarSwitchesLive = func() bool { return false }
	integratedTitlebar.Store(true)
	app := newWindowStateApp(t)

	if _, err := app.windowSetTitlebar(map[string]any{"integrated": false}); err != nil {
		t.Fatal(err)
	}
	if got := loadWindowState(app.windowStatePath).Titlebar; got != titlebarSystem {
		t.Fatalf("saved titlebar %q, want %q", got, titlebarSystem)
	}
	if len(*applied) != 0 || !integratedTitlebar.Load() {
		t.Fatalf("applied %v, integrated %v: want nothing until restart", *applied, integratedTitlebar.Load())
	}
	if !integratedTitlebarSupported {
		return
	}
	result, err := app.windowChrome()
	if err != nil {
		t.Fatal(err)
	}
	if got := result.(map[string]any); got["integrated"] != true || got["wanted"] != false {
		t.Fatalf("got %v, want integrated now and system wanted", got)
	}
}
