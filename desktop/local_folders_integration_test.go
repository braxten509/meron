//go:build integration

// Exercises local folders through desktop app commands.
// The fixture connects local account operations to the test core.
// It follows the mail through the local folder lifecycle.

package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// Local folders end to end: mail moved off a real IMAP server into the archive
// is gone from the server folder (its copy in Trash), reads, flags and searches
// from the archive, and goes back to the server intact.
func TestIntegrationLocalFolders(t *testing.T) {
	server := startMailServer(t)
	sidecar, _ := startSidecar(t)
	connectAccount(t, sidecar, server, "alice", "alice@mail.test")

	nonce := fmt.Sprintf("%d", time.Now().UnixNano())
	subject := "Local archive " + nonce
	body := "kept on this computer " + nonce
	imapAppend(t, server.imapPort, "alice@mail.test", testPassword, "INBOX", rawMessage([]string{
		"From: Bob <bob@mail.test>",
		"To: alice@mail.test",
		"Subject: " + subject,
		"Message-ID: <local-" + nonce + "@mail.test>",
		"Date: Tue, 1 Oct 2024 10:00:00 +0000",
		"MIME-Version: 1.0",
		`Content-Type: multipart/mixed; boundary="b"`,
	}, "--b\nContent-Type: text/plain\n\n"+body+"\n--b\nContent-Type: text/plain\nContent-Disposition: attachment; filename=\"notes.txt\"\n\nattached\n--b--"))
	source := pollInbox(t, sidecar, "alice", func(message map[string]any) bool {
		return stringField(message, "subject") == subject
	})
	uid := numericField(source, "uid")

	callMap(t, sidecar, "account.addLocal", map[string]any{"name": "Local Folders", "folder": "Archive"})
	if !foldersContain(callMap(t, sidecar, "folders.list", map[string]any{"account": localAccountID}), "Archive") {
		t.Fatal("new local folders have no default folder")
	}
	// Adding again creates no second default folder.
	callMap(t, sidecar, "account.addLocal", map[string]any{"name": "Local Folders", "folder": "Other"})
	if foldersContain(callMap(t, sidecar, "folders.list", map[string]any{"account": localAccountID}), "Other") {
		t.Fatal("re-adding local folders created another default folder")
	}
	accounts := callMap(t, sidecar, "account.list", map[string]any{})
	if !strings.Contains(fmt.Sprint(accounts["accounts"]), "auth_type:local") {
		t.Fatalf("account.list has no local account: %v", accounts)
	}
	created := callMap(t, sidecar, "folders.create", map[string]any{"account": localAccountID, "name": "Old/2024"})
	if !foldersContain(created, "Old/2024") {
		t.Fatalf("folders.create did not return Old/2024: %v", created)
	}

	t.Run("move off the server", func(t *testing.T) {
		result := callMap(t, sidecar, "messages.move", map[string]any{
			"account":        "alice",
			"folder":         "INBOX",
			"uid":            uid,
			"target_account": localAccountID,
			"target_folder":  "Old/2024",
		})
		if moved := jsonNumber(result["moved"]); moved != 1 {
			t.Fatalf("moved = %v, want 1: %v", result["moved"], result)
		}
		if uids := imapSearchSubject(t, server.imapPort, "alice@mail.test", testPassword, "INBOX", subject); len(uids) != 0 {
			t.Fatalf("message still in the server INBOX: %v", uids)
		}
		if uids := imapSearchSubject(t, server.imapPort, "alice@mail.test", testPassword, "Trash", subject); len(uids) != 1 {
			t.Fatalf("server copy not in Trash: %v", uids)
		}
	})

	var localUID uint32
	t.Run("read from the archive", func(t *testing.T) {
		message := pollFolder(t, sidecar, localAccountID, "Old/2024", func(message map[string]any) bool {
			return stringField(message, "subject") == subject
		})
		localUID = numericField(message, "uid")
		thread := callMap(t, sidecar, "messages.thread", map[string]any{
			"account":    localAccountID,
			"folder":     "Old/2024",
			"thread_key": stringField(message, "thread_key"),
			"limit":      20,
		})
		first := firstThreadMessage(t, thread)
		if !strings.Contains(stringField(first, "body"), body) {
			t.Fatalf("archived body missing: %v", first)
		}
		if rows := attachmentRows(t, first); len(rows) != 1 {
			t.Fatalf("attachments = %v, want notes.txt", rows)
		}
		search := callMap(t, sidecar, "messages.recent", map[string]any{
			"account": localAccountID,
			"folder":  "Old/2024",
			"query":   nonce,
			"limit":   20,
		})
		if !messagesContainSubject(search, subject) {
			t.Fatalf("search in the local folder missed the message: %v", search)
		}
	})

	t.Run("flags stay local", func(t *testing.T) {
		callMap(t, sidecar, "messages.markStarred", map[string]any{
			"account": localAccountID,
			"folder":  "Old/2024",
			"uid":     localUID,
			"starred": true,
		})
		message := pollFolder(t, sidecar, localAccountID, "Old/2024", func(message map[string]any) bool {
			return stringField(message, "subject") == subject && boolValue(message, "starred")
		})
		if starred := callMap(t, sidecar, "starred.items", map[string]any{}); !starredMailContainsSubject(starred, subject) {
			t.Fatalf("archived star missing from the Starred view: %v", starred)
		}
		if !boolValue(message, "seen") {
			// The source was unread; marking it read must work too.
			callMap(t, sidecar, "messages.markRead", map[string]any{
				"account": localAccountID,
				"folder":  "Old/2024",
				"uid":     localUID,
				"seen":    true,
			})
		}
	})

	t.Run("rename keeps the mail", func(t *testing.T) {
		renamed := callMap(t, sidecar, "folders.rename", map[string]any{"account": localAccountID, "folder": "Old", "name": "Older"})
		if foldersContain(renamed, "Old/2024") || !foldersContain(renamed, "Older/2024") {
			t.Fatalf("folders.rename did not move the subtree: %v", renamed)
		}
		message := pollFolder(t, sidecar, localAccountID, "Older/2024", func(message map[string]any) bool {
			return stringField(message, "subject") == subject && boolValue(message, "starred")
		})
		thread := callMap(t, sidecar, "messages.thread", map[string]any{
			"account":    localAccountID,
			"folder":     "Older/2024",
			"thread_key": stringField(message, "thread_key"),
			"limit":      20,
		})
		if rows := attachmentRows(t, firstThreadMessage(t, thread)); len(rows) != 1 {
			t.Fatalf("attachments after rename = %v, want notes.txt", rows)
		}
		// Back, so the steps below find it where they expect.
		callMap(t, sidecar, "folders.rename", map[string]any{"account": localAccountID, "folder": "Older", "name": "Old"})
		localUID = numericField(pollFolder(t, sidecar, localAccountID, "Old/2024", func(message map[string]any) bool {
			return stringField(message, "subject") == subject
		}), "uid")
	})

	t.Run("move back to the server", func(t *testing.T) {
		result := callMap(t, sidecar, "messages.move", map[string]any{
			"account":        localAccountID,
			"folder":         "Old/2024",
			"uid":            localUID,
			"target_account": "alice",
			"target_folder":  "INBOX",
		})
		if moved := jsonNumber(result["moved"]); moved != 1 {
			t.Fatalf("moved = %v, want 1: %v", result["moved"], result)
		}
		uids := imapSearchSubject(t, server.imapPort, "alice@mail.test", testPassword, "INBOX", subject)
		if len(uids) != 1 {
			t.Fatalf("message not back in INBOX: %v", uids)
		}
		flags := imapFlags(t, server.imapPort, "alice@mail.test", testPassword, "INBOX", uids[0])
		if !strings.Contains(flags, `\Flagged`) || !strings.Contains(flags, `\Seen`) {
			t.Fatalf("flags = %q, want the local star and read state", flags)
		}
		assertNoMessageInFolder(t, sidecar, localAccountID, "Old/2024", func(message map[string]any) bool {
			return stringField(message, "subject") == subject
		})
	})

	t.Run("back up", func(t *testing.T) {
		dest := filepath.Join(t.TempDir(), "local.db")
		callMap(t, sidecar, "account.localBackup", map[string]any{"path": dest, "passphrase": "itest"})
		if info, err := os.Stat(dest); err != nil || info.Size() == 0 {
			t.Fatalf("snapshot not written: %v", err)
		}
		if _, err := sidecar.Call("account.localRestore", map[string]any{"path": dest, "passphrase": "nope"}); err == nil ||
			!strings.Contains(err.Error(), wrongLocalBackupPassphrase) {
			t.Fatalf("a wrong passphrase was not reported as one: %v", err)
		}
		restored := callMap(t, sidecar, "account.localRestore", map[string]any{"path": dest, "passphrase": "itest"})
		if jsonNumber(restored["restored"]) != 0 {
			t.Fatalf("restoring into the same folders added duplicates: %v", restored)
		}
	})

	t.Run("remove local folders", func(t *testing.T) {
		deleted := callMap(t, sidecar, "folders.delete", map[string]any{"account": localAccountID, "folder": "Old"})
		if foldersContain(deleted, "Old") || foldersContain(deleted, "Old/2024") {
			t.Fatalf("folders.delete left the subtree: %v", deleted)
		}
		callMap(t, sidecar, "account.remove", map[string]any{"id": localAccountID})
		archive := filepath.Join(appConfigDirectory(), "archive.db")
		if _, err := os.Stat(archive); !os.IsNotExist(err) {
			t.Fatalf("archive.db still present after removing local folders: %v", err)
		}
	})
}
