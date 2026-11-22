// Schedules desktop delivery of due task and calendar reminders.
// The running app checks the core for reminders ready to display.
// Delivery passes the results to the native notification layer.

package main

import (
	"encoding/json"
	"time"
)

const reminderNotificationAccount = "__meron_reminder__"

type reminder struct {
	Key   string `json:"key"`
	Kind  string `json:"kind"`
	Title string `json:"title"`
	Body  string `json:"body"`
	At    int64  `json:"at"`
}

// Runs while Meron is open, including when its window is hidden in the tray.
// The core keeps delivery receipts across restarts and revalidates on claim.
func (a *App) runReminders() {
	tick := time.NewTicker(30 * time.Second)
	defer tick.Stop()
	for {
		a.deliverDueReminders(time.Now().Unix())
		select {
		case <-a.ctx.Done():
			return
		case <-tick.C:
		}
	}
}

func (a *App) deliverDueReminders(now int64) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return
	}
	result, err := a.sidecar.Call("reminders.list", map[string]any{"due_only": true})
	if err != nil {
		a.logf("reminders: %v", err)
		return
	}
	data, err := json.Marshal(result)
	if err != nil {
		return
	}
	var plan struct {
		Reminders []reminder `json:"reminders"`
	}
	if json.Unmarshal(data, &plan) != nil {
		return
	}
	for _, r := range plan.Reminders {
		if r.At > now {
			break
		}
		result, err := a.sidecar.Call("reminders.claim", map[string]any{"key": r.Key})
		if err != nil {
			a.logf("claim reminder: %v", err)
			continue
		}
		data, err := json.Marshal(result)
		if err != nil {
			continue
		}
		var claim struct {
			Reminder *reminder `json:"reminder"`
		}
		if json.Unmarshal(data, &claim) != nil || claim.Reminder == nil {
			continue
		}
		r = *claim.Reminder
		labels := a.currentNativeLabels()
		body := labels.calendar
		if r.Kind == "tasks" {
			body = labels.tasks
		}
		if r.Body != "" {
			body += " · " + r.Body
		}
		a.deliverNotification(notification{title: r.Title, body: body, account: reminderNotificationAccount, threadID: r.Kind})
	}
}
