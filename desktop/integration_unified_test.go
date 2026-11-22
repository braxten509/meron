//go:build integration

// Integration scenarios for unified.
// Runs with the integration build tag against the test mail server.
// The main test owns account setup and the order of dependent scenarios.
package main

import (
	"fmt"
	"strings"
	"testing"
	"time"
)

func checkUnifiedInboxMergesAndWritesAcrossAccounts(t *testing.T, sidecar *Sidecar, nonce string, subject string) {
	// The unified view fans messages.recent out over every included account
	// and merges the pages by date; markAllReadUnified fans the write out the
	// same way. Both must cover *both* accounts and report no per-account
	// failure — a partial merge looks like a working inbox with mail missing.
	unifiedSubject := "Meron integration unified " + nonce
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "bob",
		"to":         "alice@mail.test",
		"subject":    unifiedSubject,
		"body":       "so alice has inbox mail too",
		"message_id": fmt.Sprintf("itest-unified-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send unified fixture: %v", err)
	}
	pollInbox(t, sidecar, "alice", func(m map[string]any) bool {
		return str(m, "subject") == unifiedSubject
	})
	pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == subject
	})

	// refresh:false: both stores were just primed, and the merge is the thing
	// under test, not another round of IMAP sync.
	merged := callMap(t, sidecar, "messages.unifiedRecent", map[string]any{
		"limit":   50,
		"refresh": false,
	})
	if failures, _ := merged["failures"].([]any); len(failures) > 0 {
		t.Fatalf("unifiedRecent reported account failures: %v", failures)
	}
	threads, _ := merged["threads"].([]any)
	if len(threads) == 0 {
		t.Fatalf("unifiedRecent returned no threads: %v", merged)
	}
	seen := map[string]bool{}
	lastDate := int64(-1)
	for _, item := range threads {
		card, ok := item.(map[string]any)
		if !ok {
			t.Fatalf("unified thread has type %T", item)
		}
		seen[str(card, "account_id")] = true
		date, _ := card["date"].(float64)
		if lastDate >= 0 && int64(date) > lastDate {
			t.Fatalf("unified page is not sorted newest first: %v", threads)
		}
		lastDate = int64(date)
	}
	for _, account := range []string{"alice", "bob"} {
		if !seen[account] {
			t.Fatalf("unified page has no cards from %s: %v", account, seen)
		}
	}

	result := callMap(t, sidecar, "messages.markAllReadUnified", map[string]any{"folder": "INBOX"})
	if failures, _ := result["failures"].([]any); len(failures) > 0 {
		t.Fatalf("markAllReadUnified reported account failures: %v", failures)
	}
	deadline := time.Now().Add(30 * time.Second)
	for {
		merged = callMap(t, sidecar, "messages.unifiedRecent", map[string]any{
			"limit":   50,
			"refresh": false,
		})
		unread, _ := merged["folder_unread"].(float64)
		if unread == 0 {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("unified folder_unread stayed at %v after markAllReadUnified: %v", unread, merged)
		}
		time.Sleep(500 * time.Millisecond)
	}
}

func checkUnifiedMarkAllReadReportsAPartialFailure(t *testing.T) {
	server := startMailServer(t)
	sidecar, _ := startSidecar(t)
	connectAccount(t, sidecar, server, "alice", "alice@mail.test")
	connectAccount(t, sidecar, server, "bob", "bob@mail.test")
	nonce := fmt.Sprintf("%d", time.Now().UnixNano())

	// Give both healthy accounts a known unread row, and give a third account
	// a cached row before replacing its credentials with a bad password.
	// The fan-out must continue past that failure and identify it precisely.
	aliceSubject := "Meron integration unified partial alice " + nonce
	bobSubject := "Meron integration unified partial bob " + nonce
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "bob",
		"to":         "alice@mail.test",
		"subject":    aliceSubject,
		"body":       "healthy alice unread",
		"message_id": fmt.Sprintf("itest-unified-partial-alice-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send alice partial-failure fixture: %v", err)
	}
	if _, err := sidecar.Call("send", map[string]any{
		"account":    "alice",
		"to":         "bob@mail.test",
		"subject":    bobSubject,
		"body":       "healthy bob unread",
		"message_id": fmt.Sprintf("itest-unified-partial-bob-%s@mail.test", nonce),
	}); err != nil {
		t.Fatalf("send bob partial-failure fixture: %v", err)
	}
	aliceMessage := pollInbox(t, sidecar, "alice", func(m map[string]any) bool {
		return str(m, "subject") == aliceSubject
	})
	bobMessage := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return str(m, "subject") == bobSubject
	})

	broken := "itest-unified-broken"
	connectAccount(t, sidecar, server, broken, "bob@mail.test")
	t.Cleanup(func() {
		if _, err := sidecar.Call("account.remove", map[string]any{"account": broken}); err != nil {
			t.Logf("remove partial-failure account: %v", err)
		}
	})
	pollInbox(t, sidecar, broken, func(m map[string]any) bool {
		return str(m, "subject") == bobSubject
	})
	callMap(t, sidecar, "account.connect", map[string]any{
		"account":   broken,
		"email":     "bob@mail.test",
		"host":      "127.0.0.1",
		"port":      server.imapPort,
		"tls":       false,
		"smtp_host": "127.0.0.1",
		"smtp_port": server.smtpPort,
		"smtp_tls":  false,
		"user":      "bob@mail.test",
		"password":  "definitely-not-" + testPassword,
		"validate":  false,
	})
	// Drop any authenticated connection retained from the initial sync.
	callMap(t, sidecar, "account.setPaused", map[string]any{"account": broken, "enabled": true})

	result := callMap(t, sidecar, "messages.markAllReadUnified", map[string]any{"folder": "INBOX"})
	if boolValue(result, "ok") {
		t.Fatalf("markAllReadUnified reported success despite a broken account: %v", result)
	}
	failures, _ := result["failures"].([]any)
	if len(failures) != 1 {
		t.Fatalf("markAllReadUnified failures len = %d, want 1: %v", len(failures), result)
	}
	failure, ok := failures[0].(map[string]any)
	if !ok || str(failure, "account_id") != broken {
		t.Fatalf("markAllReadUnified failure = %v, want account %s", failures[0], broken)
	}
	folderUnreads, _ := result["folder_unreads"].(map[string]any)
	for _, account := range []string{"alice", "bob"} {
		if _, ok := folderUnreads[account]; !ok {
			t.Fatalf("healthy account %s missing from markAllReadUnified result: %v", account, result)
		}
	}
	for _, fixture := range []struct {
		user string
		uid  uint32
	}{
		{"alice@mail.test", num(aliceMessage, "uid")},
		{"bob@mail.test", num(bobMessage, "uid")},
	} {
		if flags := imapFlags(t, server.imapPort, fixture.user, testPassword, "INBOX", fixture.uid); !strings.Contains(flags, `\Seen`) {
			t.Fatalf("healthy account %s uid %d was not marked read on the server: %q", fixture.user, fixture.uid, flags)
		}
	}
}

func checkUnifiedCursorsAdvanceEachAccountIndependently(t *testing.T) {
	server := startMailServer(t)
	sidecar, _ := startSidecar(t)
	connectAccount(t, sidecar, server, "alice", "alice@mail.test")
	connectAccount(t, sidecar, server, "bob", "bob@mail.test")
	nonce := fmt.Sprintf("%d", time.Now().UnixNano())

	aliceSubject := "Meron integration unified cursor alice " + nonce
	bobSubject := "Meron integration unified cursor bob " + nonce
	// Alice must be newer than Bob's rows so she is exhausted on the first
	// page. Wall-clock send dates can put her last when sends cross a second.
	newest := time.Date(2025, time.January, 1, 12, 0, 0, 0, time.UTC)
	fixtures := []struct {
		account, subject, id string
		date                 time.Time
	}{{"alice", aliceSubject, "alice", newest}}
	for i := 0; i < 5; i++ {
		fixtures = append(fixtures, struct {
			account, subject, id string
			date                 time.Time
		}{
			"bob",
			fmt.Sprintf("%s %d", bobSubject, i),
			fmt.Sprintf("bob-%d", i),
			newest.Add(-time.Duration(i+1) * time.Minute),
		})
	}
	for _, fixture := range fixtures {
		user := fixture.account + "@mail.test"
		imapAppend(t, server.imapPort, user, testPassword, "INBOX", rawMessage([]string{
			"From: Carol <carol@example.net>",
			"To: " + user,
			"Subject: " + fixture.subject,
			fmt.Sprintf("Message-ID: <itest-unified-cursor-%s-%s@mail.test>", fixture.id, nonce),
			"Date: " + fixture.date.Format(time.RFC1123Z),
		}, "unified cursor fixture"))
	}
	for _, fixture := range fixtures {
		pollInbox(t, sidecar, fixture.account, func(m map[string]any) bool {
			return str(m, "subject") == fixture.subject
		})
	}

	seenIDs := map[string]bool{}
	seenSubjects := map[string]bool{}
	cursor := ""
	sawBobOnlyContinuation := false
	for pageNumber := 0; ; pageNumber++ {
		if pageNumber > 50 {
			t.Fatal("unified cursor did not terminate within 50 pages")
		}
		params := map[string]any{
			"limit":   3,
			"refresh": false,
		}
		if cursor != "" {
			params["before_cursor"] = cursor
		}
		page := callMap(t, sidecar, "messages.unifiedRecent", params)
		if failures, _ := page["failures"].([]any); len(failures) != 0 {
			t.Fatalf("unified cursor page reported failures: %v", page)
		}
		threads, _ := page["threads"].([]any)
		accountsOnPage := map[string]bool{}
		lastDate := int64(-1)
		for _, item := range threads {
			card, ok := item.(map[string]any)
			if !ok {
				t.Fatalf("unified cursor card has type %T", item)
			}
			id := str(card, "id")
			if id == "" {
				t.Fatalf("unified cursor card has no id: %v", card)
			}
			if seenIDs[id] {
				t.Fatalf("unified cursor repeated card %q: %v", id, card)
			}
			seenIDs[id] = true
			accountsOnPage[str(card, "account_id")] = true
			seenSubjects[str(card, "subject")] = true
			date, _ := card["date"].(float64)
			if lastDate >= 0 && int64(date) > lastDate {
				t.Fatalf("unified cursor page is not sorted newest first: %v", threads)
			}
			lastDate = int64(date)
		}
		if pageNumber > 0 && accountsOnPage["bob"] && !accountsOnPage["alice"] {
			sawBobOnlyContinuation = true
		}
		next, _ := page["next_cursor"].(string)
		if next == "" {
			break
		}
		if next == cursor {
			t.Fatalf("unified cursor did not advance: %q", next)
		}
		cursor = next
	}
	for _, fixture := range fixtures {
		if !seenSubjects[fixture.subject] {
			t.Fatalf("unified cursor traversal missed %q", fixture.subject)
		}
	}
	if !sawBobOnlyContinuation {
		t.Fatal("unified cursor never continued bob after alice was exhausted")
	}
}
