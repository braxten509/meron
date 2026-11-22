// Handles the desktop composer's native file and image pickers.
// The host remembers the last usable directory for subsequent selections.
// Opening an attachment uses a sanitized file name in the app cache.

package main

import (
	"context"
	"encoding/base64"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"

	wailsRuntime "github.com/wailsapp/wails/v2/pkg/runtime"
)

func (a *App) pickImageFile(payload map[string]any) (any, error) {
	res, err := a.pickFiles(payload, true, false)
	if err != nil {
		return nil, err
	}
	resMap, _ := res.(map[string]any)
	if cancelled, _ := resMap["cancelled"].(bool); cancelled {
		return resMap, nil
	}
	files, _ := resMap["files"].([]any)
	if len(files) == 0 {
		return map[string]any{"cancelled": true}, nil
	}
	return files[0], nil
}

func (a *App) pickFiles(payload map[string]any, imagesOnly, multiple bool) (any, error) {
	title, _ := payload["title"].(string)
	if title == "" {
		if imagesOnly {
			title = "Choose image"
		} else {
			title = "Choose files"
		}
	}

	a.filePickerMu.Lock()
	defaultDirectory := a.lastFileDir
	lastDirPath := lastFileDirectoryPath()
	if imagesOnly {
		defaultDirectory = a.lastImageDir
		lastDirPath = lastImageDirectoryPath()
	}
	if defaultDirectory == "" {
		if data, err := os.ReadFile(lastDirPath); err == nil {
			defaultDirectory = strings.TrimSpace(string(data))
			if imagesOnly {
				a.lastImageDir = defaultDirectory
			} else {
				a.lastFileDir = defaultDirectory
			}
		}
	}
	a.filePickerMu.Unlock()

	defaultDirectory = filePickerDefaultDirectory(defaultDirectory)

	paths, usedPortal, err := openFilePicker(a.ctx, title, defaultDirectory, imagesOnly, multiple)
	if err != nil {
		return nil, err
	}
	if len(paths) == 0 {
		return map[string]any{"cancelled": true}, nil
	}

	files := make([]any, 0, len(paths))
	for _, path := range paths {
		file, err := readPickedFile(path)
		if err != nil {
			return nil, err
		}
		if mime, _ := file["mime"].(string); imagesOnly && !strings.HasPrefix(mime, "image/") {
			return nil, errors.New("selected file is not an image")
		}
		files = append(files, file)
	}

	if !usedPortal {
		a.filePickerMu.Lock()
		nextDir := filepath.Dir(paths[0])
		if imagesOnly {
			a.lastImageDir = nextDir
		} else {
			a.lastFileDir = nextDir
		}
		if err := os.MkdirAll(filepath.Dir(lastDirPath), 0o755); err == nil {
			_ = os.WriteFile(lastDirPath, []byte(nextDir), 0o600)
		}
		a.filePickerMu.Unlock()
	}

	return map[string]any{"files": files}, nil
}

func openWailsFilePicker(
	requestContext context.Context,
	title string,
	defaultDirectory string,
	imagesOnly bool,
	multiple bool,
) ([]string, error) {
	options := wailsRuntime.OpenDialogOptions{
		Title:            title,
		DefaultDirectory: defaultDirectory,
	}
	if imagesOnly {
		options.Filters = []wailsRuntime.FileFilter{
			{DisplayName: "Image files (*.png, *.jpg, *.jpeg, *.webp, *.gif)", Pattern: "*.png;*.jpg;*.jpeg;*.webp;*.gif"},
		}
	}

	if multiple {
		return wailsRuntime.OpenMultipleFilesDialog(requestContext, options)
	}
	path, err := wailsRuntime.OpenFileDialog(requestContext, options)
	if err != nil || path == "" {
		return nil, err
	}
	return []string{path}, nil
}

// openComposerAttachment writes an in-memory composer attachment to the app
// cache and asks the OS to open it with the user's default handler.
func (a *App) openComposerAttachment(payload map[string]any) (any, error) {
	filename, _ := payload["filename"].(string)
	data, _ := payload["data"].(string)
	if data == "" {
		return nil, errors.New("missing attachment data")
	}
	if filename == "" {
		filename = "attachment"
	}
	if i := strings.Index(data, ","); strings.HasPrefix(data, "data:") && i >= 0 {
		data = data[i+1:]
	}
	bytes, err := base64.StdEncoding.DecodeString(data)
	if err != nil {
		return nil, fmt.Errorf("decode attachment: %w", err)
	}

	dir := filepath.Join(appCacheDirectory(), "composer-open")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return nil, err
	}
	path := filepath.Join(dir, randomBase64URL(8)+"-"+safeFilename(filename))
	if err := os.WriteFile(path, bytes, 0o600); err != nil {
		return nil, fmt.Errorf("write attachment: %w", err)
	}
	if err := openSystemFile(path); err != nil {
		return nil, err
	}
	return map[string]any{"opened": true, "path": path}, nil
}

func safeFilename(name string) string {
	name = filepath.Base(strings.TrimSpace(name))
	if name == "." || name == string(os.PathSeparator) || name == "" {
		return "attachment"
	}
	replacer := strings.NewReplacer("/", "_", "\\", "_", "\x00", "_", ":", "_")
	return replacer.Replace(name)
}

func openSystemFile(path string) error {
	// macOS goes through NSWorkspace rather than `open`, because the App Store
	// build is sandboxed and the sandbox denies exec'ing it.
	if runtime.GOOS == "darwin" {
		return openSystemFileDarwin(path)
	}
	var cmd *exec.Cmd
	switch runtime.GOOS {
	case "windows":
		cmd = exec.Command("rundll32", "url.dll,FileProtocolHandler", path)
	default:
		cmd = exec.Command("xdg-open", path)
	}
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("open attachment: %w", err)
	}
	_ = cmd.Process.Release()
	return nil
}

func lastImageDirectoryPath() string {
	return filepath.Join(appConfigDirectory(), "last-image-dir")
}

func lastFileDirectoryPath() string {
	return filepath.Join(appConfigDirectory(), "last-file-dir")
}

func filePickerDefaultDirectory(candidate string) string {
	if dir, ok := usablePickerDirectory(candidate); ok {
		return dir
	}
	home, err := os.UserHomeDir()
	if err == nil {
		if dir, ok := usablePickerDirectory(home); ok {
			return dir
		}
	}
	return ""
}

func usablePickerDirectory(path string) (string, bool) {
	if path == "" {
		return "", false
	}
	info, err := os.Stat(path)
	if err != nil || !info.IsDir() {
		return "", false
	}
	realPath, err := filepath.EvalSymlinks(path)
	if err == nil && realPath != "" {
		return realPath, true
	}
	return path, true
}
