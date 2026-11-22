// Desktop commands for moving, copying, archiving and deleting mail.
// Single and bulk operations translate the same user selection into core requests.
// Explicit message identities and account boundaries determine the targets.

package main

import (
	"errors"
	"fmt"
	"strconv"
	"strings"
)

// coreAsk is what one mail.delete or mail.move asks the core: the params of
// its single command, and how the core's answer becomes the reply. A nil
// params means there is nothing to ask and answer is the reply already.
type coreAsk struct {
	params map[string]any
	finish func(res any) any
	answer any
}

func (a *App) mailDelete(payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		if threadID, _ := payload["thread_id"].(string); threadID == "" {
			return map[string]any{"ok": true}, nil
		}
		return nil, a.engineUnavailable()
	}
	ask, err := deleteAsk(payload)
	if err != nil {
		return nil, err
	}
	return a.callOne("messages.delete", ask)
}

func deleteAsk(payload map[string]any) (coreAsk, error) {
	threadID, _ := payload["thread_id"].(string)
	if threadID == "" {
		return coreAsk{answer: map[string]any{"ok": true}}, nil
	}
	if _, _, ok := parseRSSThreadID(threadID); ok {
		return coreAsk{}, errors.New("feed items cannot be moved to trash")
	}
	if ids, ok := parseImapThreadID(threadID); ok {
		sourceFolder := deleteFolder(payload, ids.Folder)
		// Branch-compound thread keys pass through untouched; the sidecar
		// splits them and scopes the delete to the subject branch.
		params := map[string]any{"account": ids.Account, "folder": sourceFolder, "thread_key": ids.ThreadKey}
		// The core then puts off re-reading Trash until a later delete.
		if more, _ := payload["more_coming"].(bool); more {
			params["more_coming"] = true
		}
		if uids := imapUIDsFromPayload(threadID, payload); len(uids) > 0 {
			params["uids"] = uids
			delete(params, "thread_key")
		} else if ids.ThreadKey == "" && ids.UID > 0 {
			delete(params, "thread_key")
			params["uid"] = ids.UID
		}
		return coreAsk{params: params, finish: func(res any) any {
			return withMovedThreadLocation(res, ids, "trash")
		}}, nil
	}
	return coreAsk{answer: map[string]any{"ok": true, "deleted": 0}}, nil
}

func (a *App) mailMove(payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		threadID, _ := payload["thread_id"].(string)
		targetFolder, _ := payload["target_folder_id"].(string)
		if threadID == "" || targetFolder == "" {
			return map[string]any{"ok": true}, nil
		}
		return nil, a.engineUnavailable()
	}
	ask, err := moveAsk(payload)
	if err != nil {
		return nil, err
	}
	return a.callOne("messages.move", ask)
}

func moveAsk(payload map[string]any) (coreAsk, error) {
	threadID, _ := payload["thread_id"].(string)
	targetFolder, _ := payload["target_folder_id"].(string)
	targetAccount, _ := payload["target_account_id"].(string)
	if threadID == "" || targetFolder == "" {
		return coreAsk{answer: map[string]any{"ok": true}}, nil
	}
	if _, _, ok := parseRSSThreadID(threadID); ok {
		return coreAsk{}, errors.New("feed items cannot be moved between folders")
	}
	if ids, ok := parseImapThreadID(threadID); ok {
		if (targetAccount == "" || targetAccount == ids.Account) && canonThreadFolder(ids.Folder) == canonThreadFolder(targetFolder) {
			return coreAsk{answer: map[string]any{"ok": true, "moved": 0}}, nil
		}
		// Branch-compound thread keys pass through untouched; the sidecar
		// splits them and scopes the move to the subject branch.
		params := map[string]any{"account": ids.Account, "folder": ids.Folder, "target_folder": targetFolder, "thread_key": ids.ThreadKey}
		if targetAccount != "" {
			params["target_account"] = targetAccount
		}
		// An Undo says what it puts back besides the mail (the Inbox view,
		// a Meron folder); the core applies it after the move. more_coming
		// has the core put off re-reading the target until a later move.
		for _, key := range []string{"undo", "show_in_inbox", "meron_folder_id", "meron_source", "more_coming"} {
			if value, ok := payload[key]; ok && value != nil {
				params[key] = value
			}
		}
		if uids := imapUIDsFromPayload(threadID, payload); len(uids) > 0 {
			params["uids"] = uids
			delete(params, "thread_key")
		} else if ids.ThreadKey == "" && ids.UID > 0 {
			delete(params, "thread_key")
			params["uid"] = ids.UID
		}
		return coreAsk{params: params}, nil
	}
	return coreAsk{answer: map[string]any{"ok": true}}, nil
}

// callOne sends one ask to the core as the single command `method`.
func (a *App) callOne(method string, ask coreAsk) (any, error) {
	if ask.params == nil {
		return ask.answer, nil
	}
	res, err := a.sidecar.Call(method, ask.params)
	if err != nil {
		return nil, err
	}
	if ask.finish != nil {
		return ask.finish(res), nil
	}
	return res, nil
}

// mailDeleteMany deletes several conversations as one bulk run: `items` holds
// one mail.delete payload each. The core does the server steps once for all
// of them. `results` answers each item in order: what mail.delete would have
// answered, or {"error": "..."} for one that failed.
func (a *App) mailDeleteMany(payload map[string]any) (any, error) {
	return a.callMany("messages.deleteMany", "messages.delete", payload, deleteAsk)
}

// mailMoveMany is mailDeleteMany for mail.move payloads.
func (a *App) mailMoveMany(payload map[string]any) (any, error) {
	return a.callMany("messages.moveMany", "messages.move", payload, moveAsk)
}

// callMany sends the asks of `items` to the core as one `method` command;
// `single` is the command each item would have been alone, for the wording of
// its error.
func (a *App) callMany(method, single string, payload map[string]any, toAsk func(map[string]any) (coreAsk, error)) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	items, _ := payload["items"].([]any)
	results := make([]any, len(items))
	asks := make([]coreAsk, len(items))
	var params []any
	var asked []int
	for i, item := range items {
		itemPayload, _ := item.(map[string]any)
		ask, err := toAsk(itemPayload)
		switch {
		case err != nil:
			results[i] = map[string]any{"error": err.Error()}
		case ask.params == nil:
			results[i] = ask.answer
		default:
			asks[i] = ask
			params = append(params, ask.params)
			asked = append(asked, i)
		}
	}
	if len(asked) == 0 {
		return map[string]any{"results": results}, nil
	}
	res, err := a.sidecar.Call(method, map[string]any{"items": params})
	if err != nil {
		return nil, err
	}
	obj, _ := res.(map[string]any)
	answers, _ := obj["results"].([]any)
	if len(answers) != len(asked) {
		return nil, fmt.Errorf("%s answered %d of %d items", method, len(answers), len(asked))
	}
	for n, i := range asked {
		answer := answers[n]
		if failed, _ := answer.(map[string]any); failed != nil && failed["error"] != nil && failed["ok"] == nil {
			results[i] = map[string]any{"error": fmt.Sprintf("sidecar %s error: %s", single, sidecarErrorMessage(failed["error"]))}
			continue
		}
		if asks[i].finish != nil {
			answer = asks[i].finish(answer)
		}
		results[i] = answer
	}
	return map[string]any{"results": results}, nil
}

func (a *App) mailCopy(payload map[string]any) (any, error) {
	threadID, _ := payload["thread_id"].(string)
	targetAccount, _ := payload["target_account_id"].(string)
	targetFolder, _ := payload["target_folder_id"].(string)
	if threadID == "" || targetAccount == "" || targetFolder == "" {
		return map[string]any{"ok": true}, nil
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	if _, _, ok := parseRSSThreadID(threadID); ok {
		return nil, errors.New("feed items cannot be copied between folders")
	}
	if ids, ok := parseImapThreadID(threadID); ok {
		// Branch-compound thread keys pass through untouched; the sidecar
		// splits them and scopes the copy to the subject branch.
		params := map[string]any{
			"account":        ids.Account,
			"folder":         ids.Folder,
			"target_account": targetAccount,
			"target_folder":  targetFolder,
			"thread_key":     ids.ThreadKey,
		}
		if uids := imapUIDsFromPayload(threadID, payload); len(uids) > 0 {
			params["uids"] = uids
			delete(params, "thread_key")
			return a.sidecar.Call("messages.copy", params)
		}
		if ids.ThreadKey == "" && ids.UID > 0 {
			params = map[string]any{
				"account":        ids.Account,
				"folder":         ids.Folder,
				"target_account": targetAccount,
				"target_folder":  targetFolder,
				"uid":            ids.UID,
			}
		}
		res, err := a.sidecar.Call("messages.copy", params)
		if err != nil {
			return nil, err
		}
		out, _ := res.(map[string]any)
		if out == nil {
			out = map[string]any{"ok": true}
		}
		if ids.ThreadKey != "" {
			out["target_thread_id"] = formatImapThreadID(targetAccount, targetFolder, ids.ThreadKey)
		}
		return out, nil
	}
	return map[string]any{"ok": true}, nil
}

func (a *App) mailArchive(payload map[string]any) (any, error) {
	threadID, _ := payload["thread_id"].(string)
	if threadID == "" {
		return map[string]any{"ok": true}, nil
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	if _, _, ok := parseRSSThreadID(threadID); ok {
		return nil, errors.New("feed items cannot be archived")
	}
	ids, ok := parseImapThreadID(threadID)
	if !ok {
		return map[string]any{"ok": true}, nil
	}
	res, err := a.sidecar.Call("folders.archive", map[string]any{"account": ids.Account, "allow_missing": true})
	if err != nil {
		return nil, err
	}
	obj, _ := res.(map[string]any)
	targetFolder, _ := obj["folder"].(string)
	if targetFolder == "" {
		// No archive mailbox: a whole conversation is archived Meron-only,
		// filed into the built-in Archive with no server move. A single
		// email keeps the "no archive folder" answer.
		if ids.ThreadKey == "" || len(imapUIDsFromPayload(threadID, payload)) > 0 {
			return nil, errors.New("Archive folder not found for this account")
		}
		filed, err := a.sidecar.Call("meronFolders.moveIn", map[string]any{
			"folder_id":  "mf_archive",
			"thread_ids": []string{threadID},
		})
		if err != nil {
			return nil, err
		}
		out, _ := filed.(map[string]any)
		if out == nil {
			out = map[string]any{"ok": true}
		}
		out["meron_only"] = true
		out["folder"] = ids.Folder
		out["thread_id"] = threadID
		return out, nil
	}
	payload["target_folder_id"] = targetFolder
	moveRes, err := a.mailMove(payload)
	if err != nil {
		return nil, err
	}
	out, _ := moveRes.(map[string]any)
	if out == nil {
		out = map[string]any{"ok": true}
	}
	out["folder"] = targetFolder
	out["thread_id"] = formatParsedImapThreadIDInFolder(ids, targetFolder)
	return out, nil
}

// deleteFolder lets delete callers override the thread id's nominal folder with
// the message/thread row's current folder. Thread ids can outlive a move, and a
// UID/thread key is only meaningful within its source mailbox.
func deleteFolder(payload map[string]any, fallback string) string {
	if folder, ok := payload["folder"].(string); ok && folder != "" {
		return folder
	}
	return fallback
}

func imapUIDsFromPayload(threadID string, payload map[string]any) []uint32 {
	raw, _ := payload["message_ids"].([]any)
	uids := make([]uint32, 0, len(raw))
	prefix := threadID + "#"
	for _, item := range raw {
		messageID, _ := item.(string)
		uidPart := messageID
		if rest, ok := strings.CutPrefix(messageID, prefix); ok {
			uidPart = rest
		}
		uid, err := strconv.ParseUint(uidPart, 10, 32)
		if err == nil {
			uids = append(uids, uint32(uid))
			continue
		}
		if ids, ok := parseImapThreadID(messageID); ok && ids.UID > 0 {
			uids = append(uids, ids.UID)
		}
	}
	return uids
}
