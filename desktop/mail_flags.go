// Read state, stars and tags changed from the desktop mailbox.
// Requests identify the affected message or conversation before reaching the core.
// Folder-wide read operations and tag management share the same host interface.

package main

import (
	"fmt"
)

func (a *App) markRead(payload map[string]any) (any, error) {
	threadID, _ := payload["thread_id"].(string)
	if threadID == "" {
		return map[string]any{"ok": true}, nil
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	// Defaults to read; pass seen:false to mark a thread unread.
	seen := true
	if v, ok := payload["seen"].(bool); ok {
		seen = v
	}
	if _, _, ok := parseRSSThreadID(threadID); ok {
		if itemKeys := rssItemKeysFromPayload(threadID, payload); len(itemKeys) > 0 {
			return a.sidecar.Call("rss.markRead", map[string]any{"thread_id": threadID, "item_keys": itemKeys, "seen": seen})
		}
		return a.sidecar.Call("rss.markRead", map[string]any{"thread_id": threadID, "seen": seen})
	}
	if ids, ok := parseImapThreadID(threadID); ok {
		operationFolder := deleteFolder(payload, ids.Folder)
		// Branch-compound thread keys pass through untouched; the sidecar
		// splits them and marks only the subject branch (both on the server
		// and in the local store).
		params := map[string]any{"account": ids.Account, "folder": operationFolder, "thread_key": ids.ThreadKey, "seen": seen}
		if uids := imapUIDsFromPayload(threadID, payload); len(uids) > 0 {
			params["uids"] = uids
			delete(params, "thread_key")
			return a.sidecar.Call("messages.markRead", params)
		}
		if ids.ThreadKey == "" && ids.UID > 0 {
			params = map[string]any{"account": ids.Account, "folder": operationFolder, "uid": ids.UID, "seen": seen}
		}
		return a.sidecar.Call("messages.markRead", params)
	}
	return map[string]any{"ok": true}, nil
}

func (a *App) markStarred(payload map[string]any) (any, error) {
	threadID, _ := payload["thread_id"].(string)
	if threadID == "" {
		return map[string]any{"ok": true}, nil
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return map[string]any{"ok": true}, nil
	}
	starred := true
	if v, ok := payload["starred"].(bool); ok {
		starred = v
	}
	if _, _, ok := parseRSSThreadID(threadID); ok {
		if itemKeys := rssItemKeysFromPayload(threadID, payload); len(itemKeys) > 0 {
			return a.sidecar.Call("rss.markStarred", map[string]any{"thread_id": threadID, "item_keys": itemKeys, "starred": starred})
		}
		return a.sidecar.Call("rss.markStarred", map[string]any{"thread_id": threadID, "starred": starred})
	}
	if ids, ok := parseImapThreadID(threadID); ok {
		operationFolder := deleteFolder(payload, ids.Folder)
		// Branch-compound thread keys pass through untouched; the sidecar
		// splits them and stars only the subject branch (both on the server
		// and in the local store).
		params := map[string]any{"account": ids.Account, "folder": operationFolder, "thread_key": ids.ThreadKey, "starred": starred}
		if uids := imapUIDsFromPayload(threadID, payload); len(uids) > 0 {
			params["uids"] = uids
			delete(params, "thread_key")
			return a.sidecar.Call("messages.markStarred", params)
		}
		if ids.ThreadKey == "" && ids.UID > 0 {
			params = map[string]any{"account": ids.Account, "folder": operationFolder, "uid": ids.UID, "starred": starred}
		}
		return a.sidecar.Call("messages.markStarred", params)
	}
	return map[string]any{"ok": true}, nil
}

// setTags adds and removes tags on a thread and answers with the tags it now
// carries. Tags are saved locally, so this never waits for the server.
func (a *App) setTags(payload map[string]any) (any, error) {
	threadID, _ := payload["thread_id"].(string)
	ids, ok := parseImapThreadID(threadID)
	if !ok {
		return nil, fmt.Errorf("not a mail thread: %s", threadID)
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, fmt.Errorf("mail core is not running")
	}
	params := map[string]any{
		"account":    ids.Account,
		"folder":     deleteFolder(payload, ids.Folder),
		"thread_key": ids.ThreadKey,
		"add":        payload["add"],
		"remove":     payload["remove"],
	}
	if ids.ThreadKey == "" && ids.UID > 0 {
		delete(params, "thread_key")
		params["uid"] = ids.UID
	}
	return a.sidecar.Call("messages.setTags", params)
}

// tagList lists every tag in use, for pickers.
func (a *App) tagList() (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return map[string]any{"tags": []any{}}, nil
	}
	return a.sidecar.Call("tags.list", map[string]any{})
}

// replaceTag renames a tag everywhere, or deletes it when "to" is left out.
func (a *App) replaceTag(payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, fmt.Errorf("mail core is not running")
	}
	return a.sidecar.Call("tags.replace", map[string]any{"tag": payload["tag"], "to": payload["to"], "queries": payload["queries"]})
}

// markAllRead marks every message in one mail folder as read. An RSS account has
// one synthetic Inbox, so its account-level operation marks every feed item read.
func (a *App) markAllRead(payload map[string]any) (any, error) {
	accountID, _ := payload["account_id"].(string)
	folderID, _ := payload["folder_id"].(string)
	if folderID == "" {
		folderID = "inbox"
	}
	if a.sidecar == nil || !a.sidecar.Started() || accountID == "" {
		return map[string]any{"ok": true}, nil
	}
	if isRSSAccountID(accountID) {
		return a.sidecar.Call("rss.markAllRead", map[string]any{"account": accountID})
	}
	if accountID == "unified" {
		return a.sidecar.Call("messages.markAllReadUnified", map[string]any{"folder": folderID})
	}
	return a.sidecar.Call("messages.markAllRead", map[string]any{
		"account": accountID,
		"folder":  folderID,
	})
}
