//go:build !linux

// Adapts the desktop file picker to platforms outside Linux.
// The shared picker calls these open and save entry points.
// Wails displays the platform dialog and returns the selected paths.

package main

import (
	"context"

	wailsRuntime "github.com/wailsapp/wails/v2/pkg/runtime"
)

func openFilePicker(
	requestContext context.Context,
	title string,
	defaultDirectory string,
	imagesOnly bool,
	multiple bool,
) ([]string, bool, error) {
	paths, err := openWailsFilePicker(requestContext, title, defaultDirectory, imagesOnly, multiple)
	return paths, false, err
}

func saveFileDialog(requestContext context.Context, options wailsRuntime.SaveDialogOptions) (string, error) {
	return wailsRuntime.SaveFileDialog(requestContext, options)
}

func openFileDialog(requestContext context.Context, options wailsRuntime.OpenDialogOptions) (string, error) {
	return wailsRuntime.OpenFileDialog(requestContext, options)
}
