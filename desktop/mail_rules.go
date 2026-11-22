// Bridge commands for rules, inbox sorting and Meron folders.
// The host maps frontend action names to their core command names.
// The shared engine owns rule execution, progress and folder membership.

package main

import (
	"fmt"
)

// rules lists what to do with new mail.
func (a *App) rules() (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return map[string]any{"rules": []any{}}, nil
	}
	return a.sidecar.Call("rules.list", map[string]any{})
}

// setRules replaces those rules, answering with what the core kept.
func (a *App) setRules(payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, fmt.Errorf("mail core is not running")
	}
	return a.sidecar.Call("rules.set", map[string]any{"rules": payload["rules"]})
}

// ruleMatches counts the mail already in the Inbox that a saved rule would run on.
func (a *App) ruleMatches(payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, fmt.Errorf("mail core is not running")
	}
	return a.sidecar.Call("rules.matches", map[string]any{"id": payload["id"]})
}

// runRule runs a saved rule over the mail already in the Inbox.
func (a *App) runRule(payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, fmt.Errorf("mail core is not running")
	}
	return a.sidecar.Call("rules.run", map[string]any{"id": payload["id"]})
}

// ruleStatus answers with what rules did lately and what runs by hand have left.
func (a *App) ruleStatus() (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return map[string]any{"log": []any{}, "running": map[string]any{}}, nil
	}
	return a.sidecar.Call("rules.status", map[string]any{})
}

// stopRule drops what is left of a rule run by hand.
func (a *App) stopRule(payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, fmt.Errorf("mail core is not running")
	}
	return a.sidecar.Call("rules.stop", map[string]any{"id": payload["id"]})
}

// aiSortMethods maps the UI's AI sorting commands to the core's aiSort.* methods.
// The payload goes through as it is: the core checks and cleans what it keeps.
var aiSortMethods = map[string]string{
	"mail.aiSortConfig":    "aiSort.get",
	"mail.setAiSortConfig": "aiSort.set",
	"mail.setAiSortKey":    "aiSort.setKey",
	"mail.checkAiSortKey":  "aiSort.checkKey",
	"mail.aiSortModels":    "aiSort.models",
	"mail.aiSortEstimate":  "aiSort.estimate",
	"mail.runAiSort":       "aiSort.run",
	"mail.stopAiSort":      "aiSort.stop",
	"mail.aiSortStatus":    "aiSort.status",
	"mail.aiSortMoveBack":  "aiSort.moveBack",
	// What the reader taught AI sorting by moving mail by hand.
	"mail.aiSortLessons":      "aiSort.lessons",
	"mail.aiSortForgetLesson": "aiSort.forgetLesson",
}

// aiSort passes one AI sorting command to the core. Nothing here logs the
// payload, which for aiSort.setKey holds the API key.
func (a *App) aiSort(command string, payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, fmt.Errorf("mail core is not running")
	}
	return a.sidecar.Call(aiSortMethods[command], payload)
}

// meronFolderMethods maps the UI's Meron folder commands (folders that exist
// only in Meron, on this device) to the core's meronFolders.* methods.
var meronFolderMethods = map[string]string{
	"mail.meronFolders":         "meronFolders.list",
	"mail.createMeronFolder":    "meronFolders.create",
	"mail.renameMeronFolder":    "meronFolders.rename",
	"mail.setMeronFolderNotify": "meronFolders.setNotify",
	"mail.deleteMeronFolder":    "meronFolders.delete",
	"mail.moveToMeronFolder":    "meronFolders.moveIn",
	"mail.moveOutOfMeronFolder": "meronFolders.moveOut",
	"mail.meronFolderThreads":   "meronFolders.threads",
	"mail.markMeronFolderRead":  "meronFolders.markAllRead",
}

// meronFolders passes one Meron folder command to the core. A folder's page
// of cards is decoded into Message like every other thread list, so the
// typed transport keeps meron_folder_id and drops nothing else it knows.
func (a *App) meronFolders(command string, payload map[string]any) (any, error) {
	// An error, not an empty list, while the core is down: the UI must not
	// take "not started yet" for "every Meron folder is gone".
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	res, err := a.sidecar.Call(meronFolderMethods[command], payload)
	if err != nil || command != "mail.meronFolderThreads" {
		return res, err
	}
	return threadsJSON("", "", res), nil
}
