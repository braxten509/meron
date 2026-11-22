//go:build integration

// End-to-end mail flow and common assertions.
// Runs with the integration build tag against the test mail server.
// The main test owns account setup and the order of dependent scenarios.
package main

import (
	"fmt"
	"testing"
	"time"
)

// pollFolder refreshes an account folder until predicate matches a message row
// (messages.recent triggers a background IMAP sync and serves the store cache).
func pollFolder(t *testing.T, sidecar *Sidecar, account, folder string, predicate func(map[string]any) bool) map[string]any {
	t.Helper()
	deadline := time.Now().Add(60 * time.Second)
	for time.Now().Before(deadline) {
		result := callMap(t, sidecar, "messages.recent", map[string]any{
			"account": account,
			"folder":  folder,
			"refresh": true,
			"limit":   50,
		})
		rows, _ := result["messages"].([]any)
		for _, row := range rows {
			message, ok := row.(map[string]any)
			if ok && predicate(message) {
				return message
			}
		}
		time.Sleep(500 * time.Millisecond)
	}
	t.Fatalf("message did not arrive in %s %s within deadline", account, folder)
	return nil
}

func pollInbox(t *testing.T, sidecar *Sidecar, account string, predicate func(map[string]any) bool) map[string]any {
	t.Helper()
	return pollFolder(t, sidecar, account, "INBOX", predicate)
}

func assertNoMessageInFolder(t *testing.T, sidecar *Sidecar, account, folder string, predicate func(map[string]any) bool) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		result := callMap(t, sidecar, "messages.recent", map[string]any{
			"account": account,
			"folder":  folder,
			"refresh": true,
			"limit":   50,
		})
		rows, _ := result["messages"].([]any)
		found := false
		for _, row := range rows {
			message, ok := row.(map[string]any)
			if ok && predicate(message) {
				found = true
				break
			}
		}
		if !found {
			return
		}
		time.Sleep(500 * time.Millisecond)
	}
	t.Fatalf("message still present in %s %s after deadline", account, folder)
}

func str(message map[string]any, key string) string {
	value, _ := message[key].(string)
	return value
}

func num(message map[string]any, key string) uint32 {
	switch value := message[key].(type) {
	case float64:
		return uint32(value)
	case int:
		return uint32(value)
	case uint32:
		return value
	default:
		return 0
	}
}

func boolValue(message map[string]any, key string) bool {
	value, _ := message[key].(bool)
	return value
}

func TestIMAPResponseParsing(t *testing.T) {
	t.Run("tagged status is case insensitive", func(t *testing.T) {
		if !imapTaggedOK("t4 ok completed", "t4") {
			t.Fatal("lowercase tagged OK was rejected")
		}
		if imapTaggedOK("t4 NO failed", "t4") {
			t.Fatal("tagged NO was accepted")
		}
		if imapTaggedOK("t40 OK completed", "t4") {
			t.Fatal("different tag was accepted")
		}
	})

	t.Run("uidvalidity tolerates response text and casing", func(t *testing.T) {
		got, ok := parseIMAPUIDValidity([]string{
			"* FLAGS (\\Seen)",
			"* ok [uidvalidity 4294967295] UIDs valid",
		})
		if !ok || got != ^uint32(0) {
			t.Fatalf("UIDVALIDITY = %d, %v; want %d, true", got, ok, ^uint32(0))
		}
		for _, lines := range [][]string{
			{"* OK mailbox selected"},
			{"* OK [UIDVALIDITY 0] invalid"},
			{"* OK [UIDVALIDITY nope] invalid"},
		} {
			if got, ok := parseIMAPUIDValidity(lines); ok || got != 0 {
				t.Fatalf("invalid UIDVALIDITY parsed as %d, %v from %v", got, ok, lines)
			}
		}
	})

	t.Run("flags tolerate reordered fetch fields", func(t *testing.T) {
		lines := []string{
			"* 3 FETCH (FLAGS (\\Seen custom) RFC822.SIZE 42 uid 77)",
			"* 4 FETCH (UID 78 FLAGS (\\Flagged))",
		}
		if got := parseIMAPFlags(lines, 77); got != `\Seen custom` {
			t.Fatalf("flags for UID 77 = %q", got)
		}
		if got := parseIMAPFlags(lines, 78); got != `\Flagged` {
			t.Fatalf("flags for UID 78 = %q", got)
		}
		if got := parseIMAPFlags(lines, 79); got != "" {
			t.Fatalf("flags for absent UID = %q", got)
		}
	})

	t.Run("search handles empty and malformed fields", func(t *testing.T) {
		got := parseIMAPSearch([]string{
			"* SEARCH 2 nope 3x 0 17",
			"* VANISHED 99",
		})
		if fmt.Sprint(got) != "[2 17]" {
			t.Fatalf("SEARCH UIDs = %v, want [2 17]", got)
		}
		if got := parseIMAPSearch([]string{"* SEARCH"}); len(got) != 0 {
			t.Fatalf("empty SEARCH = %v", got)
		}
	})
}

func TestIntegrationMailFlow(t *testing.T) {
	server := startMailServer(t)
	sidecar, events := startSidecar(t)

	connectAccount(t, sidecar, server, "alice", "alice@mail.test")
	connectAccount(t, sidecar, server, "bob", "bob@mail.test")

	t.Run("folders", func(t *testing.T) { checkFolders(t, sidecar) })

	nonce := fmt.Sprintf("%d", time.Now().UnixNano())
	subject := "Meron integration " + nonce
	// Bare id (no angle brackets) — the app convention: the frontend mints bare
	// ids and the backend wraps them when emitting headers.
	messageID := fmt.Sprintf("itest-%s@mail.test", nonce)

	t.Run("send and receive", func(t *testing.T) { checkSendAndReceive(t, sidecar, subject, messageID) })

	t.Run("threading", func(t *testing.T) { checkThreading(t, sidecar, subject, messageID) })

	t.Run("move and flags", func(t *testing.T) { checkMoveAndFlags(t, sidecar, server, nonce) })

	t.Run("failed imap write does not falsely mark read locally", func(t *testing.T) {
		checkFailedImapWriteDoesNotFalselyMarkReadLocally(t, sidecar, server, nonce)
	})

	t.Run("search and starred items", func(t *testing.T) { checkSearchAndStarredItems(t, sidecar, nonce) })

	t.Run("draft lifecycle", func(t *testing.T) { checkDraftLifecycle(t, sidecar, nonce) })

	t.Run("quick reply draft lifecycle", func(t *testing.T) { checkQuickReplyDraftLifecycle(t, sidecar, nonce) })

	t.Run("attachments", func(t *testing.T) { checkAttachments(t, sidecar, nonce) })

	t.Run("delete moves to trash", func(t *testing.T) { checkDeleteMovesToTrash(t, sidecar, nonce) })

	t.Run("empty trash", func(t *testing.T) { checkEmptyTrash(t, sidecar, nonce) })

	t.Run("copy keeps original", func(t *testing.T) { checkCopyKeepsOriginal(t, sidecar, nonce) })

	t.Run("mark all read", func(t *testing.T) { checkMarkAllRead(t, sidecar, nonce) })

	t.Run("contacts suggest from history", func(t *testing.T) { checkContactsSuggestFromHistory(t, sidecar) })

	t.Run("archive folder resolves", func(t *testing.T) { checkArchiveFolderResolves(t, sidecar) })

	t.Run("grouped thread cards", func(t *testing.T) { checkGroupedThreadCards(t, sidecar) })

	t.Run("external expunge prunes the local row", func(t *testing.T) { checkExternalExpungePrunesTheLocalRow(t, sidecar, server, nonce) })

	t.Run("external flag changes reconcile locally", func(t *testing.T) { checkExternalFlagChangesReconcileLocally(t) })

	t.Run("uidvalidity change clears the cached folder", func(t *testing.T) { checkUidvalidityChangeClearsTheCachedFolder(t, sidecar, server, nonce) })

	t.Run("idle pushes folder updates while watched", func(t *testing.T) {
		checkIdlePushesFolderUpdatesWhileWatched(t, sidecar, server, events, nonce)
	})

	t.Run("unified inbox merges and writes across accounts", func(t *testing.T) { checkUnifiedInboxMergesAndWritesAcrossAccounts(t, sidecar, nonce, subject) })

	t.Run("unified mark-all-read reports a partial failure", func(t *testing.T) { checkUnifiedMarkAllReadReportsAPartialFailure(t) })

	t.Run("unified cursors advance each account independently", func(t *testing.T) {
		checkUnifiedCursorsAdvanceEachAccountIndependently(t)
	})

	t.Run("encoded subject and quoted-printable body decode", func(t *testing.T) {
		checkEncodedSubjectAndQuotedPrintableBodyDecode(t, sidecar, server, nonce)
	})

	t.Run("multipart alternative keeps both text and html", func(t *testing.T) { checkMultipartAlternativeKeepsBothTextAndHtml(t, sidecar, server, nonce) })

	t.Run("nested multipart keeps inline and downloadable attachments", func(t *testing.T) {
		checkNestedMultipartKeepsInlineAndDownloadableAttachments(t)
	})

	t.Run("message without a message-id still threads", func(t *testing.T) { checkMessageWithoutAMessageIdStillThreads(t, sidecar, server, nonce) })

	t.Run("empty junk", func(t *testing.T) { checkEmptyJunk(t, sidecar, server, nonce) })

	t.Run("clearing the attachment cache refetches the bytes", func(t *testing.T) { checkClearingTheAttachmentCacheRefetchesTheBytes(t, sidecar, nonce) })

	t.Run("cold sync from an empty profile sees server state", func(t *testing.T) { checkColdSyncFromAnEmptyProfileSeesServerState(t, server, nonce) })

	t.Run("sent copy honors the account override", func(t *testing.T) { checkSentCopyHonorsTheAccountOverride(t, sidecar, server, nonce) })

	t.Run("connect rejects bad credentials", func(t *testing.T) { checkConnectRejectsBadCredentials(t, sidecar, server) })

	t.Run("sync recovers after a server restart", func(t *testing.T) { checkSyncRecoversAfterAServerRestart(t) })

	t.Run("flag writes recover from a stale pooled connection", func(t *testing.T) {
		checkFlagWritesRecoverFromAStalePooledConnection(t)
	})

	// Last on purpose: no later subtest sends from alice, so nothing but the
	// piggyback under test can refresh her Sent folder.
	t.Run("sent from another client surfaces via inbox sync", func(t *testing.T) { checkSentFromAnotherClientSurfacesViaInboxSync(t, sidecar, server, nonce) })
}
