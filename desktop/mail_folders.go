// Folder management for the desktop account sidebar.
// These handlers validate create, rename, delete and notification requests.
// Folder listings and mutation results are returned in the desktop wire format.

package main

import (
	"errors"
	"strings"
)

func (a *App) folderList(payload map[string]any) (any, error) {
	var request FolderListRequest
	_ = decode(payload, &request)
	if request.AccountID == "" {
		request.AccountID, _ = payload["account_id"].(string)
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return map[string]any{"folders": []Folder{}}, nil
	}
	res, err := a.sidecar.Call("folders.list", map[string]any{"account": request.AccountID, "refresh": request.Refresh})
	if err != nil {
		return nil, err
	}
	// RSS folders arrive already in final bridge shape; mail returns raw rows.
	if isRSSAccountID(request.AccountID) {
		return res, nil
	}
	return foldersJSON(request.AccountID, res), nil
}

func (a *App) folderCreate(payload map[string]any) (any, error) {
	var request FolderCreateRequest
	_ = decode(payload, &request)
	if request.AccountID == "" {
		request.AccountID, _ = payload["account_id"].(string)
	}
	if request.Name == "" {
		request.Name, _ = payload["name"].(string)
	}
	if request.AccountID == "" {
		return nil, errors.New("account_id is required")
	}
	if strings.TrimSpace(request.Name) == "" {
		return nil, errors.New("Folder name is required")
	}
	if isRSSAccountID(request.AccountID) {
		return nil, errors.New("RSS accounts do not support folders")
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	res, err := a.sidecar.Call("folders.create", map[string]any{"account": request.AccountID, "name": request.Name})
	if err != nil {
		return nil, err
	}
	return foldersJSON(request.AccountID, res), nil
}

// folderSetNotify opts a folder in to (or out of) live sync and new-mail
// notifications. The sidecar keeps the flag and the folder's IDLE watch.
func (a *App) folderSetNotify(payload map[string]any) (any, error) {
	var request FolderSetNotifyRequest
	_ = decode(payload, &request)
	if request.AccountID == "" {
		return nil, errors.New("account_id is required")
	}
	if strings.TrimSpace(request.FolderID) == "" {
		return nil, errors.New("folder_id is required")
	}
	if isRSSAccountID(request.AccountID) {
		return nil, errors.New("RSS accounts do not support folders")
	}
	if request.AccountID == "unified" {
		return nil, errors.New("Pick a single account to change folder notifications")
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	res, err := a.sidecar.Call("folders.setNotify", map[string]any{
		"account": request.AccountID,
		"folder":  request.FolderID,
		"enabled": request.Enabled,
	})
	if err != nil {
		return nil, err
	}
	return foldersJSON(request.AccountID, res), nil
}

// folderDelete removes a folder on the server, subfolders included, along with
// the cached messages under it. The sidecar re-checks that no special-use folder
// is in that subtree, so a bad payload cannot delete Sent or Archive.
func (a *App) folderDelete(payload map[string]any) (any, error) {
	var request FolderDeleteRequest
	_ = decode(payload, &request)
	if request.AccountID == "" {
		request.AccountID, _ = payload["account_id"].(string)
	}
	if request.FolderID == "" {
		request.FolderID, _ = payload["folder_id"].(string)
	}
	if request.AccountID == "" {
		return nil, errors.New("account_id is required")
	}
	if strings.TrimSpace(request.FolderID) == "" {
		return nil, errors.New("folder_id is required")
	}
	if isRSSAccountID(request.AccountID) {
		return nil, errors.New("RSS accounts do not support folders")
	}
	if request.AccountID == "unified" {
		return nil, errors.New("Pick a single account to delete a folder")
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	res, err := a.sidecar.Call("folders.delete", map[string]any{"account": request.AccountID, "folder": request.FolderID})
	if err != nil {
		return nil, err
	}
	out := map[string]any{"ok": true, "folder": request.FolderID}
	if m, isMap := res.(map[string]any); isMap {
		if ok, found := m["ok"]; found {
			out["ok"] = ok
		}
		if deleted, found := m["deleted"]; found {
			out["deleted"] = deleted
		}
		if warning, found := m["warning"]; found {
			out["warning"] = warning
		}
		// The whole subtree that went with the folder, so the frontend can
		// clear the views and caches keyed on a nested folder as well.
		if removed, found := m["removed"]; found {
			out["removed"] = removed
		}
		// Same reshaping as folderList: the sidecar returns raw folder rows.
		if shaped, isShaped := foldersJSON(request.AccountID, res).(map[string]any); isShaped {
			out["folders"] = shaped["folders"]
		}
	}
	return out, nil
}

// folderRename renames a folder, subfolders included: on the server, or in
// local folders. name is the folder's new full path. The sidecar refuses
// special-use folders, like a delete.
func (a *App) folderRename(payload map[string]any) (any, error) {
	var request FolderRenameRequest
	_ = decode(payload, &request)
	if request.AccountID == "" {
		return nil, errors.New("account_id is required")
	}
	if strings.TrimSpace(request.FolderID) == "" {
		return nil, errors.New("folder_id is required")
	}
	if strings.TrimSpace(request.Name) == "" {
		return nil, errors.New("Folder name is required")
	}
	if isRSSAccountID(request.AccountID) {
		return nil, errors.New("RSS accounts do not support folders")
	}
	if request.AccountID == "unified" {
		return nil, errors.New("Pick a single account to rename a folder")
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	res, err := a.sidecar.Call("folders.rename", map[string]any{"account": request.AccountID, "folder": request.FolderID, "name": request.Name})
	if err != nil {
		return nil, err
	}
	out := map[string]any{"ok": true, "folder": request.FolderID}
	if m, isMap := res.(map[string]any); isMap {
		// Every folder that moved, as [old, new] pairs, so the frontend can
		// repoint the views and caches keyed on a nested folder as well.
		if renamed, found := m["renamed"]; found {
			out["renamed"] = renamed
		}
		if shaped, isShaped := foldersJSON(request.AccountID, res).(map[string]any); isShaped {
			out["folders"] = shaped["folders"]
		}
	}
	return out, nil
}

// emptyFolder permanently deletes every message in one account's Trash or Junk
// folder. The sidecar re-checks the folder role, so a bad payload cannot empty
// anything else. RSS accounts and the unified view have no folder to empty.
func (a *App) emptyFolder(payload map[string]any) (any, error) {
	accountID, _ := payload["account_id"].(string)
	folderID, _ := payload["folder_id"].(string)
	if a.sidecar == nil || !a.sidecar.Started() || accountID == "" || folderID == "" {
		return map[string]any{"ok": true}, nil
	}
	if isRSSAccountID(accountID) || accountID == "unified" {
		return map[string]any{"ok": true}, nil
	}
	return a.sidecar.Call("messages.emptyFolder", map[string]any{
		"account": accountID,
		"folder":  folderID,
	})
}
