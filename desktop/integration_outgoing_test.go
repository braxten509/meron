//go:build integration

// Integration scenarios for outgoing.
// Runs with the integration build tag against the test mail server.
// The main test owns account setup and the order of dependent scenarios.
package main

import (
	"fmt"
	"testing"
	"time"
)

func checkDraftLifecycle(t *testing.T, sidecar *Sidecar, nonce string) {
	draftID := fmt.Sprintf("itest-draft-%s@mail.test", nonce)
	draftSubject := "Meron integration draft " + nonce
	if _, err := sidecar.Call("save_draft", map[string]any{
		"account":  "alice",
		"to":       "bob@mail.test",
		"subject":  draftSubject,
		"body":     "draft body",
		"draft_id": draftID,
	}); err != nil {
		t.Fatalf("save_draft: %v", err)
	}

	draft := pollFolder(t, sidecar, "alice", "Drafts", func(m map[string]any) bool {
		return str(m, "subject") == draftSubject
	})
	if str(draft, "thread_key") == "" {
		t.Fatalf("draft has empty thread_key: %v", draft)
	}

	if _, err := sidecar.Call("discard_draft", map[string]any{
		"account":  "alice",
		"draft_id": draftID,
	}); err != nil {
		t.Fatalf("discard_draft: %v", err)
	}
	assertNoMessageInFolder(t, sidecar, "alice", "Drafts", func(m map[string]any) bool {
		return str(m, "subject") == draftSubject
	})
}

func checkQuickReplyDraftLifecycle(t *testing.T, sidecar *Sidecar, nonce string) {
	// The quick reply saves a *reply* draft: it must thread into the
	// conversation it answers (that is what the frontend hydrates the box
	// from), and discard_draft with a thread_key must scrub it from the
	// thread so a cleared reply cannot resurface on the next thread open.
	quickSubject := "Meron integration quick reply " + nonce
	quickMessageID := fmt.Sprintf("itest-qr-%s@mail.test", nonce)
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    quickSubject,
		"body":       "please reply inline",
		"message_id": quickMessageID,
	}); err != nil {
		t.Fatalf("send quick reply fixture: %v", err)
	}
	original := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == quickSubject
	})
	threadKey := str(original, "thread_key")
	if threadKey == "" {
		t.Fatalf("quick reply fixture has empty thread_key: %v", original)
	}

	// Same Message-ID shape the frontend mints (newDraftMessageId): the
	// store-side thread cleanup only targets meron-draft-*@meron ids.
	draftID := fmt.Sprintf("meron-draft-%s-itest@meron", nonce)
	draftSubject := "Re: " + quickSubject
	if _, err := sidecar.Call("save_draft", map[string]any{
		"account":     "bob",
		"to":          "alice@mail.test",
		"subject":     draftSubject,
		"body":        "quick reply draft body",
		"in_reply_to": quickMessageID,
		"references":  quickMessageID,
		"draft_id":    draftID,
	}); err != nil {
		t.Fatalf("save_draft: %v", err)
	}

	draft := pollFolder(t, sidecar, "bob", "Drafts", func(m map[string]any) bool {
		return str(m, "subject") == draftSubject
	})
	if got := str(draft, "thread_key"); got != threadKey {
		t.Fatalf("reply draft thread_key = %q, want %q (draft would not hydrate into its thread)", got, threadKey)
	}
	thread := callMap(t, sidecar, "messages.thread", map[string]any{
		"account":    "bob",
		"folder":     "INBOX",
		"thread_key": threadKey,
	})
	if !threadContainsSubject(thread, draftSubject) {
		t.Fatalf("messages.thread does not include the reply draft: %v", thread)
	}

	if _, err := sidecar.Call("discard_draft", map[string]any{
		"account":    "bob",
		"draft_id":   draftID,
		"thread_key": threadKey,
	}); err != nil {
		t.Fatalf("discard_draft: %v", err)
	}
	assertNoMessageInFolder(t, sidecar, "bob", "Drafts", func(m map[string]any) bool {
		return str(m, "subject") == draftSubject
	})
	thread = callMap(t, sidecar, "messages.thread", map[string]any{
		"account":    "bob",
		"folder":     "INBOX",
		"thread_key": threadKey,
	})
	if threadContainsSubject(thread, draftSubject) {
		t.Fatalf("discarded reply draft still in messages.thread: %v", thread)
	}
	if !threadContainsSubject(thread, quickSubject) {
		t.Fatalf("original message missing from thread after draft discard: %v", thread)
	}
}

func checkSentCopyHonorsTheAccountOverride(t *testing.T, sidecar *Sidecar, server *mailServer, nonce string) {
	// A generic IMAP server does not file sent mail itself, so meron APPENDs
	// the copy — exactly once, since a duplicate upload would show the
	// message twice in Sent. The override exists for providers that do file
	// their own, and must suppress the upload entirely.
	defaultSubject := "Meron integration sentcopy default " + nonce
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    defaultSubject,
		"body":       "should be filed in Sent by us",
		"message_id": fmt.Sprintf("itest-sentcopy-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send sentcopy fixture: %v", err)
	}
	pollFolder(t, sidecar, "alice", "Sent", func(m map[string]any) bool {
		return str(m, "subject") == defaultSubject
	})
	if uids := imapSearchSubject(t, server.imapPort, "alice@mail.test", testPassword, "Sent", defaultSubject); len(uids) != 1 {
		t.Fatalf("Sent holds %d copies of the message, want 1: %v", len(uids), uids)
	}

	callMap(t, sidecar, "account.setSaveSentCopy", map[string]any{"account": "alice", "value": false})
	// Back to the provider default, so nothing after this inherits it.
	defer callMap(t, sidecar, "account.setSaveSentCopy", map[string]any{"account": "alice", "value": nil})

	suppressedSubject := "Meron integration sentcopy off " + nonce
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    suppressedSubject,
		"body":       "should not be filed in Sent",
		"message_id": fmt.Sprintf("itest-sentcopy-off-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send suppressed sentcopy fixture: %v", err)
	}
	// Delivery is what proves the send itself went through.
	pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == suppressedSubject
	})
	if uids := imapSearchSubject(t, server.imapPort, "alice@mail.test", testPassword, "Sent", suppressedSubject); len(uids) != 0 {
		t.Fatalf("override off still uploaded a Sent copy: %v", uids)
	}
}

func checkSentFromAnotherClientSurfacesViaInboxSync(t *testing.T, sidecar *Sidecar, server *mailServer, nonce string) {
	// From is a send-as alias meron knows nothing about: the message must
	// still classify as outgoing purely from its Sent-folder provenance.
	externalSubject := "Meron integration external sent " + nonce
	raw := fmt.Sprintf(
		"From: Alice Alias <alice-alias@example.net>\r\nTo: bob@mail.test\r\nSubject: %s\r\n"+
			"Message-ID: <itest-external-%s@mail.test>\r\nDate: %s\r\n\r\n"+
			"sent from another client\r\n",
		externalSubject, nonce, time.Now().Format(time.RFC1123Z))
	imapAppend(t, server.imapPort, "alice@mail.test", testPassword, "Sent", []byte(raw))

	// The copy must reach the local store without Sent ever being opened:
	// an INBOX refresh piggybacks a Sent envelope sync (messages.recent
	// with refresh=false serves the store cache only).
	var row map[string]any
	deadline := time.Now().Add(60 * time.Second)
	for row == nil {
		callMap(t, sidecar, "messages.recent", map[string]any{
			"account": "alice",
			"folder":  "INBOX",
			"refresh": true,
			"limit":   50,
		})
		cached := callMap(t, sidecar, "messages.recent", map[string]any{
			"account": "alice",
			"folder":  "Sent",
			"refresh": false,
			"limit":   50,
		})
		rows, _ := cached["messages"].([]any)
		for _, item := range rows {
			message, ok := item.(map[string]any)
			if ok && str(message, "subject") == externalSubject {
				row = message
				break
			}
		}
		if row == nil && time.Now().After(deadline) {
			t.Fatalf("externally appended Sent message never surfaced via inbox piggyback: %v", cached)
		}
		if row == nil {
			time.Sleep(500 * time.Millisecond)
		}
	}

	thread := callMap(t, sidecar, "messages.thread", map[string]any{
		"account":    "alice",
		"folder":     "Sent",
		"thread_key": str(row, "thread_key"),
		"limit":      50,
	})
	entries, _ := thread["messages"].([]any)
	for _, item := range entries {
		message, ok := item.(map[string]any)
		if !ok {
			continue
		}
		if str(message, "subject") != externalSubject {
			continue
		}
		if !boolValue(message, "outgoing") {
			t.Fatalf("alias-sent Sent copy not classified outgoing: %v", message)
		}
		return
	}
	t.Fatalf("appended message missing from thread read: %v", thread)
}
