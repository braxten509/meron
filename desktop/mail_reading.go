// Conversation lists, starred items and individual thread reads.
// These handlers translate folder and pagination state from the desktop request.
// The core supplies the messages and identities used in the frontend response.

package main

import (
	"encoding/json"
	"strings"
)

func (a *App) threadList(payload map[string]any) (any, error) {
	var request ThreadListRequest
	_ = decode(payload, &request)
	if request.AccountID == "" {
		request.AccountID, _ = payload["account_id"].(string)
	}
	if request.FolderID == "" {
		request.FolderID = "inbox"
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return map[string]any{"threads": []Message{}}, nil
	}
	if request.Filter == "starred" {
		a.logf("threadList starred: account=%s folder=%s query=%q refresh=%t", request.AccountID, request.FolderID, request.Query, request.Refresh)
	}
	method := "messages.recent"
	// The unified fan-out resolves a *role* per account, not a folder id. The
	// frontend sends the role alongside the folder; without forwarding it the
	// core falls back to "inbox" and every unified folder shows the inbox.
	folderRole := request.FolderRole
	if request.AccountID == "unified" {
		method = "messages.unifiedRecent"
		if strings.TrimSpace(folderRole) == "" {
			folderRole = request.FolderID
		}
	}
	limit := request.Limit
	if limit <= 0 {
		limit = 50
	}
	res, err := a.sidecar.Call(method, map[string]any{
		"account":       request.AccountID,
		"folder":        request.FolderID,
		"folder_role":   folderRole,
		"query":         request.Query,
		"filter":        request.Filter,
		"attachments":   request.Attachments,
		"group_by":      request.GroupBy,
		"before_cursor": request.BeforeCursor,
		"limit":         limit,
		"refresh":       request.Refresh,
		// Thread grouping (subject branching, root titles, unread counts)
		// runs in the core, shared with mobile; the bridge only mints ids.
		"group": true,
	})
	if err != nil {
		return nil, err
	}
	// RSS threads arrive already in final bridge shape; mail returns raw rows.
	if isRSSAccountID(request.AccountID) {
		return res, nil
	}
	out := threadsJSON(request.AccountID, request.FolderID, res)
	if request.Filter == "starred" {
		rawCount := 0
		if obj, ok := res.(map[string]any); ok {
			if list, ok := obj["cards"].([]any); ok {
				rawCount = len(list)
			}
		}
		threadCount := 0
		if obj, ok := out.(map[string]any); ok {
			if list, ok := obj["threads"].([]Message); ok {
				threadCount = len(list)
			} else if list, ok := obj["threads"].([]any); ok {
				threadCount = len(list)
			}
		}
		a.logf("threadList starred: raw_messages=%d grouped_threads=%d", rawCount, threadCount)
	}
	return out, nil
}

// starredItems passes through the core's final cross-account item model. The
// bridge only decodes it into its typed transport struct.
func (a *App) starredItems(payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return map[string]any{"items": []Message{}}, nil
	}
	res, err := a.sidecar.Call("starred.items", payload)
	if err != nil {
		return nil, err
	}
	object, _ := res.(map[string]any)
	rows, _ := object["items"].([]any)
	encoded, _ := json.Marshal(rows)
	items := make([]Message, 0, len(rows))
	_ = json.Unmarshal(encoded, &items)
	out := map[string]any{"items": items}
	if cursor, _ := object["next_cursor"].(string); cursor != "" {
		out["next_cursor"] = cursor
	}
	return out, nil
}

func (a *App) threadRead(payload map[string]any) (any, error) {
	threadID, _ := payload["thread_id"].(string)
	if a.sidecar == nil || !a.sidecar.Started() {
		return map[string]any{"messages": []Message{}}, nil
	}
	// Pagination is opt-in from the frontend: when `limit` is omitted the
	// engine returns the full thread (used by markRead's scan path).
	limit, hasLimit := payload["limit"]
	beforeCursor, _ := payload["before_cursor"].(string)
	if _, _, ok := parseRSSThreadID(threadID); ok {
		params := map[string]any{"thread_id": threadID}
		if hasLimit {
			params["limit"] = limit
		}
		if beforeCursor != "" {
			params["before_cursor"] = beforeCursor
		}
		res, err := a.sidecar.Call("rss.thread", params)
		if err != nil {
			return nil, err
		}
		// Already final bridge-shaped Message JSON.
		return res, nil
	}
	if ids, ok := parseImapThreadID(threadID); ok {
		method := "messages.thread"
		// The sidecar splits branch-compound keys and scopes the thread to the
		// subject branch itself; the bridge passes the key through untouched.
		params := map[string]any{"account": ids.Account, "folder": ids.Folder, "thread_key": ids.ThreadKey}
		if ids.ThreadKey == "" && ids.UID > 0 {
			method = "messages.read"
			params = map[string]any{"account": ids.Account, "folder": ids.Folder, "uid": ids.UID}
		}
		if method == "messages.thread" {
			// The sidecar shapes final bridge-ready message JSON and needs the
			// frontend's exact thread id for the messages' id/thread_id fields.
			params["thread_id"] = threadID
			if forPrint, _ := payload["for_print"].(bool); forPrint {
				params["for_print"] = true
			}
			if hasLimit {
				params["limit"] = limit
			}
			if beforeCursor != "" {
				params["before_cursor"] = beforeCursor
			}
		}
		res, err := a.sidecar.Call(method, params)
		if err != nil {
			return nil, err
		}
		if method == "messages.thread" {
			// Already final bridge-shaped Message JSON (same contract as
			// rss.thread above).
			return res, nil
		}
		return messageJSON(ids.Account, threadID, ids.Folder, res), nil
	}
	return map[string]any{"messages": []Message{}}, nil
}
