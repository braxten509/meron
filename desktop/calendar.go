// Routes desktop calendar requests to the mail core.
// The Wails command handler passes the requested calendar operation and its payload.
// The core supplies the event data returned to the calendar view.

package main

// calendarInvoke forwards one calendar command to the core unchanged, like
// tasksInvoke: the core's method names are the frontend's.
func (a *App) calendarInvoke(command string, payload map[string]any) (any, error) {
	if a.sidecar == nil || !a.sidecar.Started() {
		return nil, a.engineUnavailable()
	}
	return a.sidecar.Call(command, payload)
}
