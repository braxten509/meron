//go:build !linux || bindings

// Desktop startup keeps the window hidden until the frontend is
// ready. This backend uses the Wails show operation to reveal
// it and supplies the initial window state to the host.

package main

import (
	"context"

	"github.com/wailsapp/wails/v2/pkg/options"
	wailsRuntime "github.com/wailsapp/wails/v2/pkg/runtime"
)

const startWindowHidden = true

func revealStartupWindow(requestContext context.Context) {
	wailsRuntime.WindowShow(requestContext)
}

// WindowShow completes the reveal immediately on these platforms. Keeping
// force separate prevents the fallback from reopening a window hidden later.
func forceRevealStartupWindow(requestContext context.Context) {}

// startWindowState maximises the window as it opens when the last session
// ended maximised. Windows and macOS keep a sane restore geometry when a window
// starts maximised, so nothing else is needed there.
func startWindowState(maximised bool) options.WindowStartState {
	if maximised {
		return options.Maximised
	}
	return options.Normal
}

func maximiseOnDomReady(requestContext context.Context, maximised bool) {}
