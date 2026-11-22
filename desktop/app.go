// Coordinates the desktop app lifecycle with the Rust sidecar.
// Wails supplies the window context and calls the startup and shutdown hooks.
// Core startup errors are retained for commands issued before the engine is ready.

package main

import (
	"context"
	"errors"
	"fmt"
	"log"
	"net"
	"os"
	"path/filepath"
	"runtime"
	"sync"
	"sync/atomic"
	"time"

	wailsRuntime "github.com/wailsapp/wails/v2/pkg/runtime"
)

type App struct {
	mcp           *mcpService
	ctx           context.Context
	sidecar       *Sidecar
	logger        *log.Logger
	logFile       *os.File
	tray          trayRenderer
	trayInit      sync.Once
	trayStylePath string
	trayStyleMu   sync.Mutex
	trayStop      func()
	trayStopOnce  sync.Once
	// closeToTray is pushed by the frontend. Tray style is restored from
	// native storage before publication.
	closeToTray     atomic.Bool
	quitting        atomic.Bool
	closing         atomic.Bool
	windowHidden    atomic.Bool
	windowReadyOnce sync.Once

	windowMu        sync.Mutex
	window          windowState
	windowSaved     windowState
	windowSaveTimer *time.Timer
	windowStatePath string

	// At most one bulk .eml export runs at a time; emlExportCancel is non-nil
	// for as long as it does.
	emlExportMu     sync.Mutex
	emlExportCancel context.CancelFunc

	filePickerMu sync.Mutex
	lastFileDir  string
	lastImageDir string

	// The paths of the latest native file drop (Linux), see filedrop.go.
	fileDropMu   sync.Mutex
	droppedPaths []string
	droppedAt    time.Time

	oauthMu          sync.Mutex
	oauthState       string
	oauthVerifier    string
	oauthProvider    string
	oauthRedirectURI string
	oauthProfile     *ExchangedProfile
	// The running sign-in attempt: its callback listener, the page it opened
	// (for "Open the page again") and why it failed, if it did. oauthAttempt
	// counts attempts so a stale timeout never ends a newer one.
	oauthListener net.Listener
	oauthAuthURL  string
	oauthFailure  string
	oauthAttempt  int

	mailtoMu      sync.Mutex
	pendingMailto []string

	updaterOnce        sync.Once
	updater            *updater
	updateChannelOnce  sync.Once
	updateChannelValue updateChannel

	coreErrorMu sync.Mutex
	coreError   string
}

// setCoreError remembers the last fatal condition the core reported, so the
// generic "engine unavailable" replies can name the actual cause.
func (a *App) setCoreError(message string) {
	a.coreErrorMu.Lock()
	a.coreError = message
	a.coreErrorMu.Unlock()
}

func (a *App) lastCoreError() string {
	a.coreErrorMu.Lock()
	defer a.coreErrorMu.Unlock()
	return a.coreError
}

// engineUnavailable is the error every handler returns when the core isn't
// usable. It carries the core's own error message when there is one: without
// it the UI can only say the engine is unavailable, which is true of a missing
// binary, a crashed process, and an unreachable keychain alike.
func (a *App) engineUnavailable() error {
	if message := a.lastCoreError(); message != "" {
		return fmt.Errorf("mail engine unavailable: %s", message)
	}
	return errors.New("mail engine unavailable")
}

func NewApp() *App {
	loadDotEnvLocal()
	logger, logFile := openAppLog()
	captureCrashes(logFile)
	app := &App{logger: logger, logFile: logFile, windowStatePath: windowStatePath()}
	app.window = loadWindowState(app.windowStatePath)
	app.windowSaved = app.window
	app.closeToTray.Store(true)
	app.trayStylePath = filepath.Join(appConfigDirectory(), "tray.json")
	app.tray.desired.monochrome = loadTrayStyle(app.trayStylePath)
	return app
}

func (a *App) Startup(requestContext context.Context) {
	a.ctx = requestContext
	a.armStartupWindowFallback(startupWindowRevealDelay)
	a.quitOnSignal()
	a.queueStartupMailtoURLs(os.Args[1:])
	if wailsRuntime.Environment(requestContext).BuildType != "dev" {
		installDesktopEntry()
	}
	// A staged update either landed on the last quit or was abandoned; either
	// way it is re-downloadable, so start clean.
	sweepReplacedExecutable()
	cleanupUpdateCache()
	a.startTray()
	a.logf("startup: core path=%s", coreBinaryPath())
	a.sidecar = NewSidecar(coreBinaryPath(), a.logWriter())
	a.sidecar.onEvent = a.handleSidecarEvent
	if err := a.sidecar.Start(requestContext); err != nil {
		a.setCoreError(fmt.Sprintf("core failed to start: %v (path: %s)", err, coreBinaryPath()))
		a.logf("core failed to start: %v", err)
		fmt.Fprintf(os.Stderr, "meron: core failed to start: %v (path: %s)\n", err, coreBinaryPath())
	} else {
		a.logf("core started")
	}
	a.startMCP()
	a.setupNotificationListener()
	go a.runReminders()
	a.setupResumeListener()
	a.setupFileDrop(requestContext)
	a.setupAppearanceListener()
}

// onSystemResumed tells the sidecar the host woke from suspend so its IDLE
// watchers drop sockets that died during sleep and reconnect immediately,
// instead of waiting out TCP keepalive / the IDLE timeout. Called from the
// per-platform resume listeners.
func (a *App) onSystemResumed() {
	a.logf("resume: system woke, kicking IDLE watchers")
	if a.sidecar != nil && a.sidecar.Started() {
		if _, err := a.sidecar.Call("system.resumed", nil); err != nil {
			a.logf("resume: system.resumed failed: %v", err)
		}
	}
}

// HandleSecondInstanceLaunch brings the running window forward when Meron is
// launched again, so a window closed to the tray is found again from the app
// launcher instead of the launch appearing to do nothing.
func (a *App) HandleSecondInstanceLaunch(args []string) {
	urls := mailtoURLs(args)
	if len(urls) == 0 {
		a.showMainWindow()
		return
	}
	for _, raw := range urls {
		a.openMailtoURL(raw)
	}
}

func (a *App) Shutdown(requestContext context.Context) {
	a.logf("shutdown")
	if a.mcp != nil {
		a.mcp.close()
	}
	a.stopTray()
	if a.sidecar != nil {
		a.sidecar.Close()
	}
	a.closeNotificationListener()
	a.closeResumeListener()
	a.closeAppearanceListener()
	if a.logFile != nil {
		_ = a.logFile.Close()
	}
}

func (a *App) appPrefsGet(payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return map[string]any{"prefs": map[string]any{}}, nil
	}
	return a.sidecar.Call("app.prefsGet", payload)
}

func (a *App) appPrefsSet(payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return map[string]any{"ok": true}, nil
	}
	return a.sidecar.Call("app.prefsSet", payload)
}

func (a *App) systemCheck() (any, error) {
	dbPath := filepath.Join(appConfigDirectory(), "meron.db")
	return map[string]any{
		"platform":        runtime.GOOS,
		"tray_monochrome": a.trayState().state().monochrome,
		"mail_engine":     "meron_mail",
		"meron_mail": map[string]any{
			"configured":  coreBinaryPath() != "",
			"available":   fileExists(coreBinaryPath()),
			"server_path": coreBinaryPath(),
		},
		"gmail_oauth_configured":   gmailOAuthConfigured(),
		"outlook_oauth_configured": outlookOAuthConfigured(),
		"database_path":            dbPath,
		"log_path":                 filepath.Join(appConfigDirectory(), "meron.log"),
	}, nil
}
