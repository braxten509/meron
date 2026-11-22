// Export of a single email as an EML file from the desktop host.
// Requests resolve the message location and choose a filename from its subject.
// The host retrieves the source and writes the selected destination.

package main

import (
	"errors"
	wailsRuntime "github.com/wailsapp/wails/v2/pkg/runtime"
	"strings"
	"unicode/utf8"
)

// emlFilename turns a subject into a safe ".eml" default filename for the save
// dialog; an empty or unusable subject falls back to "message".
func emlFilename(subject string) string {
	return safeFilenameBase(subject, "message") + ".eml"
}

// safeFilenameBase turns free text into a safe filename without its extension.
// Path separators, control characters and the Windows-reserved set are dropped
// so the name can't escape the chosen directory or be rejected by the platform
// dialog; an empty or unusable name becomes fallback.
func safeFilenameBase(name, fallback string) string {
	cleaned := strings.Map(func(r rune) rune {
		switch {
		case r < 0x20 || r == 0x7f:
			return -1
		case strings.ContainsRune(`/\:*?"<>|`, r):
			return -1
		}
		return r
	}, name)
	cleaned = strings.Trim(cleaned, " .")
	if cleaned == "" {
		cleaned = fallback
	}
	// Leave room for the extension inside the common 255-byte name limit.
	if len(cleaned) > 120 {
		end := 120
		for end > 0 && !utf8.RuneStart(cleaned[end]) {
			end--
		}
		cleaned = strings.TrimSpace(cleaned[:end])
	}
	// Windows reserves these device basenames even when an extension follows.
	base, _, _ := strings.Cut(cleaned, ".")
	upper := strings.ToUpper(base)
	if upper == "CON" || upper == "PRN" || upper == "AUX" || upper == "NUL" ||
		(len(upper) == 4 && (strings.HasPrefix(upper, "COM") || strings.HasPrefix(upper, "LPT")) &&
			upper[3] >= '1' && upper[3] <= '9') {
		cleaned = "_" + cleaned
	}
	return cleaned
}

func messageEmlSource(payload map[string]any) (map[string]any, error) {
	threadID, _ := payload["thread_id"].(string)
	if threadID == "" {
		return nil, errors.New("missing thread id")
	}
	if _, _, ok := parseRSSThreadID(threadID); ok {
		return nil, errors.New("feed items have no original message to save")
	}
	ids, ok := parseImapThreadID(threadID)
	if !ok {
		return nil, errors.New("invalid thread id")
	}
	uid := ids.UID
	if uids := imapUIDsFromPayload(threadID, payload); len(uids) > 0 {
		uid = uids[0]
	}
	if uid == 0 {
		return nil, errors.New("missing message uid")
	}
	return map[string]any{
		"account": ids.Account,
		"folder":  deleteFolder(payload, ids.Folder),
		"uid":     uid,
	}, nil
}

// saveMessageEml exports one message's original RFC822 bytes to a user-chosen
// path via the native save dialog. The sidecar writes directly to that path so
// large messages never cross the size-bounded JSON protocol. Raw bytes aren't
// cached locally, so this always needs the sidecar (and the network).
func (a *App) saveMessageEml(payload map[string]any) (any, error) {
	params, err := messageEmlSource(payload)
	if err != nil {
		return nil, err
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}

	subject, _ := payload["subject"].(string)
	dest, err := saveFileDialog(a.ctx, wailsRuntime.SaveDialogOptions{
		Title:                "Save message",
		DefaultFilename:      emlFilename(subject),
		CanCreateDirectories: true,
	})
	if err != nil {
		return nil, err
	}
	if dest == "" {
		return map[string]any{"saved": false}, nil // user cancelled
	}
	params["path"] = dest
	if _, err := a.sidecar.Call("messages.saveRaw", params); err != nil {
		return nil, err
	}
	return map[string]any{"saved": true, "path": dest}, nil
}
