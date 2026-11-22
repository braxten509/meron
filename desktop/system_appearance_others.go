//go:build !linux && !darwin && !windows

// Completes appearance initialization on platforms without a
// native color-scheme source. The desktop frontend can then
// resolve its theme from its own prefers-color-scheme query.

package main

// No system appearance source on this platform; the frontend falls back to
// prefers-color-scheme.
func (a *App) setupAppearanceListener() { systemAppearanceSettled() }

func (a *App) closeAppearanceListener() {}
