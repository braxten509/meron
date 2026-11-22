//go:build integration

// Integration scenarios for folders.
// Runs with the integration build tag against the test mail server.
// The main test owns account setup and the order of dependent scenarios.
package main

import (
	"fmt"
	"strings"
	"testing"
	"time"
)

func checkFolders(t *testing.T, sidecar *Sidecar) {
	// folders.list serves the store cache and refreshes in the background,
	// so poll until the IMAP folder sync lands.
	deadline := time.Now().Add(30 * time.Second)
	for {
		result := callMap(t, sidecar, "folders.list", map[string]any{"account": "alice"})
		if foldersContain(result, "INBOX") {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("INBOX never appeared in folders.list: %v", result)
		}
		time.Sleep(300 * time.Millisecond)
	}

	// Not "Archive"/"Sent"/etc — the mail server pre-creates the special folders.
	result := callMap(t, sidecar, "folders.create", map[string]any{"account": "alice", "name": "ITestFolder"})
	if !foldersContain(result, "ITestFolder") {
		t.Fatalf("folders.create did not return ITestFolder: %v", result)
	}
	result = callMap(t, sidecar, "folders.create", map[string]any{"account": "bob", "name": "ITestFolder"})
	if !foldersContain(result, "ITestFolder") {
		t.Fatalf("folders.create for bob did not return ITestFolder: %v", result)
	}

	// Deleting on the server: an ordinary folder goes with its cached mail,
	// while a special-use folder is refused before any IMAP work.
	result = callMap(t, sidecar, "folders.create", map[string]any{"account": "alice", "name": "ITestDeleteMe"})
	if !foldersContain(result, "ITestDeleteMe") {
		t.Fatalf("folders.create did not return ITestDeleteMe: %v", result)
	}
	result = callMap(t, sidecar, "folders.delete", map[string]any{"account": "alice", "folder": "ITestDeleteMe"})
	if foldersContain(result, "ITestDeleteMe") {
		t.Fatalf("folders.delete left ITestDeleteMe in the folder list: %v", result)
	}
	if _, err := sidecar.Call("folders.delete", map[string]any{"account": "alice", "folder": "INBOX"}); err == nil {
		t.Fatal("folders.delete accepted INBOX, want refusal")
	}

	// A real hierarchy is removed leaf-first, and the response names every
	// mailbox that disappeared so clients can clear nested views.
	delimiter := folderDelimiter(result, "INBOX")
	if delimiter == "" {
		t.Fatal("mail server reported no hierarchy delimiter for INBOX")
	}
	parent := "ITestDeleteTree"
	child := parent + delimiter + "Child"
	callMap(t, sidecar, "folders.create", map[string]any{"account": "alice", "name": parent})
	callMap(t, sidecar, "folders.create", map[string]any{"account": "alice", "name": child})
	deadline = time.Now().Add(30 * time.Second)
	for {
		result = callMap(t, sidecar, "folders.list", map[string]any{"account": "alice"})
		if foldersContain(result, parent) &&
			foldersContain(result, child) &&
			folderDelimiter(result, parent) == delimiter {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("nested folders never refreshed with delimiter %q: %v", delimiter, result)
		}
		time.Sleep(300 * time.Millisecond)
	}
	result = callMap(t, sidecar, "folders.delete", map[string]any{"account": "alice", "folder": parent})
	if foldersContain(result, parent) || foldersContain(result, child) {
		t.Fatalf("folders.delete left part of the subtree cached: %v", result)
	}
	removed, _ := result["removed"].([]any)
	if len(removed) != 2 || removed[0] != child || removed[1] != parent {
		t.Fatalf("removed = %#v, want [%q %q]", removed, child, parent)
	}

	// Renaming moves the subtree on the server and in the folder list, and
	// names every folder that moved. Special-use folders are refused.
	parent = "ITestRenameTree"
	child = parent + delimiter + "Child"
	renamedParent := "ITestRenamed"
	renamedChild := renamedParent + delimiter + "Child"
	callMap(t, sidecar, "folders.create", map[string]any{"account": "alice", "name": parent})
	callMap(t, sidecar, "folders.create", map[string]any{"account": "alice", "name": child})
	deadline = time.Now().Add(30 * time.Second)
	for {
		result = callMap(t, sidecar, "folders.list", map[string]any{"account": "alice"})
		if foldersContain(result, child) && folderDelimiter(result, parent) == delimiter {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("nested folders never refreshed with delimiter %q: %v", delimiter, result)
		}
		time.Sleep(300 * time.Millisecond)
	}
	result = callMap(t, sidecar, "folders.rename", map[string]any{"account": "alice", "folder": parent, "name": renamedParent})
	if foldersContain(result, parent) || foldersContain(result, child) ||
		!foldersContain(result, renamedParent) || !foldersContain(result, renamedChild) {
		t.Fatalf("folders.rename did not move the subtree: %v", result)
	}
	if renamed, _ := result["renamed"].([]any); len(renamed) != 2 {
		t.Fatalf("renamed = %#v, want the folder and its child", result["renamed"])
	}
	if _, err := sidecar.Call("folders.rename", map[string]any{"account": "alice", "folder": "INBOX", "name": "Elsewhere"}); err == nil {
		t.Fatal("folders.rename accepted INBOX, want refusal")
	}
	callMap(t, sidecar, "folders.delete", map[string]any{"account": "alice", "folder": renamedParent})
}

func checkArchiveFolderResolves(t *testing.T, sidecar *Sidecar) {
	// folders.archive resolves the account's special-use Archive folder; the
	// move-to-archive action depends on this lookup succeeding.
	result := callMap(t, sidecar, "folders.archive", map[string]any{"account": "bob"})
	if folder := str(result, "folder"); folder == "" {
		t.Fatalf("folders.archive returned no folder: %v", result)
	}
}

func checkExternalExpungePrunesTheLocalRow(t *testing.T, sidecar *Sidecar, server *mailServer, nonce string) {
	// A message another client deletes for good must not linger as a ghost
	// row: the sync compares the server's UID set against the cache and
	// drops what is no longer there.
	expungeSubject := "Meron integration expunge " + nonce
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    expungeSubject,
		"body":       "delete me from another client",
		"message_id": fmt.Sprintf("itest-expunge-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send expunge fixture: %v", err)
	}
	message := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == expungeSubject
	})
	uid := num(message, "uid")
	if uid == 0 {
		t.Fatalf("expunge fixture has no uid: %v", message)
	}

	imapExpunge(t, server.imapPort, "bob@mail.test", testPassword, "INBOX", uid)
	assertNoMessageInFolder(t, sidecar, "bob", "INBOX", func(m map[string]any) bool {
		return str(m, "subject") == expungeSubject
	})
}

func checkExternalFlagChangesReconcileLocally(t *testing.T) {
	server := startMailServer(t)
	sidecar, _ := startSidecar(t)
	connectAccount(t, sidecar, server, "alice", "alice@mail.test")
	connectAccount(t, sidecar, server, "bob", "bob@mail.test")
	nonce := fmt.Sprintf("%d", time.Now().UnixNano())

	// The reverse direction of the server-truth checks above: another client
	// can read or star a message, and the next sync must replace the cached
	// flags and unread count with what IMAP reports.
	flagSubject := "Meron integration external flags " + nonce
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    flagSubject,
		"body":       "change my flags from another client",
		"message_id": fmt.Sprintf("itest-external-flags-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send external-flags fixture: %v", err)
	}
	message := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == flagSubject
	})
	uid := num(message, "uid")
	if uid == 0 {
		t.Fatalf("external-flags fixture has no uid: %v", message)
	}
	if boolValue(message, "seen") || boolValue(message, "starred") {
		t.Fatalf("external-flags fixture did not start unread and unstarred: %v", message)
	}
	page := callMap(t, sidecar, "messages.recent", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"refresh": false,
		"limit":   50,
	})
	unreadBefore, _ := page["folder_unread"].(float64)
	if unreadBefore != 1 {
		t.Fatalf("folder_unread before external flag change = %v, want 1: %v", unreadBefore, page)
	}

	imapSetFlags(t, server.imapPort, "bob@mail.test", testPassword, "INBOX", uid, true, true)
	updated := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == flagSubject && boolValue(m, "seen") && boolValue(m, "starred")
	})
	if num(updated, "uid") != uid {
		t.Fatalf("externally flagged message uid = %d, want %d", num(updated, "uid"), uid)
	}
	page = callMap(t, sidecar, "messages.recent", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"refresh": false,
		"limit":   50,
	})
	if unread, _ := page["folder_unread"].(float64); unread != 0 {
		t.Fatalf("folder_unread after external \\Seen = %v, want 0: %v", unread, page)
	}

	imapSetFlags(t, server.imapPort, "bob@mail.test", testPassword, "INBOX", uid, false, false)
	updated = pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == flagSubject && !boolValue(m, "seen") && !boolValue(m, "starred")
	})
	if num(updated, "uid") != uid {
		t.Fatalf("externally unflagged message uid = %d, want %d", num(updated, "uid"), uid)
	}
	page = callMap(t, sidecar, "messages.recent", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"refresh": false,
		"limit":   50,
	})
	if unread, _ := page["folder_unread"].(float64); unread != 1 {
		t.Fatalf("folder_unread after removing external \\Seen = %v, want 1: %v", unread, page)
	}
}

func checkUidvalidityChangeClearsTheCachedFolder(t *testing.T, sidecar *Sidecar, server *mailServer, nonce string) {
	// A recreated mailbox hands out a new UIDVALIDITY and starts UIDs over
	// from 1, so cached rows describe messages that no longer exist under
	// those UIDs. The sync must drop the folder's rows wholesale instead of
	// merging two generations — the UID-set prune is deliberately skipped
	// when validity changed, so nothing else would clean them up.
	folder := "ITestUidv"
	if result := callMap(t, sidecar, "folders.create", map[string]any{"account": "bob", "name": folder}); !foldersContain(result, folder) {
		t.Fatalf("folders.create did not return %s: %v", folder, result)
	}
	firstSubject := "Meron integration uidv first " + nonce
	secondSubject := "Meron integration uidv second " + nonce
	for i, subj := range []string{firstSubject, secondSubject} {
		imapAppend(t, server.imapPort, "bob@mail.test", testPassword, folder, rawMessage([]string{
			"From: Carol <carol@example.net>",
			"To: bob@mail.test",
			"Subject: " + subj,
			fmt.Sprintf("Message-ID: <itest-uidv-%d-%s@mail.test>", i, nonce),
			"Date: " + time.Now().Format(time.RFC1123Z),
		}, "before the mailbox was recreated"))
	}
	for _, subj := range []string{firstSubject, secondSubject} {
		pollFolder(t, sidecar, "bob", folder, func(m map[string]any) bool {
			return str(m, "subject") == subj
		})
	}
	before := imapUIDValidity(t, server.imapPort, "bob@mail.test", testPassword, folder)

	imapRecreateFolder(t, server.imapPort, "bob@mail.test", testPassword, folder)
	if after := imapUIDValidity(t, server.imapPort, "bob@mail.test", testPassword, folder); after == before {
		t.Skipf("mail server reused UIDVALIDITY %d for the recreated folder; nothing to reconcile", after)
	}

	freshSubject := "Meron integration uidv fresh " + nonce
	imapAppend(t, server.imapPort, "bob@mail.test", testPassword, folder, rawMessage([]string{
		"From: Carol <carol@example.net>",
		"To: bob@mail.test",
		"Subject: " + freshSubject,
		fmt.Sprintf("Message-ID: <itest-uidv-fresh-%s@mail.test>", nonce),
		"Date: " + time.Now().Format(time.RFC1123Z),
	}, "after the mailbox was recreated"))

	pollFolder(t, sidecar, "bob", folder, func(m map[string]any) bool {
		return str(m, "subject") == freshSubject
	})
	// The new message reuses UID 1, so an upsert alone would hide the first
	// stale row. The second one can only be gone if the folder was cleared.
	for _, subj := range []string{firstSubject, secondSubject} {
		assertNoMessageInFolder(t, sidecar, "bob", folder, func(m map[string]any) bool {
			return str(m, "subject") == subj
		})
	}
}

func checkIdlePushesFolderUpdatesWhileWatched(t *testing.T, sidecar *Sidecar, server *mailServer, events *eventLog, nonce string) {
	// IMAP IDLE is per selected mailbox, so kanban starts a watch per visible
	// non-INBOX column. A message another client appends has to reach the
	// store from the push alone — no refresh call — and stop reaching it once
	// the watch is dropped.
	folder := "ITestIdle"
	if result := callMap(t, sidecar, "folders.create", map[string]any{"account": "bob", "name": folder}); !foldersContain(result, folder) {
		t.Fatalf("folders.create did not return %s: %v", folder, result)
	}
	synced := func() int {
		return events.count(func(event sidecarEvent) bool {
			return event.name == "mail.synced" && str(event.detail, "account") == "bob" &&
				strings.EqualFold(str(event.detail, "folder"), folder)
		})
	}
	// Finish the startup catch-up before appending, so it cannot satisfy
	// the event and cache assertions for the subsequent IDLE push.
	baseline := synced()
	callMap(t, sidecar, "watch.start", map[string]any{"account": "bob", "folder": folder})
	deadline := time.Now().Add(60 * time.Second)
	for synced() == baseline {
		if time.Now().After(deadline) {
			t.Fatalf("initial watch catch-up did not sync %s (synced %d, baseline %d)", folder, synced(), baseline)
		}
		time.Sleep(100 * time.Millisecond)
	}
	baseline = synced()
	pushedSubject := "Meron integration idle pushed " + nonce
	imapAppend(t, server.imapPort, "bob@mail.test", testPassword, folder, rawMessage([]string{
		"From: Carol <carol@example.net>",
		"To: bob@mail.test",
		"Subject: " + pushedSubject,
		fmt.Sprintf("Message-ID: <itest-idle-%s@mail.test>", nonce),
		"Date: " + time.Now().Format(time.RFC1123Z),
	}, "pushed over IDLE"))

	// Wait for the expected cache contents as well as a new sync event;
	// refresh:false ensures the watcher alone brings the message into SQLite.
	waitCached := func(subject string, eventBaseline int, timeout time.Duration) {
		t.Helper()
		deadline := time.Now().Add(timeout)
		for {
			cached := callMap(t, sidecar, "messages.recent", map[string]any{
				"account": "bob",
				"folder":  folder,
				"refresh": false,
				"limit":   50,
			})
			eventCount := synced()
			if eventCount > eventBaseline && messagesContainSubject(cached, subject) {
				return
			}
			if time.Now().After(deadline) {
				t.Fatalf("IDLE watch did not cache %q in %s (synced %d, baseline %d): %v", subject, folder, eventCount, eventBaseline, cached)
			}
			time.Sleep(100 * time.Millisecond)
		}
	}
	waitCached(pushedSubject, baseline, 60*time.Second)

	// Let the initial IDLE responses settle, then stop another folder's
	// watcher. The original connection must stay parked, rather than doing
	// another catch-up sync in response to the shared pause signal.
	otherFolder := "ITestIdleOther"
	callMap(t, sidecar, "folders.create", map[string]any{"account": "bob", "name": otherFolder})
	callMap(t, sidecar, "watch.start", map[string]any{"account": "bob", "folder": otherFolder})
	time.Sleep(time.Second)
	baseline = synced()
	if result := callMap(t, sidecar, "watch.stop", map[string]any{"account": "bob", "folder": otherFolder}); !boolValue(result, "stopped") {
		t.Fatalf("watch.stop did not stop the other watcher: %v", result)
	}
	deadline = time.Now().Add(time.Second)
	for time.Now().Before(deadline) {
		if synced() != baseline {
			t.Fatalf("stopping %s restarted the watcher for %s", otherFolder, folder)
		}
		time.Sleep(50 * time.Millisecond)
	}

	stopped := func() int {
		return events.count(func(event sidecarEvent) bool {
			return event.name == "watch.stopped" && str(event.detail, "account") == "bob" &&
				strings.EqualFold(str(event.detail, "folder"), folder)
		})
	}
	stoppedBaseline := stopped()
	if result := callMap(t, sidecar, "watch.stop", map[string]any{"account": "bob", "folder": folder}); !boolValue(result, "stopped") {
		t.Fatalf("watch.stop did not stop the watcher: %v", result)
	}
	deadline = time.Now().Add(10 * time.Second)
	for stopped() == stoppedBaseline {
		if time.Now().After(deadline) {
			t.Fatalf("watch.stop never completed for %s", folder)
		}
		time.Sleep(100 * time.Millisecond)
	}
	baseline = synced()
	quietSubject := "Meron integration idle quiet " + nonce
	imapAppend(t, server.imapPort, "bob@mail.test", testPassword, folder, rawMessage([]string{
		"From: Carol <carol@example.net>",
		"To: bob@mail.test",
		"Subject: " + quietSubject,
		fmt.Sprintf("Message-ID: <itest-idle-quiet-%s@mail.test>", nonce),
		"Date: " + time.Now().Format(time.RFC1123Z),
	}, "appended after the watch stopped"))
	if synced() != baseline {
		t.Fatalf("a stopped watch kept syncing %s", folder)
	}
	cached := callMap(t, sidecar, "messages.recent", map[string]any{
		"account": "bob",
		"folder":  folder,
		"refresh": false,
		"limit":   50,
	})
	if messagesContainSubject(cached, quietSubject) {
		t.Fatalf("a stopped watch still cached %q: %v", quietSubject, cached)
	}

	// Starting the watch again catches up the message that arrived while it
	// was stopped, proving the negative assertion above was not merely an
	// append that had not reached the server yet.
	callMap(t, sidecar, "watch.start", map[string]any{"account": "bob", "folder": folder})
	waitCached(quietSubject, baseline, 30*time.Second)

	// Best-effort teardown only. The stop behavior was asserted above; once
	// the restarted watcher has caught up, waiting for a second lifecycle
	// event adds no coverage and can race the watcher entering IDLE.
	callMap(t, sidecar, "watch.stop", map[string]any{"account": "bob", "folder": folder})
}
