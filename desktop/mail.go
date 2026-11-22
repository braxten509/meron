// Desktop sync and search requests for mail and feed accounts.
// Incoming Wails payloads carry the selected mailbox and account.
// The host asks the sidecar to sync a folder, start watching it, or interpret a search.

package main

func (a *App) mailSync(payload map[string]any) (any, error) {
	accountID, _ := payload["account_id"].(string)
	folder, _ := payload["folder"].(string)
	if folder == "" {
		folder = "inbox"
	}
	engine := "meron_mail"
	params := map[string]any{"account": accountID, "folder": folder, "limit": 250}
	if isRSSAccountID(accountID) {
		engine = "rss"
		delete(params, "folder")
	}
	online := false
	if a.sidecar != nil && a.sidecar.Started() {
		// The sidecar routes messages.sync to the right engine by account.
		if _, err := a.sidecar.Call("messages.sync", params); err == nil {
			online = true
		}
	}
	return map[string]any{"ok": true, "synced_at": "now", "engine": engine, "online": online}, nil
}

func (a *App) watchFolder(command string, payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return map[string]any{"ok": true, "online": false}, nil
	}
	return a.sidecar.Call(command, payload)
}

// searchQuery converts between a search query's text and the search options
// form's fields. The parser lives in the shared core so every platform reads
// operators the same way.
func (a *App) searchQuery(command string, payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	return a.sidecar.Call(command, payload)
}
