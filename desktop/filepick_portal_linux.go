//go:build linux

// Connects the Linux file picker to desktop portals and native dialogs.
// The selection request carries the starting directory and file options.
// The portal response is decoded into local paths for the composer.

package main

import (
	"context"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"

	"github.com/godbus/dbus/v5"
	"github.com/google/uuid"
	wailsRuntime "github.com/wailsapp/wails/v2/pkg/runtime"
)

const (
	portalDesktopName        = "org.freedesktop.portal.Desktop"
	portalDesktopPath        = dbus.ObjectPath("/org/freedesktop/portal/desktop")
	portalRequestNamespace   = dbus.ObjectPath("/org/freedesktop/portal/desktop/request")
	portalFileChooserOpen    = "org.freedesktop.portal.FileChooser.OpenFile"
	portalFileChooserSave    = "org.freedesktop.portal.FileChooser.SaveFile"
	portalRequestResponse    = "org.freedesktop.portal.Request.Response"
	portalRequestInterface   = "org.freedesktop.portal.Request"
	portalRequestCloseMethod = "org.freedesktop.portal.Request.Close"
)

type portalFilterRule struct {
	Kind    uint32
	Pattern string
}

type portalFilter struct {
	Name  string
	Rules []portalFilterRule
}

func openFilePicker(
	requestContext context.Context,
	title string,
	defaultDirectory string,
	imagesOnly bool,
	multiple bool,
) ([]string, bool, error) {
	if !useFileChooserPortal() {
		paths, operationError := openWailsFilePicker(requestContext, title, defaultDirectory, imagesOnly, multiple)
		return paths, false, operationError
	}

	paths, operationError := portalFileChooser(requestContext, portalFileChooserOpen, title, portalOpenFileOptions(imagesOnly, multiple))
	return paths, true, operationError
}

// useFileChooserPortal reports whether file dialogs must go through the
// xdg-desktop-portal. Wails builds its GTK dialog in-process, which inside the
// Flatpak sandbox can only browse the sandbox itself, while the portal shows
// the host filesystem and grants access to just the chosen file.
func useFileChooserPortal() bool {
	return os.Getenv("FLATPAK_ID") != ""
}

// saveFileDialog is wailsRuntime.SaveFileDialog, routed through the portal
// when sandboxed. It returns "" when the user cancels.
func saveFileDialog(requestContext context.Context, options wailsRuntime.SaveDialogOptions) (string, error) {
	if !useFileChooserPortal() {
		return wailsRuntime.SaveFileDialog(requestContext, options)
	}
	portalOptions := portalBaseOptions()
	if options.DefaultFilename != "" {
		portalOptions["current_name"] = dbus.MakeVariant(options.DefaultFilename)
	}
	addPortalFilters(portalOptions, options.Filters)
	return firstPortalPath(portalFileChooser(requestContext, portalFileChooserSave, options.Title, portalOptions))
}

// openFileDialog is wailsRuntime.OpenFileDialog, routed through the portal
// when sandboxed. It returns "" when the user cancels.
func openFileDialog(requestContext context.Context, options wailsRuntime.OpenDialogOptions) (string, error) {
	if !useFileChooserPortal() {
		return wailsRuntime.OpenFileDialog(requestContext, options)
	}
	portalOptions := portalBaseOptions()
	addPortalFilters(portalOptions, options.Filters)
	return firstPortalPath(portalFileChooser(requestContext, portalFileChooserOpen, options.Title, portalOptions))
}

func firstPortalPath(paths []string, operationError error) (string, error) {
	if operationError != nil || len(paths) == 0 {
		return "", operationError
	}
	return paths[0], nil
}

// portalFileChooser runs one FileChooser portal request and returns the
// chosen local paths, or none when the user cancels.
func portalFileChooser(requestContext context.Context, method, title string, options map[string]dbus.Variant) ([]string, error) {
	uris, operationError := portalFileChooserURIs(requestContext, method, title, options)
	if operationError != nil {
		return nil, operationError
	}
	return portalFilePaths(uris)
}

func portalFileChooserURIs(requestContext context.Context, method, title string, options map[string]dbus.Variant) ([]string, error) {
	conn, operationError := dbus.ConnectSessionBus()
	if operationError != nil {
		return nil, fmt.Errorf("connect to file chooser portal: %w", operationError)
	}
	defer conn.Close()

	signals := make(chan *dbus.Signal, 1)
	conn.Signal(signals)
	defer conn.RemoveSignal(signals)

	matchOptions := []dbus.MatchOption{
		dbus.WithMatchInterface(portalRequestInterface),
		dbus.WithMatchMember("Response"),
		dbus.WithMatchPathNamespace(portalRequestNamespace),
	}
	if operationError := conn.AddMatchSignal(matchOptions...); operationError != nil {
		return nil, fmt.Errorf("listen for file chooser response: %w", operationError)
	}
	defer conn.RemoveMatchSignal(matchOptions...)

	var handle dbus.ObjectPath
	call := conn.Object(portalDesktopName, portalDesktopPath).CallWithContext(
		requestContext,
		method,
		0,
		"",
		title,
		options,
	)
	if call.Err != nil {
		return nil, fmt.Errorf("open file chooser portal: %w", call.Err)
	}
	if operationError := call.Store(&handle); operationError != nil {
		return nil, fmt.Errorf("read file chooser handle: %w", operationError)
	}

	for {
		select {
		case <-requestContext.Done():
			conn.Object(portalDesktopName, handle).Go(portalRequestCloseMethod, 0, nil)
			return nil, requestContext.Err()
		case signal, ok := <-signals:
			if !ok {
				return nil, errors.New("file chooser portal closed unexpectedly")
			}
			if signal.Path != handle || signal.Name != portalRequestResponse {
				continue
			}
			return portalResponseURIs(signal.Body)
		}
	}
}

func portalBaseOptions() map[string]dbus.Variant {
	return map[string]dbus.Variant{
		"handle_token": dbus.MakeVariant("meron_" + strings.ReplaceAll(uuid.NewString(), "-", "_")),
		"modal":        dbus.MakeVariant(true),
	}
}

// addPortalFilters converts Wails filters ("*.opml;*.xml") into portal glob
// filters, preselecting the first one as Wails does.
func addPortalFilters(options map[string]dbus.Variant, filters []wailsRuntime.FileFilter) {
	converted := portalFilters(filters)
	if len(converted) == 0 {
		return
	}
	options["filters"] = dbus.MakeVariant(converted)
	options["current_filter"] = dbus.MakeVariant(converted[0])
}

func portalFilters(filters []wailsRuntime.FileFilter) []portalFilter {
	var converted []portalFilter
	for _, filter := range filters {
		var rules []portalFilterRule
		for _, pattern := range strings.Split(filter.Pattern, ";") {
			if pattern = strings.TrimSpace(pattern); pattern != "" {
				rules = append(rules, portalFilterRule{Kind: 0, Pattern: pattern})
			}
		}
		if len(rules) > 0 {
			converted = append(converted, portalFilter{Name: filter.DisplayName, Rules: rules})
		}
	}
	return converted
}

func portalOpenFileOptions(imagesOnly, multiple bool) map[string]dbus.Variant {
	options := portalBaseOptions()
	options["multiple"] = dbus.MakeVariant(multiple)
	if imagesOnly {
		options["filters"] = dbus.MakeVariant([]portalFilter{
			{
				Name: "Image files",
				Rules: []portalFilterRule{
					{Kind: 1, Pattern: "image/png"},
					{Kind: 1, Pattern: "image/jpeg"},
					{Kind: 1, Pattern: "image/gif"},
					{Kind: 1, Pattern: "image/webp"},
				},
			},
		})
	}
	return options
}

func portalResponseURIs(body []any) ([]string, error) {
	if len(body) != 2 {
		return nil, errors.New("invalid file chooser response")
	}
	response, ok := body[0].(uint32)
	if !ok {
		return nil, errors.New("invalid file chooser response code")
	}
	switch response {
	case 0:
		results, ok := body[1].(map[string]dbus.Variant)
		if !ok {
			return nil, errors.New("invalid file chooser results")
		}
		uris, ok := results["uris"]
		if !ok {
			return nil, errors.New("file chooser returned no files")
		}
		values, ok := uris.Value().([]string)
		if !ok {
			return nil, errors.New("invalid file chooser file list")
		}
		return values, nil
	case 1:
		return nil, nil
	default:
		return nil, errors.New("file chooser request failed")
	}
}

func portalFilePaths(uris []string) ([]string, error) {
	paths := make([]string, 0, len(uris))
	for _, raw := range uris {
		uri, operationError := url.Parse(raw)
		if operationError != nil {
			return nil, fmt.Errorf("parse selected file URI: %w", operationError)
		}
		if uri.Scheme != "file" || (uri.Host != "" && uri.Host != "localhost") {
			return nil, fmt.Errorf("unsupported selected file URI: %q", raw)
		}
		if uri.Path == "" {
			return nil, fmt.Errorf("selected file URI has no path: %q", raw)
		}
		paths = append(paths, filepath.FromSlash(uri.Path))
	}
	return paths, nil
}
