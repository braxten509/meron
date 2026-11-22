//go:build linux && !bindings

// Applies desktop theme colors to GTK window decoration.
// The native titlebar and its tiled appearance use
// CSS derived from the current frontend palette.

package main

/*
#cgo pkg-config: gtk+-3.0

#include <stdlib.h>
#include <gtk/gtk.h>

static gboolean applyPreferDarkTheme(gpointer data) {
	GtkSettings *settings = gtk_settings_get_default();
	if (settings != NULL) {
		g_object_set(settings, "gtk-application-prefer-dark-theme", data != NULL ? TRUE : FALSE, NULL);
	}
	return G_SOURCE_REMOVE;
}

static void setPreferDarkTheme(int dark) {
	g_idle_add(applyPreferDarkTheme, dark ? GINT_TO_POINTER(1) : NULL);
}

static GtkCssProvider *titlebarProvider = NULL;

// Takes ownership of css (a g_strdup'd string).
static gboolean applyTitlebarCss(gpointer data) {
	gchar *css = data;
	GdkScreen *screen = gdk_screen_get_default();
	if (screen != NULL) {
		if (titlebarProvider == NULL) {
			titlebarProvider = gtk_css_provider_new();
			gtk_style_context_add_provider_for_screen(screen, GTK_STYLE_PROVIDER(titlebarProvider),
				GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
		}
		gtk_css_provider_load_from_data(titlebarProvider, css, -1, NULL);
	}
	g_free(css);
	return G_SOURCE_REMOVE;
}

static void setTitlebarCss(const char *css) {
	g_idle_add(applyTitlebarCss, g_strdup(css));
}
*/
import "C"

import (
	"fmt"
	"sync"
	"unsafe"
)

// setNativeWindowDark switches GTK to the dark variant of the current theme.
//
// That is what decorates the title bar: with client-side decorations GTK draws
// the header bar itself, and under a window manager GTK reflects the preference
// onto the toplevel's _GTK_THEME_VARIANT so the WM picks its dark frame.
//
// The property is set from a GTK idle callback because invoke() runs on a
// request goroutine, not the thread owning the GTK main loop.
func setNativeWindowDark(dark bool) {
	value := C.int(0)
	if dark {
		value = 1
	}
	C.setPreferDarkTheme(value)
}

// floatingDecoration selects the frame GTK draws around a floating window;
// maximised, fullscreen and tiled windows keep GTK's square frame.
const floatingDecoration = `window.csd:not(.maximized):not(.fullscreen):not(.tiled):not(.tiled-top):not(.tiled-bottom):not(.tiled-left):not(.tiled-right) decoration`

// decorationCss rounds all four corners of that frame (outline and shadow); stock GTK 3 rounds only
// the top two. The webview rounds its own corners to match (html.window-rounded). With the integrated
// title bar the window is libadwaita's: 15px corners and its shadow (libadwaita 1.8 default.css,
// window.csd and window.csd:backdrop).
func decorationCss(fg string) string {
	if !integratedTitlebarActive() {
		return floatingDecoration + " {\n\tborder-radius: 8px;\n}\n"
	}
	return floatingDecoration + ` {
	border-radius: 15px;
	box-shadow: 0 0 14px 5px rgba(0, 0, 0, 0.15), 0 0 5px 2px rgba(0, 0, 0, 0.1), 0 0 0 1px rgba(0, 0, 0, 0.05);
}
` + floatingDecoration + `:backdrop {
	box-shadow: 0 0 14px 5px transparent, 0 0 10px 5px rgba(0, 0, 0, 0.08), 0 0 0 1px rgba(0, 0, 0, 0.05);
}
` + tiledDecorationCss(fg)
}

// tiledDecorationCss: a tiled window's edge is libadwaita's 1px line in the
// window's text color at 15% (window.csd.tiled, --border-opacity), where GTK 3
// draws black at 75%, a hard dark line down the middle of a split screen.
func tiledDecorationCss(fg string) string {
	line := "rgba(255, 255, 255, 0.15)"
	if opaqueHexColor.MatchString(fg) {
		line = fmt.Sprintf("alpha(%s, 0.15)", fg)
	}
	return fmt.Sprintf(`.tiled decoration, .tiled-top decoration, .tiled-right decoration, .tiled-bottom decoration, .tiled-left decoration,
.tiled decoration:backdrop, .tiled-top decoration:backdrop, .tiled-right decoration:backdrop, .tiled-bottom decoration:backdrop, .tiled-left decoration:backdrop {
	box-shadow: 0 0 0 1px %s, 0 0 0 20px transparent;
}
`, line)
}

var (
	titlebarColorsMu sync.Mutex
	titlebarBg       string
	titlebarFg       string
)

// refreshNativeTitlebarCss re-applies the title bar CSS after the title bar
// mode changes, keeping the last colors.
func refreshNativeTitlebarCss() {
	titlebarColorsMu.Lock()
	bg, fg := titlebarBg, titlebarFg
	titlebarColorsMu.Unlock()
	setNativeTitlebarColors(bg, fg)
}

// setNativeTitlebarColors paints the title bar GTK draws itself (client-side decorations, e.g.
//
// GNOME on Wayland) in the side nav's colors, so the window top reads as one piece with the rail.
// Server-side frames drawn by the window manager ignore it and keep following setNativeWindowDark.
// Anything but an opaque #rrggbb pair clears the color override (the values are spliced into CSS); the
// rounded corners stay.
func setNativeTitlebarColors(bg, fg string) {
	titlebarColorsMu.Lock()
	titlebarBg, titlebarFg = bg, fg
	titlebarColorsMu.Unlock()
	css := decorationCss(fg)
	if opaqueHexColor.MatchString(bg) && opaqueHexColor.MatchString(fg) {
		css += fmt.Sprintf(`.titlebar.default-decoration, .titlebar.default-decoration:backdrop {
	background: %[1]s;
	color: %[2]s;
	border-color: %[1]s;
	box-shadow: none;
}
.titlebar.default-decoration button.titlebutton,
.titlebar.default-decoration:backdrop button.titlebutton {
	color: %[2]s;
}`, bg, fg)
	}
	cs := C.CString(css)
	defer C.free(unsafe.Pointer(cs))
	C.setTitlebarCss(cs)
}
