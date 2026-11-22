// Computes readable foregrounds and semantic colors for the mobile theme.
// Drawer and chat colors derive from the active light or dark palette.
// The contrast functions are also used by the theme regression tests.

package jp.nonbili.meron.ui

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance

/** WCAG contrast ratio between two opaque colors. */
internal fun contrastRatio(
    a: Color,
    b: Color,
): Float {
    val la = a.luminance()
    val lb = b.luminance()
    return (maxOf(la, lb) + 0.05f) / (minOf(la, lb) + 0.05f)
}

/** [text] blended toward [toward] just until it reaches [min] contrast on [bg]. */
private fun readableOn(
    text: Color,
    bg: Color,
    toward: Color,
    min: Float = 4.5f,
): Color {
    for (step in 0..20) {
        val candidate = lerp(text, toward, step / 20f)
        if (contrastRatio(candidate, bg) >= min) return candidate
    }
    return toward
}

/** The accent darkened just enough for white numbers, like desktop's whiteLabelAccent. */
private fun whiteLabelAccent(accent: Color): Color {
    for (step in 0..50) {
        val candidate = Color.Black.copy(alpha = step / 50f).compositeOver(accent)
        if (contrastRatio(Color.White, candidate) >= 4.5f) return candidate
    }
    return Color.Black
}

/**
 * Drawer colors for Meron's own themes, mirroring desktop's rail: the drawer
 * wears the rail color, rows take the thread list's selection tint, and unread
 * counts are the rail's white numbers on the accent.
 */
private fun drawerColors(spec: MobileThemeSpec): SidebarColors {
    // Desktop's sideNavInkColor: the theme's text where it reads on the rail.
    val text =
        if (contrastRatio(spec.textPrimary, spec.sidebar) >= 4.5f) spec.textPrimary else accentLabelColor(spec.sidebar)
    return SidebarColors(
        text = text,
        textMuted = readableOn(spec.textSecondary, spec.sidebar, text),
        selected = spec.accent.copy(alpha = if (spec.dark) 0.2f else 0.13f),
        // An icon tint, so 3:1 is enough.
        selectedContent = readableOn(spec.accent, spec.sidebar, text, 3f),
        unreadBackground = whiteLabelAccent(spec.accent),
        unreadText = Color.White,
    )
}

/** A built-in theme's chat colors, outside composition (for tests). */
internal fun builtinChatColors(mode: AppAppearanceMode): ChatColors = chatColors(mobileThemeSpec(mode))

internal fun chatColors(spec: MobileThemeSpec): ChatColors {
    val drawer = spec.sidebarColors ?: drawerColors(spec)
    return ChatColors(
        sidebar = spec.sidebar,
        onSidebar = drawer.text,
        onSidebarMuted = drawer.textMuted,
        sidebarAccent = drawer.selectedContent,
        bubbleIn = spec.bubbleIn,
        bubbleInText = spec.bubbleInText,
        bubbleOut = spec.bubbleOut,
        bubbleOutText = spec.bubbleOutText,
        star = if (spec.dark) Color(0xFFFBBF24) else Color(0xFFF59E0B),
        // Alpha compositing over opaque sRGB colors matches desktop's color-mix(in srgb).
        unreadBackground = spec.accent.copy(alpha = 0.18f).compositeOver(spec.bgChats),
        unreadText = spec.accent.copy(alpha = 0.55f).compositeOver(spec.textPrimary),
        sidebarUnreadBackground = drawer.unreadBackground,
        sidebarUnreadText = drawer.unreadText,
        sidebarSelected = drawer.selected,
    )
}
