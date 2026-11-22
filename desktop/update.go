// Checks for desktop releases and tracks download state.
// The updater fetches the manifest and selects the appropriate asset.
// Status changes are sent to the frontend while the update proceeds.

package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	wailsRuntime "github.com/wailsapp/wails/v2/pkg/runtime"
)

// The updater reads a manifest published as a release asset. GitHub serves
// `releases/latest/download/<asset>` as a redirect to the newest non-prerelease
// release, so the URL never changes and there is no API rate limit to hit.
const updateManifestURL = "https://github.com/nonbili/meron/releases/latest/download/latest.json"

// releasesPageURL is where the UI sends users whose channel can't self-update
// (a root-owned install, an unreadable DMG).
const releasesPageURL = "https://github.com/nonbili/meron/releases/latest"

// releaseTagURLPrefix builds the link to one desktop release. The repo also
// publishes `android/v*` tags, so once a known desktop version is in hand the
// exact tag beats the `releases/latest` pointer, which follows whichever
// release was published most recently regardless of platform.
const releaseTagURLPrefix = "https://github.com/nonbili/meron/releases/tag/v"

// releasesURLFor points at the specific release when the check succeeded, and
// falls back to the latest-release page when it didn't.
func releasesURLFor(version string) string {
	if version == "" {
		return releasesPageURL
	}
	return releaseTagURLPrefix + strings.TrimPrefix(version, "v")
}

const (
	updateStateIdle        = "idle"
	updateStateChecking    = "checking"
	updateStateAvailable   = "available"
	updateStateDownloading = "downloading"
	updateStateReady       = "ready"
	updateStateInstalling  = "installing"
	updateStateError       = "error"
)

// maxUpdatePayloadBytes caps a download. The largest asset today is the ~120 MB
// DMG; the ceiling exists so a bad manifest can't fill the user's disk.
const maxUpdatePayloadBytes = 512 << 20

// updateManifest is the shape of latest.json (see scripts/gen-latest-json.sh).
type updateManifest struct {
	Version string `json:"version"`
	PubDate string `json:"pubDate"`
	// Platforms is keyed "<goos>-<goarch>", then by channel kind.
	Platforms map[string]map[string]updateAsset `json:"platforms"`
}

type updateAsset struct {
	URL    string `json:"url"`
	SHA256 string `json:"sha256"`
	Size   int64  `json:"size"`
}

// updater owns the check/download/install state machine. Exactly one update can
// be in flight; the mutex guards every field.
type updater struct {
	app *App

	mu           sync.Mutex
	state        string
	latest       string
	pubDate      string
	errMessage   string
	installError string // Restored helper failure; checks must not erase it.
	downloaded   int64
	total        int64
	asset        updateAsset
	stagedPath   string
	busy         bool
	lastEmit     time.Time
}

func newUpdater(app *App) *updater {
	u := &updater{app: app, state: updateStateIdle}
	if message := pendingUpdateError(); message != "" {
		u.state = updateStateError
		u.errMessage = message
		u.installError = message
		app.logf("update: %s", message)
	}
	return u
}

// updateStatusPayload is what both the update.status command and the
// update.status event carry — the frontend replaces its state wholesale.
type updateStatusPayload struct {
	State          string `json:"state"`
	Channel        string `json:"channel"`
	Managed        bool   `json:"managed"`
	Supported      bool   `json:"supported"`
	CurrentVersion string `json:"currentVersion"`
	LatestVersion  string `json:"latestVersion"`
	PubDate        string `json:"pubDate"`
	Downloaded     int64  `json:"downloaded"`
	Total          int64  `json:"total"`
	Error          string `json:"error"`
	InstallError   string `json:"installError,omitempty"`
	ReleasesURL    string `json:"releasesUrl"`
}

// status builds the payload. Callers must hold u.mu.
func (updaterState *updater) statusLocked() updateStatusPayload {
	channel := updaterState.app.updateChannel()
	return updateStatusPayload{
		State:          updaterState.state,
		Channel:        channel.Kind,
		Managed:        channel.Managed,
		Supported:      channel.SelfUpdatable() && !updaterState.app.updatesDisabled(),
		CurrentVersion: appVersion(),
		LatestVersion:  updaterState.latest,
		PubDate:        updaterState.pubDate,
		Downloaded:     updaterState.downloaded,
		Total:          updaterState.total,
		Error:          updaterState.errMessage,
		InstallError:   updaterState.installError,
		ReleasesURL:    releasesURLFor(updaterState.latest),
	}
}

// emitLocked pushes the current status to the webview. Download progress calls
// this on every chunk, so non-terminal states are throttled; state changes
// always go through (force).
func (updaterState *updater) emitLocked(force bool) {
	if !force && time.Since(updaterState.lastEmit) < 100*time.Millisecond {
		return
	}
	updaterState.lastEmit = time.Now()
	if updaterState.app.ctx != nil {
		wailsRuntime.EventsEmit(updaterState.app.ctx, "update.status", updaterState.statusLocked())
	}
}

func (updaterState *updater) setErrorLocked(operationError error) {
	updaterState.state = updateStateError
	updaterState.errMessage = operationError.Error()
	updaterState.busy = false
	updaterState.emitLocked(true)
}

// ---------------------------------------------------------------- App wiring

func (a *App) updateChannel() updateChannel {
	a.updateChannelOnce.Do(func() { a.updateChannelValue = detectChannel() })
	return a.updateChannelValue
}

// updatesDisabled turns the updater off for `wails dev`, where the running
// binary lives in a build dir and replacing it would be meaningless.
func (a *App) updatesDisabled() bool {
	return a.ctx != nil && wailsRuntime.Environment(a.ctx).BuildType == "dev"
}

func (a *App) ensureUpdater() *updater {
	a.updaterOnce.Do(func() { a.updater = newUpdater(a) })
	return a.updater
}

func (a *App) updateStatus() (any, error) {
	u := a.ensureUpdater()
	u.mu.Lock()
	defer u.mu.Unlock()
	return u.statusLocked(), nil
}

func (a *App) updateCheck() (any, error) {
	return a.ensureUpdater().check()
}

func (a *App) updateDownload() (any, error) {
	return a.ensureUpdater().startDownload()
}

func (a *App) updateInstall() (any, error) {
	return a.ensureUpdater().install()
}

// --------------------------------------------------------------------- Check

func (updaterState *updater) check() (any, error) {
	channel := updaterState.app.updateChannel()
	updaterState.mu.Lock()
	if updaterState.app.updatesDisabled() || !channel.SelfUpdatable() {
		status := updaterState.statusLocked()
		updaterState.mu.Unlock()
		return status, nil
	}
	if updaterState.busy || updaterState.state == updateStateDownloading || updaterState.state == updateStateInstalling {
		status := updaterState.statusLocked()
		updaterState.mu.Unlock()
		return status, nil
	}
	// A staged payload for a still-current version survives a re-check.
	if updaterState.state != updateStateReady {
		updaterState.state = updateStateChecking
		updaterState.errMessage = ""
	}
	updaterState.busy = true
	updaterState.emitLocked(true)
	updaterState.mu.Unlock()

	manifest, operationError := fetchUpdateManifest(updateManifestSource())

	updaterState.mu.Lock()
	defer updaterState.mu.Unlock()
	updaterState.busy = false
	if operationError != nil {
		updaterState.app.logf("update: check failed: %v", operationError)
		if updaterState.state == updateStateReady {
			// A background re-check failing (offline, GitHub hiccup) must not
			// throw away a payload the user already downloaded.
			updaterState.emitLocked(true)
			return updaterState.statusLocked(), nil
		}
		updaterState.setErrorLocked(operationError)
		return updaterState.statusLocked(), nil
	}

	asset, ok := manifest.assetFor(runtime.GOOS, runtime.GOARCH, channel.Kind)
	if !ok {
		updaterState.state = updateStateIdle
		updaterState.latest = manifest.Version
		updaterState.pubDate = manifest.PubDate
		updaterState.errMessage = ""
		updaterState.app.logf("update: manifest %s has no %s asset for %s-%s", manifest.Version, channel.Kind, runtime.GOOS, runtime.GOARCH)
		updaterState.emitLocked(true)
		return updaterState.statusLocked(), nil
	}

	updaterState.latest = manifest.Version
	updaterState.pubDate = manifest.PubDate
	updaterState.errMessage = ""
	if compareVersions(manifest.Version, appVersion()) <= 0 {
		updaterState.state = updateStateIdle
		updaterState.installError = ""
		updaterState.asset = updateAsset{}
		updaterState.discardStagedLocked()
	} else if updaterState.state != updateStateReady || updaterState.asset.SHA256 != asset.SHA256 {
		// A newer release landed while an older one was staged: throw the
		// stale payload away and offer the new one.
		updaterState.discardStagedLocked()
		updaterState.asset = asset
		updaterState.total = asset.Size
		updaterState.downloaded = 0
		updaterState.state = updateStateAvailable
	}
	updaterState.app.logf("update: current=%s latest=%s state=%s channel=%s", appVersion(), updaterState.latest, updaterState.state, channel.Kind)
	updaterState.emitLocked(true)
	return updaterState.statusLocked(), nil
}

// updateManifestSource allows pointing a local build at a test manifest.
func updateManifestSource() string {
	if override := strings.TrimSpace(os.Getenv("MERON_UPDATE_MANIFEST_URL")); override != "" {
		return override
	}
	return updateManifestURL
}

func fetchUpdateManifest(url string) (*updateManifest, error) {
	req, operationError := http.NewRequest(http.MethodGet, url, nil)
	if operationError != nil {
		return nil, operationError
	}
	req.Header.Set("User-Agent", updateUserAgent())
	req.Header.Set("Accept", "application/json")
	client := &http.Client{Timeout: 20 * time.Second}
	res, operationError := client.Do(req)
	if operationError != nil {
		return nil, operationError
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("update manifest: unexpected status %d", res.StatusCode)
	}
	body, operationError := io.ReadAll(io.LimitReader(res.Body, 1<<20))
	if operationError != nil {
		return nil, operationError
	}
	var manifest updateManifest
	if operationError := json.Unmarshal(body, &manifest); operationError != nil {
		return nil, fmt.Errorf("update manifest: %w", operationError)
	}
	if strings.TrimSpace(manifest.Version) == "" {
		return nil, errors.New("update manifest: missing version")
	}
	return &manifest, nil
}

func (manifest *updateManifest) assetFor(goos, goarch, kind string) (updateAsset, bool) {
	assets, ok := manifest.Platforms[goos+"-"+goarch]
	if !ok {
		return updateAsset{}, false
	}
	asset, ok := assets[kind]
	if !ok || asset.URL == "" || asset.Size <= 0 || len(asset.SHA256) != 64 {
		return updateAsset{}, false
	}
	return asset, true
}

func updateUserAgent() string {
	return "Meron/" + appVersion() + " (" + runtime.GOOS + "; " + runtime.GOARCH + ")"
}

// ------------------------------------------------------------------ Download

func (updaterState *updater) startDownload() (any, error) {
	updaterState.mu.Lock()
	defer updaterState.mu.Unlock()
	if updaterState.state == updateStateReady || updaterState.state == updateStateDownloading {
		return updaterState.statusLocked(), nil
	}
	if updaterState.state != updateStateAvailable || updaterState.asset.URL == "" {
		return updaterState.statusLocked(), errors.New("no update available to download")
	}
	asset := updaterState.asset
	updaterState.installError = ""
	updaterState.state = updateStateDownloading
	updaterState.errMessage = ""
	updaterState.downloaded = 0
	updaterState.total = asset.Size
	updaterState.busy = true
	updaterState.emitLocked(true)
	go updaterState.download(asset)
	return updaterState.statusLocked(), nil
}

func (updaterState *updater) download(asset updateAsset) {
	path, operationError := updaterState.downloadTo(asset)

	updaterState.mu.Lock()
	defer updaterState.mu.Unlock()
	updaterState.busy = false
	if operationError != nil {
		updaterState.app.logf("update: download failed: %v", operationError)
		updaterState.setErrorLocked(operationError)
		return
	}
	updaterState.stagedPath = path
	updaterState.state = updateStateReady
	updaterState.downloaded = asset.Size
	updaterState.app.logf("update: staged %s", path)
	updaterState.emitLocked(true)
}

// downloadTo streams the asset to the update cache dir, hashing as it goes, and
// only returns a path once size and digest both match the manifest.
func (updaterState *updater) downloadTo(asset updateAsset) (string, error) {
	directory := updateCacheDir()
	// One payload at a time: wipe anything left over from a previous attempt.
	if operationError := os.RemoveAll(directory); operationError != nil {
		return "", operationError
	}
	if operationError := os.MkdirAll(directory, 0o755); operationError != nil {
		return "", operationError
	}
	if asset.Size > maxUpdatePayloadBytes {
		return "", fmt.Errorf("update payload too large: %d bytes", asset.Size)
	}

	req, operationError := http.NewRequest(http.MethodGet, asset.URL, nil)
	if operationError != nil {
		return "", operationError
	}
	req.Header.Set("User-Agent", updateUserAgent())
	// No overall timeout: a 120 MB DMG on a slow line legitimately takes a
	// while. The transport still bounds connect/TLS/header time.
	client := &http.Client{
		Transport: &http.Transport{
			TLSHandshakeTimeout:   30 * time.Second,
			ResponseHeaderTimeout: 60 * time.Second,
		},
	}
	res, operationError := client.Do(req)
	if operationError != nil {
		return "", operationError
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		return "", fmt.Errorf("update download: unexpected status %d", res.StatusCode)
	}

	path := filepath.Join(directory, updateFileName(asset.URL))
	file, operationError := os.OpenFile(path, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0o755)
	if operationError != nil {
		return "", operationError
	}
	hasher := sha256.New()
	written, copyErr := io.Copy(io.MultiWriter(file, hasher), &progressReader{
		reader:  io.LimitReader(res.Body, asset.Size+1),
		updater: updaterState,
	})
	closeErr := file.Close()
	if copyErr != nil {
		os.RemoveAll(directory)
		return "", copyErr
	}
	if closeErr != nil {
		os.RemoveAll(directory)
		return "", closeErr
	}
	if written != asset.Size {
		os.RemoveAll(directory)
		return "", fmt.Errorf("update download: expected %d bytes, got %d", asset.Size, written)
	}
	if digest := hex.EncodeToString(hasher.Sum(nil)); !strings.EqualFold(digest, asset.SHA256) {
		os.RemoveAll(directory)
		return "", fmt.Errorf("update download: checksum mismatch (expected %s, got %s)", asset.SHA256, digest)
	}
	return path, nil
}

// progressReader reports bytes read back to the updater so the UI can draw a
// progress bar. Emission is throttled inside emitLocked.
type progressReader struct {
	reader  io.Reader
	updater *updater
}

func (progress *progressReader) Read(buffer []byte) (int, error) {
	n, operationError := progress.reader.Read(buffer)
	if n > 0 {
		progress.updater.mu.Lock()
		progress.updater.downloaded += int64(n)
		progress.updater.emitLocked(false)
		progress.updater.mu.Unlock()
	}
	return n, operationError
}

func updateCacheDir() string {
	return filepath.Join(appCacheDir(), "updates")
}

// updateFileName keeps the asset's own extension — the install step dispatches
// on it (.dmg mounts, .tar.gz extracts, .exe runs).
func updateFileName(rawURL string) string {
	parsed, operationError := url.Parse(rawURL)
	if operationError != nil {
		return "meron-update"
	}
	// path.Base, not filepath.Base: URL paths use "/" on every OS. Taking the
	// base also keeps a "../" in a hostile manifest from escaping the cache dir.
	name := path.Base(strings.ReplaceAll(parsed.Path, "\\", "/"))
	if name == "" || name == "." || name == "/" || name == ".." {
		return "meron-update"
	}
	return name
}

// discardStagedLocked drops a staged payload and its cache dir.
func (updaterState *updater) discardStagedLocked() {
	if updaterState.stagedPath == "" {
		return
	}
	updaterState.stagedPath = ""
	_ = os.RemoveAll(updateCacheDir())
}

// cleanupUpdateCache removes leftovers from a previous run. Called on startup:
// a payload that survived a restart has either been installed already or was
// abandoned, and either way it is re-downloadable.
func cleanupUpdateCache() {
	_ = os.RemoveAll(updateCacheDir())
}

// ------------------------------------------------------------- Install shared

// ensureWritableDir checks that we can create files in dir, which is what every
// install strategy needs before it starts moving things around. A root-owned
// /opt or /Applications gives a clear error up front instead of a half-applied
// update.
func ensureWritableDir(directory string) error {
	probe, operationError := os.CreateTemp(directory, ".meron-write-probe-")
	if operationError != nil {
		return operationError
	}
	name := probe.Name()
	_ = probe.Close()
	return os.Remove(name)
}

func copyFile(source, dest string, mode os.FileMode) error {
	in, operationError := os.Open(source)
	if operationError != nil {
		return operationError
	}
	defer in.Close()
	out, operationError := os.OpenFile(dest, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, mode)
	if operationError != nil {
		return operationError
	}
	if _, operationError := io.Copy(out, in); operationError != nil {
		out.Close()
		return operationError
	}
	return out.Close()
}

// ------------------------------------------------------------------- Install

func (updaterState *updater) install() (any, error) {
	updaterState.mu.Lock()
	if updaterState.state != updateStateReady || updaterState.stagedPath == "" {
		status := updaterState.statusLocked()
		updaterState.mu.Unlock()
		return status, errors.New("no downloaded update to install")
	}
	channel := updaterState.app.updateChannel()
	staged := updaterState.stagedPath
	updaterState.installError = ""
	updaterState.state = updateStateInstalling
	updaterState.errMessage = ""
	updaterState.emitLocked(true)
	updaterState.mu.Unlock()

	updaterState.app.logf("update: installing %s via %s", staged, channel.Kind)
	if operationError := applyUpdate(channel, staged); operationError != nil {
		updaterState.app.logf("update: install failed: %v", operationError)
		updaterState.mu.Lock()
		defer updaterState.mu.Unlock()
		// Keep the payload: the user can retry, or open it by hand.
		updaterState.state = updateStateReady
		updaterState.errMessage = operationError.Error()
		updaterState.emitLocked(true)
		return updaterState.statusLocked(), nil
	}

	updaterState.mu.Lock()
	status := updaterState.statusLocked()
	updaterState.mu.Unlock()
	// applyUpdate has queued the relaunch; quitting releases the single-instance
	// lock so the new copy can start.
	updaterState.app.quit()
	return status, nil
}
