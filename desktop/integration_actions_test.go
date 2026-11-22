//go:build integration

// Integration scenarios for actions.
// Runs with the integration build tag against the test mail server.
// The main test owns account setup and the order of dependent scenarios.
package main

import (
	"fmt"
	"strings"
	"testing"
	"time"
)

func checkMoveAndFlags(t *testing.T, sidecar *Sidecar, server *mailServer, nonce string) {
	moveSubject := "Meron integration move " + nonce
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    moveSubject,
		"body":       "move me to the integration folder",
		"message_id": fmt.Sprintf("itest-move-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send move fixture: %v", err)
	}

	message := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == moveSubject
	})
	uid := num(message, "uid")
	if uid == 0 {
		t.Fatalf("delivered move fixture has no uid: %v", message)
	}

	callMap(t, sidecar, "messages.markRead", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"uid":     uid,
		"seen":    true,
	})
	callMap(t, sidecar, "messages.markStarred", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"uid":     uid,
		"starred": true,
	})

	updated := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == moveSubject && boolValue(m, "seen") && boolValue(m, "starred")
	})
	if num(updated, "uid") != uid {
		t.Fatalf("flagged message uid = %d, want %d", num(updated, "uid"), uid)
	}
	// The rows above came out of the same store the flag write updated, so
	// check the server too: a STORE that never left the client would look
	// identical locally.
	if flags := imapFlags(t, server.imapPort, "bob@mail.test", testPassword, "INBOX", uid); !strings.Contains(flags, `\Seen`) || !strings.Contains(flags, `\Flagged`) {
		t.Fatalf("server flags for INBOX uid %d = %q, want \\Seen and \\Flagged", uid, flags)
	}

	result := callMap(t, sidecar, "messages.move", map[string]any{
		"account":       "bob",
		"folder":        "INBOX",
		"target_folder": "ITestFolder",
		"uid":           uid,
	})
	if moved := num(result, "moved"); moved != 1 {
		t.Fatalf("messages.move moved = %d, want 1: %v", moved, result)
	}

	assertNoMessageInFolder(t, sidecar, "bob", "INBOX", func(m map[string]any) bool {
		return str(m, "subject") == moveSubject
	})
	moved := pollFolder(t, sidecar, "bob", "ITestFolder", func(m map[string]any) bool {
		return str(m, "subject") == moveSubject
	})
	if !boolValue(moved, "seen") || !boolValue(moved, "starred") {
		t.Fatalf("moved message lost flags: %v", moved)
	}
	// A MOVE re-APPENDs at the destination, so the flags have to survive the
	// copy on the server, under the new UID.
	movedUID := num(moved, "uid")
	if movedUID == 0 {
		t.Fatalf("moved message has no uid: %v", moved)
	}
	if flags := imapFlags(t, server.imapPort, "bob@mail.test", testPassword, "ITestFolder", movedUID); !strings.Contains(flags, `\Seen`) || !strings.Contains(flags, `\Flagged`) {
		t.Fatalf("server flags for ITestFolder uid %d = %q, want \\Seen and \\Flagged", movedUID, flags)
	}
}

func checkDeleteMovesToTrash(t *testing.T, sidecar *Sidecar, nonce string) {
	// Delete is destructive and folder-aware: a non-draft inbox message must
	// land in Trash (not expunge), so a stray UID never silently vanishes.
	deleteSubject := "Meron integration delete " + nonce
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    deleteSubject,
		"body":       "delete me",
		"message_id": fmt.Sprintf("itest-delete-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send delete fixture: %v", err)
	}
	message := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == deleteSubject
	})
	uid := num(message, "uid")
	if uid == 0 {
		t.Fatalf("delete fixture has no uid: %v", message)
	}

	result := callMap(t, sidecar, "messages.delete", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"uid":     uid,
	})
	if trash, _ := result["trash"].(string); trash == "" {
		t.Fatalf("messages.delete did not report a trash folder: %v", result)
	}

	assertNoMessageInFolder(t, sidecar, "bob", "INBOX", func(m map[string]any) bool {
		return str(m, "subject") == deleteSubject
	})
	pollFolder(t, sidecar, "bob", "Trash", func(m map[string]any) bool {
		return str(m, "subject") == deleteSubject
	})
}

func checkCopyKeepsOriginal(t *testing.T, sidecar *Sidecar, nonce string) {
	// Copy must duplicate, not move: the source UID stays put while a copy
	// appears in the target folder.
	copySubject := "Meron integration copy " + nonce
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    copySubject,
		"body":       "copy me",
		"message_id": fmt.Sprintf("itest-copy-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send copy fixture: %v", err)
	}
	message := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == copySubject
	})
	uid := num(message, "uid")
	if uid == 0 {
		t.Fatalf("copy fixture has no uid: %v", message)
	}

	result := callMap(t, sidecar, "messages.copy", map[string]any{
		"account":        "bob",
		"folder":         "INBOX",
		"target_account": "bob",
		"target_folder":  "ITestFolder",
		"uid":            uid,
	})
	if copied := num(result, "copied"); copied != 1 {
		t.Fatalf("messages.copy copied = %d, want 1: %v", copied, result)
	}

	pollFolder(t, sidecar, "bob", "ITestFolder", func(m map[string]any) bool {
		return str(m, "subject") == copySubject
	})
	// The original must still be in INBOX.
	original := callMap(t, sidecar, "messages.recent", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"refresh": true,
		"limit":   50,
	})
	if !messagesContainSubject(original, copySubject) {
		t.Fatalf("copy removed the original from INBOX: %v", original)
	}
}

func checkEmptyJunk(t *testing.T, sidecar *Sidecar, server *mailServer, nonce string) {
	// emptyFolder is role-gated to Trash *and* Junk (the Trash half and the
	// INBOX refusal are covered above), and the role has to resolve for a
	// folder the account never had special-use metadata for.
	// The mail server pre-creates Junk; folders.list serves the cache and refreshes in
	// the background, so poll until the role-bearing row is there to empty.
	deadline := time.Now().Add(30 * time.Second)
	for {
		result := callMap(t, sidecar, "folders.list", map[string]any{"account": "bob"})
		if foldersContain(result, "Junk") {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("Junk never appeared in folders.list: %v", result)
		}
		time.Sleep(300 * time.Millisecond)
	}
	junkSubject := "Meron integration junk " + nonce
	imapAppend(t, server.imapPort, "bob@mail.test", testPassword, "Junk", rawMessage([]string{
		"From: Spammer <spam@example.net>",
		"To: bob@mail.test",
		"Subject: " + junkSubject,
		fmt.Sprintf("Message-ID: <itest-junk-%s@mail.test>", nonce),
		"Date: " + time.Now().Format(time.RFC1123Z),
	}, "junk body"))
	pollFolder(t, sidecar, "bob", "Junk", func(m map[string]any) bool {
		return str(m, "subject") == junkSubject
	})

	callMap(t, sidecar, "messages.emptyFolder", map[string]any{
		"account": "bob",
		"folder":  "Junk",
	})
	assertNoMessageInFolder(t, sidecar, "bob", "Junk", func(map[string]any) bool { return true })
	if uids := imapSearchSubject(t, server.imapPort, "bob@mail.test", testPassword, "Junk", junkSubject); len(uids) != 0 {
		t.Fatalf("emptying Junk left %v on the server", uids)
	}
}
