//go:build integration

// Integration scenarios for delivery.
// Runs with the integration build tag against the test mail server.
// The main test owns account setup and the order of dependent scenarios.
package main

import (
	"encoding/base64"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func checkSendAndReceive(t *testing.T, sidecar *Sidecar, subject string, messageID string) {
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    subject,
		"body":       "hello from the integration test",
		"message_id": messageID,
	}); err != nil {
		t.Fatalf("send: %v", err)
	}

	message := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == subject
	})
	if from := str(message, "from_addr"); from != "alice@mail.test" {
		t.Errorf("from_addr = %q, want alice@mail.test", from)
	}
	if str(message, "thread_key") == "" {
		t.Error("delivered message has empty thread_key")
	}
}

func checkThreading(t *testing.T, sidecar *Sidecar, subject string, messageID string) {
	// A reply (References/In-Reply-To pointing at the first message) must
	// land in the same thread as the original in bob's mailbox.
	if _, err := sidecar.Call("send", map[string]any{
		"account":     "alice",
		"to":          "bob@mail.test",
		"subject":     "Re: " + subject,
		"body":        "follow-up",
		"in_reply_to": messageID,
		"references":  messageID,
	}); err != nil {
		t.Fatalf("send reply: %v", err)
	}

	reply := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == "Re: "+subject
	})
	original := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == subject
	})
	if str(reply, "thread_key") != str(original, "thread_key") {
		t.Errorf("reply thread_key %q != original thread_key %q",
			str(reply, "thread_key"), str(original, "thread_key"))
	}

	result := callMap(t, sidecar, "messages.thread", map[string]any{
		"account":    "bob",
		"folder":     "INBOX",
		"thread_key": str(original, "thread_key"),
	})
	if n := threadLength(result); n < 2 {
		t.Errorf("messages.thread returned %d messages, want >= 2: %v", n, result)
	}
}

func checkFailedImapWriteDoesNotFalselyMarkReadLocally(t *testing.T, sidecar *Sidecar, server *mailServer, nonce string) {
	// Regression test: messages.markRead/markStarred/markAllRead must not
	// commit the local "seen"/"starred" state when the IMAP STORE itself
	// fails (dropped connection, offline, etc) — otherwise the local store
	// and every unread badge built on it silently drift from server truth.
	flagFailSubject := "Meron integration flagfail " + nonce
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    flagFailSubject,
		"body":       "should stay unread after a failed write",
		"message_id": fmt.Sprintf("itest-flagfail-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send flagfail fixture: %v", err)
	}
	message := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == flagFailSubject
	})
	uid := num(message, "uid")
	if uid == 0 {
		t.Fatalf("delivered flagfail fixture has no uid: %v", message)
	}

	docker := dockerBin(t)
	network := runCmd(t, docker, "inspect", "-f", "{{range $net,$v := .NetworkSettings.Networks}}{{$net}}{{end}}", server.container)
	if network == "" {
		t.Skip("could not determine container network; skipping simulated IMAP outage")
	}

	runCmd(t, docker, "network", "disconnect", network, server.container)
	reconnected := false
	reconnect := func() {
		if reconnected {
			return
		}
		reconnected = true
		if out, err := exec.Command(docker, "network", "connect", network, server.container).CombinedOutput(); err != nil {
			t.Logf("docker network connect %s %s: %v\n%s", network, server.container, err, out)
		}
		waitForIMAPGreeting(t, server.imapPort, docker, server.container)
		// with_write_session doesn't retry a stale pooled connection (only
		// reads do), so both accounts can be left holding a dead socket from
		// the outage above. Pause/resume forces the engine to drop pooled
		// sessions, so later subtests in this shared run don't inherit them.
		for _, account := range []string{"alice", "bob"} {
			callMap(t, sidecar, "account.setPaused", map[string]any{"account": account, "enabled": true})
			callMap(t, sidecar, "account.setPaused", map[string]any{"account": account, "enabled": false})
		}
	}
	defer reconnect()

	if _, err := sidecar.Call("messages.markRead", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"uid":     uid,
		"seen":    true,
	}); err == nil {
		t.Fatalf("messages.markRead succeeded despite a severed IMAP connection")
	}

	reconnect()

	stillUnread := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == flagFailSubject
	})
	if boolValue(stillUnread, "seen") {
		t.Fatalf("message was marked seen locally despite the IMAP write having failed: %v", stillUnread)
	}

	// A retry once connectivity is restored should succeed normally.
	callMap(t, sidecar, "messages.markRead", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"uid":     uid,
		"seen":    true,
	})
	updated := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == flagFailSubject && boolValue(m, "seen")
	})
	if num(updated, "uid") != uid {
		t.Fatalf("flagged message uid = %d, want %d", num(updated, "uid"), uid)
	}
}

func checkSearchAndStarredItems(t *testing.T, sidecar *Sidecar, nonce string) {
	searchSubject := "Meron integration search " + nonce
	searchBody := "unique-search-token-" + nonce
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    searchSubject,
		"body":       searchBody,
		"message_id": fmt.Sprintf("itest-search-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send search fixture: %v", err)
	}

	message := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == searchSubject
	})
	uid := num(message, "uid")
	if uid == 0 {
		t.Fatalf("delivered search fixture has no uid: %v", message)
	}

	search := callMap(t, sidecar, "messages.recent", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"query":   searchBody,
		"refresh": true,
		"limit":   50,
	})
	if !messagesContainSubject(search, searchSubject) {
		t.Fatalf("messages.recent query did not return %q: %v", searchSubject, search)
	}

	callMap(t, sidecar, "messages.markStarred", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"uid":     uid,
		"starred": true,
	})
	starred := callMap(t, sidecar, "starred.items", map[string]any{"limit": 50})
	if !starredMailContainsSubject(starred, searchSubject) {
		t.Fatalf("starred.items did not include %q: %v", searchSubject, starred)
	}
}

func checkAttachments(t *testing.T, sidecar *Sidecar, nonce string) {
	attachmentSubject := "Meron integration attachment " + nonce
	attachmentBody := "attachment body " + nonce
	attachmentBytes := []byte("hello attachment " + nonce)
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    attachmentSubject,
		"body":       attachmentBody,
		"message_id": fmt.Sprintf("itest-attachment-%s@mail.test", nonce),
		"attachments": []map[string]any{{
			"filename":  "itest-note.txt",
			"mime":      "text/plain",
			"data":      base64.StdEncoding.EncodeToString(attachmentBytes),
			"inline_id": "",
		}},
	}); err != nil {
		t.Fatalf("send attachment fixture: %v", err)
	}

	header := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == attachmentSubject
	})
	// BODYSTRUCTURE must index the file before the thread is opened.
	page := callMap(t, sidecar, "messages.recent", map[string]any{
		"account": "bob", "folder": "INBOX", "refresh": false,
		"group": true, "attachments": true, "limit": 50,
	})
	cards, _ := page["threads"].([]any)
	found := false
	for _, row := range cards {
		card, ok := row.(map[string]any)
		if !ok || str(card, "subject") != attachmentSubject {
			continue
		}
		files, _ := card["files"].([]any)
		if len(files) != 1 {
			t.Fatalf("unopened attachment card files = %v, want one file", files)
		}
		file, ok := files[0].(map[string]any)
		if !ok || str(file, "filename") != "itest-note.txt" || str(file, "mime") != "text/plain" {
			t.Fatalf("unopened attachment metadata = %v", files)
		}
		found = true
	}
	if !found {
		t.Fatalf("attachment filter omitted the unopened thread: %v", page)
	}
	threadKey := str(header, "thread_key")
	if threadKey == "" {
		t.Fatalf("attachment fixture has empty thread_key: %v", header)
	}

	thread := callMap(t, sidecar, "messages.thread", map[string]any{
		"account":    "bob",
		"folder":     "INBOX",
		"thread_key": threadKey,
	})
	message := firstThreadMessage(t, thread)
	attachments := attachmentRows(t, message)
	if len(attachments) != 1 {
		t.Fatalf("attachments len = %d, want 1: %v", len(attachments), message)
	}
	attachment := attachments[0]
	if str(attachment, "filename") != "itest-note.txt" {
		t.Fatalf("attachment filename = %q", str(attachment, "filename"))
	}
	if str(attachment, "mime") != "text/plain" {
		t.Fatalf("attachment mime = %q", str(attachment, "mime"))
	}
	if size := num(attachment, "size"); size != uint32(len(attachmentBytes)) {
		t.Fatalf("attachment size = %d, want %d", size, len(attachmentBytes))
	}
	key := str(attachment, "key")
	if key == "" {
		t.Fatalf("attachment key is empty: %v", attachment)
	}
	got, err := os.ReadFile(filepath.Join(mediaDir(), key))
	if err != nil {
		t.Fatalf("read cached attachment %q: %v", key, err)
	}
	if string(got) != string(attachmentBytes) {
		t.Fatalf("cached attachment bytes = %q, want %q", got, attachmentBytes)
	}
}

func checkEmptyTrash(t *testing.T, sidecar *Sidecar, nonce string) {
	// Emptying is folder-wide and permanent — it clears the server folder
	// itself, not just the UIDs the client cached — and role-gated, so a
	// request for INBOX must be refused before anything is touched.
	emptySubject := "Meron integration empty " + nonce
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    emptySubject,
		"body":       "empty me",
		"message_id": fmt.Sprintf("itest-empty-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send empty fixture: %v", err)
	}
	message := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == emptySubject
	})
	uid := num(message, "uid")
	if uid == 0 {
		t.Fatalf("empty fixture has no uid: %v", message)
	}
	callMap(t, sidecar, "messages.delete", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"uid":     uid,
	})
	pollFolder(t, sidecar, "bob", "Trash", func(m map[string]any) bool {
		return str(m, "subject") == emptySubject
	})

	if _, err := sidecar.Call("messages.emptyFolder", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
	}); err == nil {
		t.Fatal("messages.emptyFolder accepted INBOX, want refusal")
	}

	callMap(t, sidecar, "messages.emptyFolder", map[string]any{
		"account": "bob",
		"folder":  "Trash",
	})
	assertNoMessageInFolder(t, sidecar, "bob", "Trash", func(map[string]any) bool { return true })
}

func checkMarkAllRead(t *testing.T, sidecar *Sidecar, nonce string) {
	readSubjectA := "Meron integration markall A " + nonce
	readSubjectB := "Meron integration markall B " + nonce
	for i, subj := range []string{readSubjectA, readSubjectB} {
		if _, err := sidecar.Call("send", map[string]any{
			"account":    "alice",
			"to":         "bob@mail.test",
			"subject":    subj,
			"body":       "unread fixture",
			"message_id": fmt.Sprintf("itest-markall-%d-%s@mail.test", i, nonce),
		}); err != nil {
			t.Fatalf("send markall fixture %d: %v", i, err)
		}
	}
	// Wait for both to land while still unseen.
	for _, subj := range []string{readSubjectA, readSubjectB} {
		pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
			return str(m, "subject") == subj && !boolValue(m, "seen")
		})
	}

	callMap(t, sidecar, "messages.markAllRead", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
	})

	for _, subj := range []string{readSubjectA, readSubjectB} {
		pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
			return str(m, "subject") == subj && boolValue(m, "seen")
		})
	}
}

func checkContactsSuggestFromHistory(t *testing.T, sidecar *Sidecar) {
	// bob's mailbox holds messages from alice, so a prefix query against
	// bob's history must surface alice's address.
	result := callMap(t, sidecar, "contacts.suggest", map[string]any{
		"account": "bob",
		"query":   "alice",
		"limit":   8,
	})
	contacts, _ := result["contacts"].([]any)
	found := false
	for _, item := range contacts {
		contact, ok := item.(map[string]any)
		if ok && strings.Contains(str(contact, "addr"), "alice@mail.test") {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("contacts.suggest(%q) did not surface alice: %v", "alice", result)
	}
}

func checkGroupedThreadCards(t *testing.T, sidecar *Sidecar) {
	// group:true opts into core-side thread grouping: the response carries
	// ready thread cards (thread_id, unread counts) instead of raw message
	// rows.
	res := callMap(t, sidecar, "messages.recent", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"refresh": false,
		"limit":   50,
		"group":   true,
	})
	threads, _ := res["threads"].([]any)
	if len(threads) == 0 {
		t.Fatalf("grouped messages.recent returned no threads: %v", res)
	}
	card, ok := threads[0].(map[string]any)
	if !ok || str(card, "thread_id") == "" {
		t.Fatalf("thread card missing thread_id: %v", threads[0])
	}
	if _, hasMessages := res["messages"]; hasMessages {
		t.Fatalf("grouped response should not also carry raw messages: %v", res)
	}
}
