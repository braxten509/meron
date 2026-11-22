// Verifies the desktop mail tools with fixture accounts.
// Attachment reads and searches exercise the granted account scope.
// Draft updates use the same fixture transport as the other mail commands.

package main

import (
	"encoding/base64"

	"encoding/json"
	"fmt"

	"os"
	"path/filepath"

	"strings"
	"testing"
)

func TestMCPAttachmentStaysInsideApprovedAccount(t *testing.T) {
	t.Setenv("XDG_CACHE_HOME", t.TempDir())
	write := func(key string, size int) {
		t.Helper()
		path := filepath.Join(mediaDirectory(), filepath.FromSlash(key))
		if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte(strings.Repeat("a", size)), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	write("allowed/INBOX/7/0.png", 3)
	write("excluded/INBOX/7/0.png", 3)
	write("allowed/INBOX/7/1.bin", mcpAttachmentMaxBytes+1)
	// Every read that clears the key prefix asks the core which accounts exist,
	// so the collision check has a live answer to work from.
	accounts := sidecarResponsePlan{Result: map[string]any{"accounts": []any{
		map[string]any{"id": "allowed", "email": "allowed@example.com"},
		map[string]any{"id": "excluded", "email": "excluded@example.com"},
	}}}
	app, _ := newMailHandlerTestApp(t, accounts, accounts, accounts)
	session := connectTestMCP(t, testMCPService(t, app, false))

	result := callMCP(t, session, "read_attachment", map[string]any{"account_id": "allowed", "key": "allowed/INBOX/7/0.png"})
	encoded, _ := json.Marshal(result)
	if result.IsError || !strings.Contains(string(encoded), base64.StdEncoding.EncodeToString([]byte("aaa"))) || !strings.Contains(string(encoded), "image/png") {
		t.Fatalf("result=%s", encoded)
	}
	for _, tc := range []struct {
		name string
		args map[string]any
	}{
		{"other account", map[string]any{"account_id": "allowed", "key": "excluded/INBOX/7/0.png"}},
		{"unapproved account", map[string]any{"account_id": "excluded", "key": "excluded/INBOX/7/0.png"}},
		{"traversal", map[string]any{"account_id": "allowed", "key": "allowed/../excluded/INBOX/7/0.png"}},
		{"absolute", map[string]any{"account_id": "allowed", "key": "/etc/passwd"}},
		{"bare key", map[string]any{"account_id": "allowed", "key": "allowed"}},
		{"not cached", map[string]any{"account_id": "allowed", "key": "allowed/INBOX/9/0.png"}},
		{"too large", map[string]any{"account_id": "allowed", "key": "allowed/INBOX/7/1.bin"}},
	} {
		if !callMCP(t, session, "read_attachment", tc.args).IsError {
			t.Fatalf("allowed %s", tc.name)
		}
	}
}

// Two account IDs can fold to one media segment ("a+b@" and "a_b@" both become
// "a_b_example.com"), so the key prefix stops proving ownership and the read is
// refused rather than serving the other account's cache.
func TestMCPAttachmentRefusesSharedMediaSegment(t *testing.T) {
	t.Setenv("XDG_CACHE_HOME", t.TempDir())
	path := filepath.Join(mediaDirectory(), filepath.FromSlash("a_b_example.com/INBOX/7/0.png"))
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("aaa"), 0o600); err != nil {
		t.Fatal(err)
	}
	app, _ := newMailHandlerTestApp(t, sidecarResponsePlan{Result: map[string]any{"accounts": []any{
		map[string]any{"id": "a+b@example.com", "email": "a+b@example.com"},
		map[string]any{"id": "a_b@example.com", "email": "a_b@example.com"},
	}}})
	service := testMCPService(t, app, false)
	service.config.Clients[0].Accounts = []string{"a+b@example.com"}
	session := connectTestMCP(t, service)
	result := callMCP(t, session, "read_attachment", map[string]any{"account_id": "a+b@example.com", "key": "a_b_example.com/INBOX/7/0.png"})
	encoded, _ := json.Marshal(result)
	if !result.IsError || strings.Contains(string(encoded), base64.StdEncoding.EncodeToString([]byte("aaa"))) {
		t.Fatalf("result=%s", encoded)
	}
}

// accountList reports no accounts when the core is down, and no accounts looks
// exactly like no collision. The read has to be refused instead, or a stopped
// core reopens the shared-prefix hole against attachments still on disk.
func TestMCPAttachmentRefusesWhenCoreIsUnavailable(t *testing.T) {
	t.Setenv("XDG_CACHE_HOME", t.TempDir())
	path := filepath.Join(mediaDirectory(), filepath.FromSlash("a_b_example.com/INBOX/7/0.png"))
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("aaa"), 0o600); err != nil {
		t.Fatal(err)
	}
	service := testMCPService(t, &App{}, false)
	service.config.Clients[0].Accounts = []string{"a+b@example.com"}
	session := connectTestMCP(t, service)
	result := callMCP(t, session, "read_attachment", map[string]any{"account_id": "a+b@example.com", "key": "a_b_example.com/INBOX/7/0.png"})
	encoded, _ := json.Marshal(result)
	if !result.IsError || strings.Contains(string(encoded), base64.StdEncoding.EncodeToString([]byte("aaa"))) {
		t.Fatalf("result=%s", encoded)
	}
}

// The merge keeps only the newest 50 across accounts, so an account whose older
// results were dropped gets no cursor: following one would page past mail this
// response never carried.
func TestMCPUnscopedSearchWithholdsCursorsForTruncatedAccounts(t *testing.T) {
	cards := func(base int64) []any {
		out := []any{}
		for i := 0; i < mcpSearchLimit; i++ {
			out = append(out, map[string]any{"thread_key": fmt.Sprintf("t%d-%d", base, i), "subject": "s", "date": float64(base - int64(i))})
		}
		return out
	}
	app, _ := newMailHandlerTestApp(t,
		sidecarResponsePlan{Result: map[string]any{"accounts": []any{
			map[string]any{"id": "allowed", "email": "allowed@example.com"},
			map[string]any{"id": "second", "email": "second@example.com"},
		}}},
		sidecarResponsePlan{Result: map[string]any{"cards": cards(100000), "next_cursor": "cursor-allowed"}},
		sidecarResponsePlan{Result: map[string]any{"cards": cards(200), "next_cursor": "cursor-second"}})
	service := testMCPService(t, app, false)
	service.config.Clients[0].Accounts = []string{"allowed", "second"}
	session := connectTestMCP(t, service)
	result := callMCP(t, session, "search_messages", nil)
	if result.IsError {
		t.Fatal(result)
	}
	var page struct {
		Accounts []struct {
			AccountID  string `json:"account_id"`
			Returned   int    `json:"returned"`
			NextCursor string `json:"next_cursor"`
			Truncated  bool   `json:"truncated_by_merge"`
		} `json:"accounts"`
		Results struct{ Threads []Message } `json:"results"`
	}
	encoded, _ := json.Marshal(result.StructuredContent)
	if err := json.Unmarshal(encoded, &page); err != nil {
		t.Fatal(err)
	}
	if len(page.Results.Threads) != mcpSearchLimit || len(page.Accounts) != 2 {
		t.Fatalf("page=%s", encoded)
	}
	// "allowed" owns the whole merged page and pages on with its own cursor;
	// "second" contributed nothing, so its cursor must not be handed out.
	if page.Accounts[0].Returned != mcpSearchLimit || page.Accounts[0].NextCursor != "cursor-allowed" || page.Accounts[0].Truncated {
		t.Fatalf("allowed=%#v", page.Accounts[0])
	}
	if page.Accounts[1].Returned != 0 || page.Accounts[1].NextCursor != "" || !page.Accounts[1].Truncated {
		t.Fatalf("second=%#v", page.Accounts[1])
	}
}

func TestMCPUpdateDraftReplacesDraftInPlace(t *testing.T) {
	app, writer := newMailHandlerTestApp(t, sidecarResponsePlan{Result: map[string]any{"ok": true}}, sidecarResponsePlan{Result: map[string]any{"ok": true}}, sidecarResponsePlan{Result: map[string]any{"ok": true}})
	session := connectTestMCP(t, testMCPService(t, app, true))
	created := callMCP(t, session, "create_draft", map[string]any{"account_id": "allowed", "body": "first"})
	if created.IsError {
		t.Fatal(created)
	}
	draftID, _ := writer.calls[0].Params["draft_id"].(string)
	if draftID == "" {
		t.Fatal(writer.calls)
	}
	if result := callMCP(t, session, "update_draft", map[string]any{"account_id": "allowed", "draft_id": draftID, "body": "second", "subject": "Revised"}); result.IsError {
		t.Fatal(result)
	}
	if len(writer.calls) != 2 {
		t.Fatal(writer.calls)
	}
	update := writer.calls[1]
	if update.Method != "save_draft" || update.Params["draft_id"] != draftID || update.Params["body"] != "second" || update.Params["subject"] != "Revised" {
		t.Fatal(update)
	}
	// A draft the user started in Meron carries its own Message-ID, which
	// read_thread reports; revising it is the point of the tool.
	if result := callMCP(t, session, "update_draft", map[string]any{"account_id": "allowed", "draft_id": "<compose-1@example.com>", "body": "revised"}); result.IsError {
		t.Fatal(result)
	}
	if len(writer.calls) != 3 || writer.calls[2].Params["draft_id"] != "<compose-1@example.com>" {
		t.Fatal(writer.calls)
	}
	for _, tc := range []struct {
		name string
		args map[string]any
	}{
		{"malformed draft_id", map[string]any{"account_id": "allowed", "draft_id": "compose-1", "body": "body"}},
		{"header injection in draft_id", map[string]any{"account_id": "allowed", "draft_id": "<a@b>\r\nBcc: victim@example.com", "body": "body"}},
		{"missing draft_id", map[string]any{"account_id": "allowed", "body": "body"}},
		{"other account", map[string]any{"account_id": "excluded", "draft_id": draftID, "body": "body"}},
		{"header injection", map[string]any{"account_id": "allowed", "draft_id": draftID, "body": "body", "to": "a@example.com\r\nBcc: victim@example.com"}},
	} {
		if !callMCP(t, session, "update_draft", tc.args).IsError {
			t.Fatalf("allowed %s", tc.name)
		}
	}
}
