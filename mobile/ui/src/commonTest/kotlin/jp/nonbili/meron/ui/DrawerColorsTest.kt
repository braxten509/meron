// Checks contrast in the mobile mail drawer for each built-in
// theme. Text must remain legible on the drawer surface
// and unread counts must stand out on the accent color.

package jp.nonbili.meron.ui

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.graphics.luminance
import kotlin.test.Test
import kotlin.test.assertTrue

class DrawerColorsTest {
    private fun contrast(
        a: Color,
        b: Color,
    ): Float {
        val la = a.luminance()
        val lb = b.luminance()
        return (maxOf(la, lb) + 0.05f) / (minOf(la, lb) + 0.05f)
    }

    @Test
    fun drawerTextReadsOnEveryBuiltinDrawer() {
        for (mode in AppAppearanceMode.entries.filterNot { it.isDynamic }) {
            val colors = builtinChatColors(mode)
            assertTrue(contrast(colors.onSidebar, colors.sidebar) >= 4.5f, "$mode drawer text")
            assertTrue(contrast(colors.onSidebarMuted, colors.sidebar) >= 4.5f, "$mode drawer muted text")
            val selectedRow = colors.sidebarSelected.compositeOver(colors.sidebar)
            assertTrue(contrast(colors.onSidebar, selectedRow) >= 4.5f, "$mode selected row text")
        }
    }

    @Test
    fun drawerUnreadCountsAreWhiteOnTheAccent() {
        for (mode in AppAppearanceMode.entries.filterNot { it.isDynamic }) {
            val colors = builtinChatColors(mode)
            assertTrue(colors.sidebarUnreadText == Color.White, "$mode badge text")
            assertTrue(contrast(Color.White, colors.sidebarUnreadBackground) >= 4.5f, "$mode badge")
        }
    }
}
