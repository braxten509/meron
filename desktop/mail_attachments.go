// Files opened or saved from messages in the desktop reader.
// The host resolves cached media, handles the native file picker and copies attachments.
// Temporary copies opened by external applications have a bounded lifetime.

package main

import (
	"encoding/base64"
	"errors"
	"fmt"
	wailsRuntime "github.com/wailsapp/wails/v2/pkg/runtime"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// mediaFilePath resolves an attachment key the sidecar wrote under the media directory
// (served at /media/<key>) to an absolute path. The key is path-cleaned and
// confined to the media root to block traversal.
func mediaFilePath(key string) (string, error) {
	if key == "" {
		return "", errors.New("missing attachment key")
	}
	root := mediaDirectory()
	// Cleaning a rooted "/"+key strips any "..", then Join confines it to root.
	source := filepath.Join(root, filepath.Clean("/"+key))
	if rel, err := filepath.Rel(root, source); err != nil || rel == ".." || strings.HasPrefix(rel, ".."+string(os.PathSeparator)) {
		return "", errors.New("invalid attachment key")
	}
	return source, nil
}

// saveAttachment copies an attachment the sidecar already wrote under the media
// directory to a user-chosen path via a native save dialog.
func (a *App) saveAttachment(payload map[string]any) (any, error) {
	key, _ := payload["key"].(string)
	filename, _ := payload["filename"].(string)
	source, err := mediaFilePath(key)
	if err != nil {
		return nil, err
	}
	if filename == "" {
		filename = filepath.Base(source)
	}

	destination, err := saveFileDialog(a.ctx, wailsRuntime.SaveDialogOptions{
		Title:                "Save attachment",
		DefaultFilename:      filename,
		CanCreateDirectories: true,
	})
	if err != nil {
		return nil, err
	}
	if destination == "" {
		return map[string]any{"saved": false}, nil // user cancelled
	}

	if err := copyFileTo(source, destination, 0o666); err != nil {
		return nil, err
	}
	return map[string]any{"saved": true, "path": destination}, nil
}

// openableAttachmentExts controls which files can open in an external application.
//
// Other extensions use the save dialog. Modern Office formats with distinct
// macro-enabled extensions are eligible. Older Office and OpenDocument formats
// can carry macros without a distinguishing extension, so they require saving.
var openableAttachmentExts = map[string]bool{
	// Documents
	".pdf": true, ".txt": true, ".md": true, ".csv": true, ".log": true,
	".json": true, ".xml": true, ".ics": true, ".vcf": true,
	".docx": true, ".xlsx": true, ".pptx": true,
	// Images
	".png": true, ".jpg": true, ".jpeg": true, ".gif": true, ".webp": true,
	".bmp": true, ".tif": true, ".tiff": true, ".heic": true, ".avif": true,
	// Audio / video
	".mp3": true, ".m4a": true, ".wav": true, ".flac": true, ".ogg": true,
	".opus": true, ".aac": true,
	".mp4": true, ".m4v": true, ".mov": true, ".webm": true, ".mkv": true,
}

// attachmentOpenable reports whether a name's extension is on the click-to-open
// allowlist. Only the final extension counts, so "invoice.pdf.exe" is judged as
// .exe.
func attachmentOpenable(name string) bool {
	return openableAttachmentExts[strings.ToLower(filepath.Ext(name))]
}

const attachmentOpenDirectoryName = "attachment-open"

// attachmentOpenTTL is how long a copy handed to an external viewer is kept.
// The viewer can still hold the file open long after the click, so the sweep
// runs on the next open and only on stale entries.
const attachmentOpenTTL = 24 * time.Hour

// openAttachment hands an allowed attachment to its default application.
//
// The viewer receives a fresh copy with the original display name. A disallowed
// extension returns opened=false so the caller can show the save dialog.
func (a *App) openAttachment(payload map[string]any) (any, error) {
	key, _ := payload["key"].(string)
	filename, _ := payload["filename"].(string)
	source, err := mediaFilePath(key)
	if err != nil {
		return nil, err
	}
	if filename == "" {
		filename = filepath.Base(source)
	}
	// Both names matter: the key's extension is the one the sidecar derived
	// from the original filename, and the copy's is what picks the handler.
	name := safeFilename(filename)
	if !attachmentOpenable(source) || !attachmentOpenable(name) {
		return map[string]any{"opened": false}, nil
	}

	root := filepath.Join(appCacheDirectory(), attachmentOpenDirectoryName)
	pruneAttachmentOpenDirectory(root)
	directory := filepath.Join(root, randomBase64URL(8))
	if err := os.MkdirAll(directory, 0o700); err != nil {
		return nil, err
	}
	destination := filepath.Join(directory, name)
	if err := copyFileTo(source, destination, 0o600); err != nil {
		return nil, err
	}
	if err := openSystemFile(destination); err != nil {
		return nil, err
	}
	return map[string]any{"opened": true, "path": destination}, nil
}

// pruneAttachmentOpenDirectory removes copies older than attachmentOpenTTL.
func pruneAttachmentOpenDirectory(directory string) {
	entries, err := os.ReadDir(directory)
	if err != nil {
		return
	}
	cutoff := time.Now().Add(-attachmentOpenTTL)
	for _, entry := range entries {
		info, err := entry.Info()
		if err != nil || info.ModTime().After(cutoff) {
			continue
		}
		_ = os.RemoveAll(filepath.Join(directory, entry.Name()))
	}
}

// copyFileTo copies source over destination, creating destination with the given permissions.
func copyFileTo(source, destination string, permissions os.FileMode) error {
	in, err := os.Open(source)
	if err != nil {
		return fmt.Errorf("open attachment: %w", err)
	}
	defer in.Close()
	out, err := os.OpenFile(destination, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, permissions)
	if err != nil {
		return fmt.Errorf("create destination: %w", err)
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		return fmt.Errorf("write attachment: %w", err)
	}
	if err := out.Close(); err != nil {
		return fmt.Errorf("write attachment: %w", err)
	}
	return nil
}

// readAttachment reads an attachment the sidecar already wrote under the media
// directory and returns it base64-encoded, for pulling a stored attachment back into
// the composer (e.g. "Edit as New Message").
func (a *App) readAttachment(payload map[string]any) (any, error) {
	key, _ := payload["key"].(string)
	source, err := mediaFilePath(key)
	if err != nil {
		return nil, err
	}

	data, err := os.ReadFile(source)
	if err != nil {
		return nil, fmt.Errorf("read attachment: %w", err)
	}
	mime := http.DetectContentType(data)
	return map[string]any{
		"data": base64.StdEncoding.EncodeToString(data),
		"mime": mime,
		"size": len(data),
	}, nil
}
