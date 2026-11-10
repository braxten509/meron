package main

import "fmt"

// setupCopy passes a "Copy setup to another device" command to the core
// under the same name. The core does the networking (it listens on this
// computer's network address for the phone); the screen polls
// setupCopy.status.
func (a *App) setupCopy(command string, payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, fmt.Errorf("mail core is not running")
	}
	res, err := a.sidecar.Call(command, payload)
	if err == nil {
		if m, ok := res.(map[string]any); ok {
			if replaced, _ := m["replaced"].(bool); replaced && command == "setupCopy.restoreSafety" {
				a.requestNotificationReconcile()
			}
		}
	}
	return res, err
}
