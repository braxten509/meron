//go:build integration

// Integration scenarios for connections.
// Runs with the integration build tag against the test mail server.
// The main test owns account setup and the order of dependent scenarios.
package main

import (
	"fmt"
	"strings"
	"testing"
	"time"
)

func checkColdSyncFromAnEmptyProfileSeesServerState(t *testing.T, server *mailServer, nonce string) {
	// Everything above reads back through the same store that performed the
	// write, so a write that never left the client would still pass. A second
	// sidecar on a fresh profile has no cached rows at all: whatever it
	// reports came from the mail server.
	cold, _ := startSidecar(t)
	connectAccount(t, cold, server, "bob", "bob@mail.test")

	moveSubject := "Meron integration move " + nonce
	row := pollFolder(t, cold, "bob", "ITestFolder", func(m map[string]any) bool {
		return stringField(m, "subject") == moveSubject
	})
	if !boolValue(row, "seen") || !boolValue(row, "starred") {
		t.Fatalf("cold sync does not see the flags a warm store reported: %v", row)
	}
	// Bodies come from IMAP too — nothing local can stand in for them.
	thread := callMap(t, cold, "messages.thread", map[string]any{
		"account":    "bob",
		"folder":     "ITestFolder",
		"thread_key": stringField(row, "thread_key"),
	})
	message := firstThreadMessage(t, thread)
	if boolValue(message, "body_missing") {
		t.Fatalf("cold sync could not fetch the message body: %v", message)
	}
	if body := stringField(message, "body"); !strings.Contains(body, "move me to the integration folder") {
		t.Fatalf("cold-synced body = %q", body)
	}
}

func checkConnectRejectsBadCredentials(t *testing.T, sidecar *Sidecar, server *mailServer) {
	// Validation is what stands between a typo'd password and an account that
	// looks connected but can never sync, so the account must not be stored.
	id := "itest-badpass"
	_, err := sidecar.Call("account.connect", map[string]any{
		"account":   id,
		"email":     "alice@mail.test",
		"host":      "127.0.0.1",
		"port":      server.imapPort,
		"tls":       false,
		"smtp_host": "127.0.0.1",
		"smtp_port": server.smtpPort,
		"smtp_tls":  false,
		"user":      "alice@mail.test",
		"password":  "definitely-not-" + testPassword,
		"validate":  true,
	})
	if err == nil {
		t.Fatal("account.connect accepted a wrong password")
	}
	accounts, _ := callMap(t, sidecar, "account.list", map[string]any{})["accounts"].([]any)
	for _, item := range accounts {
		account, ok := item.(map[string]any)
		if ok && stringField(account, "id") == id {
			t.Fatalf("rejected account was stored anyway: %v", account)
		}
	}
}

func checkSyncRecoversAfterAServerRestart(t *testing.T) {
	server := startMailServer(t)
	sidecar, events := startSidecar(t)
	connectAccount(t, sidecar, server, "alice", "alice@mail.test")
	connectAccount(t, sidecar, server, "bob", "bob@mail.test")
	nonce := fmt.Sprintf("%d", time.Now().UnixNano())

	// A server that goes away must not need a client restart: reads retry a
	// stale pooled connection, and an active IDLE watcher must reconnect and
	// catch up without a client restart or an explicit refresh.
	folder := "ITestIdleRestart"
	if result := callMap(t, sidecar, "folders.create", map[string]any{"account": "bob", "name": folder}); !foldersContain(result, folder) {
		t.Fatalf("folders.create did not return %s: %v", folder, result)
	}
	synced := func() int {
		return events.count(func(event sidecarEvent) bool {
			return event.name == "mail.synced" && stringField(event.detail, "account") == "bob" &&
				strings.EqualFold(stringField(event.detail, "folder"), folder)
		})
	}
	callMap(t, sidecar, "watch.start", map[string]any{"account": "bob", "folder": folder})
	deadline := time.Now().Add(30 * time.Second)
	for synced() == 0 {
		if time.Now().After(deadline) {
			t.Fatalf("IDLE watch for %s never completed its initial catch-up", folder)
		}
		time.Sleep(200 * time.Millisecond)
	}
	t.Cleanup(func() {
		_, _ = sidecar.Call("watch.stop", map[string]any{"account": "bob", "folder": folder})
	})
	baseline := synced()

	restartMailServer(t, server)

	idleRestartSubject := "Meron integration idle restart " + nonce
	imapAppend(t, server.imapPort, "bob@mail.test", testPassword, folder, rawMessage([]string{
		"From: Carol <carol@example.net>",
		"To: bob@mail.test",
		"Subject: " + idleRestartSubject,
		fmt.Sprintf("Message-ID: <itest-idle-restart-%s@mail.test>", nonce),
		"Date: " + time.Now().Format(time.RFC1123Z),
	}, "caught up after the IDLE connection was severed"))
	deadline = time.Now().Add(60 * time.Second)
	for synced() == baseline {
		if time.Now().After(deadline) {
			t.Fatalf("IDLE watch for %s never recovered after restart", folder)
		}
		time.Sleep(200 * time.Millisecond)
	}
	cached := callMap(t, sidecar, "messages.recent", map[string]any{
		"account": "bob",
		"folder":  folder,
		"refresh": false,
		"limit":   50,
	})
	if !messagesContainSubject(cached, idleRestartSubject) {
		t.Fatalf("reconnected IDLE watch did not cache %q: %v", idleRestartSubject, cached)
	}

	restartSubject := "Meron integration restart " + nonce
	// Appended rather than sent: SMTP submission is a write path, and writes
	// deliberately do not retry a pooled connection.
	imapAppend(t, server.imapPort, "bob@mail.test", testPassword, "INBOX", rawMessage([]string{
		"From: Carol <carol@example.net>",
		"To: bob@mail.test",
		"Subject: " + restartSubject,
		fmt.Sprintf("Message-ID: <itest-restart-%s@mail.test>", nonce),
		"Date: " + time.Now().Format(time.RFC1123Z),
	}, "delivered after the restart"))

	// The refresh itself can fail while the reconnect is in flight, so retry
	// the call, not just the assertion.
	deadline = time.Now().Add(90 * time.Second)
	for {
		result, err := sidecar.Call("messages.recent", map[string]any{
			"account": "bob",
			"folder":  "INBOX",
			"refresh": true,
			"limit":   50,
		})
		if err == nil {
			if page, ok := result.(map[string]any); ok && messagesContainSubject(page, restartSubject) {
				break
			}
		}
		if time.Now().After(deadline) {
			t.Fatalf("sync never recovered after the server restart (last error: %v)", err)
		}
		time.Sleep(1 * time.Second)
	}

	// Reads recovered on their own; write sessions do not retry, so drop the
	// pooled sockets both accounts may still hold from before the restart
	// rather than leaving them for later subtests.
	for _, account := range []string{"alice", "bob"} {
		callMap(t, sidecar, "account.setPaused", map[string]any{"account": account, "enabled": true})
		callMap(t, sidecar, "account.setPaused", map[string]any{"account": account, "enabled": false})
	}
}

func checkFlagWritesRecoverFromAStalePooledConnection(t *testing.T) {
	// Regression test: a pooled session the server had silently dropped
	// failed messages.markRead outright ("SELECT: Broken pipe"), so the
	// flag never reached the server and the next sync pulled the message
	// back as unread. The SELECT now runs as a retryable preflight, ahead
	// of the STORE that must not be replayed.
	server := startMailServer(t)
	sidecar, _ := startSidecar(t)
	// The account talks to the mail server through a proxy the test can cut, so the
	// pooled socket can be killed without taking the server down with it.
	proxy := startIMAPProxy(t, server.imapPort)
	proxied := *server
	proxied.imapPort = proxy.port()
	connectAccount(t, sidecar, &proxied, "bob", "bob@mail.test")
	nonce := fmt.Sprintf("%d", time.Now().UnixNano())

	staleSubject := "Meron integration stale pool " + nonce
	imapAppend(t, server.imapPort, "bob@mail.test", testPassword, "INBOX", rawMessage([]string{
		"From: Carol <carol@example.net>",
		"To: bob@mail.test",
		"Subject: " + staleSubject,
		fmt.Sprintf("Message-ID: <itest-stale-pool-%s@mail.test>", nonce),
		"Date: " + time.Now().Format(time.RFC1123Z),
	}, "flagged over a pooled connection the server dropped"))

	message := pollInbox(t, sidecar, "bob", func(m map[string]any) bool {
		return stringField(m, "subject") == staleSubject
	})
	uid := numericField(message, "uid")
	if uid == 0 {
		t.Fatalf("delivered stale-pool fixture has no uid: %v", message)
	}

	// The sync above leaves a warm session in the pool; severing it is
	// invisible to the client until its next command, so a flag write can
	// start out holding a dead socket. Stop the IDLE watcher first, and let its connection go: a reconnecting
	// watcher would otherwise be the one to pick the dead session out of the
	// pool, leaving the flag write a healthy connection and no bug to catch.
	if _, err := sidecar.Call("watch.stop", map[string]any{"account": "bob", "folder": "INBOX"}); err != nil {
		t.Logf("watch.stop: %v", err)
	}
	time.Sleep(500 * time.Millisecond)

	// Both writes below start out holding a severed session: the first one
	// leaves a healthy connection behind, so sever again before the second.
	proxy.sever()
	callMap(t, sidecar, "messages.markRead", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"uid":     uid,
		"seen":    true,
	})
	proxy.sever()
	callMap(t, sidecar, "messages.markStarred", map[string]any{
		"account": "bob",
		"folder":  "INBOX",
		"uid":     uid,
		"starred": true,
	})

	// Server truth, not the local store: a STORE that never left the
	// client would look identical in messages.recent.
	if flags := imapFlags(t, server.imapPort, "bob@mail.test", testPassword, "INBOX", uid); !strings.Contains(flags, `\Seen`) || !strings.Contains(flags, `\Flagged`) {
		t.Fatalf("server flags for INBOX uid %d = %q, want \\Seen and \\Flagged", uid, flags)
	}
}
