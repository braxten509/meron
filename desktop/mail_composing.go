// Sending mail and updating server drafts from the desktop composer.
// Wails requests carry the identity and draft data chosen by the frontend.
// The host forwards these values to the shared mail engine for delivery or storage.

package main

import (
	"errors"
	"fmt"
)

func (a *App) mailSend(payload map[string]any) (any, error) {
	var request SendMailRequest
	if err := decode(payload, &request); err != nil {
		return nil, err
	}
	if request.To == "" {
		return nil, errors.New("invalid message")
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	if _, err := a.sidecar.Call("send", map[string]any{
		"account":     request.AccountID,
		"to":          request.To,
		"cc":          request.Cc,
		"bcc":         request.Bcc,
		"subject":     request.Subject,
		"body":        request.Body,
		"html":        request.Html,
		"in_reply_to": request.InReplyTo,
		"references":  request.References,
		"reply_to":    request.ReplyTo,
		"from":        request.From,
		"message_id":  request.MessageID,
		"attachments": request.Attachments,
	}); err != nil {
		return nil, err
	}
	return map[string]any{"ok": true, "queued": false}, nil
}

func (a *App) mailSaveDraft(payload map[string]any) (any, error) {
	var request SendMailRequest
	if err := decode(payload, &request); err != nil {
		return nil, err
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	if _, err := a.sidecar.Call("save_draft", map[string]any{
		"account":     request.AccountID,
		"to":          request.To,
		"cc":          request.Cc,
		"bcc":         request.Bcc,
		"subject":     request.Subject,
		"body":        request.Body,
		"html":        request.Html,
		"in_reply_to": request.InReplyTo,
		"references":  request.References,
		"reply_to":    request.ReplyTo,
		"from":        request.From,
		"draft_id":    request.DraftID,
		"attachments": request.Attachments,
	}); err != nil {
		return nil, err
	}
	return map[string]any{"ok": true}, nil
}

func (a *App) mailDiscardDraft(payload map[string]any) (any, error) {
	accountID, _ := payload["account_id"].(string)
	draftID, _ := payload["draft_id"].(string)
	if accountID == "" || draftID == "" {
		return map[string]any{"ok": true, "deleted": 0}, nil
	}
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	params := map[string]any{
		"account":  accountID,
		"draft_id": draftID,
	}
	if threadID, _ := payload["thread_id"].(string); threadID != "" {
		if ids, ok := parseImapThreadID(threadID); ok && ids.Account == accountID {
			params["thread_key"] = ids.ThreadKey
			if ids.UID > 0 && ids.ThreadKey == "" {
				params["thread_key"] = fmt.Sprintf("uid:%d", ids.UID)
			}
		}
	}
	return a.sidecar.Call("discard_draft", params)
}
