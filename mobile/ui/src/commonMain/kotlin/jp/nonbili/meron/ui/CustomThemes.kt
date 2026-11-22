// Defines and transfers custom mobile themes.
// The serialized theme source is compatible
// with the desktop representation used for
// sharing.

package jp.nonbili.meron.ui

import androidx.compose.ui.graphics.Color
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.addJsonObject
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.put
import kotlin.math.PI
import kotlin.math.atan2
import kotlin.math.cbrt
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.pow
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlin.random.Random

// Custom themes, ported from desktop/frontend/src/lib/themes.ts: a theme is the
// desktop editor's five colors plus an appearance, shared as one line of text
// (see serializeThemeSource), and every other color is derived from them the
// way desktop's deriveThemeTokens does. Mobile has no editor; themes arrive by
// import.

/** The desktop editor's inputs, which are also the share string's fields. */
data class CustomThemeSource(
    val dark: Boolean,
    /** Window background behind everything. */
    val bgApp: Color,
    /** Panels: thread list, header, composer, incoming bubbles. */
    val surface: Color,
    /** The account rail; the drawer on mobile. */
    val sideNav: Color,
    val accent: Color,
    /** Primary text color. */
    val text: Color,
)

/** An imported theme, picked like a built-in one. [id] is desktop's "custom-<random>" form. */
data class CustomTheme(
    val id: String,
    val name: String,
    val source: CustomThemeSource,
) : AppTheme {
    override val storageValue: String get() = id
    override val label: String get() = name
    override val isDark: Boolean get() = source.dark
}

private const val CUSTOM_THEME_ID_PREFIX = "custom-"

internal fun newCustomThemeId(): String = CUSTOM_THEME_ID_PREFIX + buildString { repeat(12) { append("0123456789abcdefghijklmnopqrstuvwxyz"[Random.nextInt(36)]) } }

/**
 * Shareable one-line form of a custom theme, the same as desktop's, e.g.
 * "light,#f0f2f1,#ffffff,#121a16,#0e7a58,#1b211e": appearance, then background,
 * surface, side navigation, accent and text as hex.
 */
internal fun serializeThemeSource(source: CustomThemeSource): String =
    listOf(
        if (source.dark) "dark" else "light",
        source.bgApp.toHex(),
        source.surface.toHex(),
        source.sideNav.toHex(),
        source.accent.toHex(),
        source.text.toHex(),
    ).joinToString(",")

/** Inverse of [serializeThemeSource]; null for anything that isn't exactly that shape. */
internal fun parseThemeSource(text: String): CustomThemeSource? {
    val parts = text.trim().split(',').map { it.trim() }
    if (parts.size != 6) return null
    val dark =
        when (parts[0].lowercase()) {
            "light" -> false
            "dark" -> true
            else -> return null
        }
    val colors = parts.drop(1).map { parseHexColor(it) ?: return null }
    return CustomThemeSource(dark, colors[0], colors[1], colors[2], colors[3], colors[4])
}

/** "#rgb", "#rgba", "#rrggbb" or "#rrggbbaa", as desktop's parseColor reads hex. */
internal fun parseHexColor(text: String): Color? {
    val hex = text.removePrefix("#").takeIf { text.startsWith("#") } ?: return null
    if (hex.length !in setOf(3, 4, 6, 8) || hex.any { it.digitToIntOrNull(16) == null }) return null
    val full = if (hex.length <= 4) hex.map { "$it$it" }.joinToString("") else hex
    val channel = { index: Int -> full.substring(index * 2, index * 2 + 2).toInt(16) }
    return Color(channel(0), channel(1), channel(2), if (full.length == 8) channel(3) else 255)
}

/** "#rrggbb", or "#rrggbbaa" when translucent, like desktop's toHex. */
internal fun Color.toHex(): String {
    val to2 = { value: Float ->
        (value * 255)
            .roundToInt()
            .coerceIn(0, 255)
            .toString(16)
            .padStart(2, '0')
    }
    return "#" + to2(red) + to2(green) + to2(blue) + if (alpha >= 1f) "" else to2(alpha)
}

private val customThemesJson = Json { ignoreUnknownKeys = true }

/**
 * The stored list: `[{"id", "name", "source"}]` with the source as its share
 * string. Entries without a valid id or source are dropped, as desktop's
 * sanitizeCustomThemes does.
 */
internal fun parseCustomThemes(raw: String): List<CustomTheme> {
    if (raw.isBlank()) return emptyList()
    val array = runCatching { customThemesJson.parseToJsonElement(raw) as? JsonArray }.getOrNull() ?: return emptyList()
    val seen = mutableSetOf<String>()
    return array.mapNotNull { element ->
        val obj = element as? JsonObject ?: return@mapNotNull null
        val id = (obj["id"] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: return@mapNotNull null
        if (!id.startsWith(CUSTOM_THEME_ID_PREFIX) || !seen.add(id)) return@mapNotNull null
        val source = (obj["source"] as? JsonPrimitive)?.content?.let(::parseThemeSource) ?: return@mapNotNull null
        val name = (obj["name"] as? JsonPrimitive)?.content?.trim().orEmpty()
        CustomTheme(id, name.ifEmpty { "Custom theme" }, source)
    }
}

internal fun serializeCustomThemes(themes: List<CustomTheme>): String =
    buildJsonArray {
        themes.forEach { theme ->
            addJsonObject {
                put("id", theme.id)
                put("name", theme.name)
                put("source", serializeThemeSource(theme.source))
            }
        }
    }.toString()

// Color math, after desktop's lib/color.ts. Every result is rounded to 8-bit
// channels as desktop's formatColor does, so both derive the same colors.

private fun rgba(
    r: Float,
    g: Float,
    b: Float,
    a: Float,
): Color = Color(r.roundToInt().coerceIn(0, 255), g.roundToInt().coerceIn(0, 255), b.roundToInt().coerceIn(0, 255), (a.coerceIn(0f, 1f) * 255).roundToInt())

/** Linear sRGB blend: weight 0 -> [a], 1 -> [b]. */
private fun mix(
    a: Color,
    b: Color,
    weight: Float,
): Color {
    val w = weight.coerceIn(0f, 1f)
    return rgba(
        (a.red + (b.red - a.red) * w) * 255,
        (a.green + (b.green - a.green) * w) * 255,
        (a.blue + (b.blue - a.blue) * w) * 255,
        a.alpha + (b.alpha - a.alpha) * w,
    )
}

private fun lighten(
    color: Color,
    amount: Float,
) = mix(color, Color.White, amount)

private fun darken(
    color: Color,
    amount: Float,
) = mix(color, Color.Black, amount)

/** [color] composited over the opaque [base], as an opaque color. */
private fun flatten(
    color: Color,
    base: Color,
): Color {
    val blend = { top: Float, bottom: Float -> (top * color.alpha + bottom * (1 - color.alpha)) * 255 }
    return rgba(blend(color.red, base.red), blend(color.green, base.green), blend(color.blue, base.blue), 1f)
}

private class Oklch(
    var l: Double,
    val c: Double,
    val h: Double,
)

private fun toLinear(channel: Float): Double = if (channel <= 0.04045f) channel / 12.92 else ((channel + 0.055) / 1.055).pow(2.4)

private fun fromLinear(value: Double): Float = (255 * if (value <= 0.0031308) 12.92 * value else 1.055 * max(0.0, value).pow(1 / 2.4) - 0.055).toFloat()

private fun toOklch(color: Color): Oklch {
    val r = toLinear(color.red)
    val g = toLinear(color.green)
    val b = toLinear(color.blue)
    val l = cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b)
    val m = cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b)
    val s = cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b)
    val lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s
    val a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s
    val bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s
    return Oklch(lightness, hypot(a, bb), (atan2(bb, a) * 180 / PI + 360) % 360)
}

private fun fromOklch(oklch: Oklch): Color {
    val a = oklch.c * cos(oklch.h * PI / 180)
    val b = oklch.c * sin(oklch.h * PI / 180)
    val l = (oklch.l + 0.3963377774 * a + 0.2158037573 * b).pow(3)
    val m = (oklch.l - 0.1055613458 * a - 0.0638541728 * b).pow(3)
    val s = (oklch.l - 0.0894841775 * a - 1.291485548 * b).pow(3)
    return rgba(
        fromLinear(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
        fromLinear(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
        fromLinear(-0.0041960863 * l - 0.7034186147 * m + 1.7076949135 * s),
        1f,
    )
}

/** [color]'s hue and chroma at perceptual lightness [lightness]. */
private fun withLightness(
    color: Color,
    lightness: Float,
): Color = fromOklch(toOklch(color).apply { l = lightness.toDouble().coerceIn(0.0, 1.0) })

private fun lightnessOf(color: Color): Float = toOklch(color).l.toFloat()

// Surface placement and contrast, after desktop's applySurfaceRules.

private class SurfaceLadder(
    val chat: Float,
    val active: Float,
    val border: Float,
    val bubbleIn: Float,
    val bubbleOut: Float,
)

private val LightLadder = SurfaceLadder(chat = -0.035f, active = -0.08f, border = -0.09f, bubbleIn = 0.035f, bubbleOut = -0.035f)
private val DarkLadder = SurfaceLadder(chat = -0.04f, active = 0.08f, border = 0.1f, bubbleIn = 0.09f, bubbleOut = 0.16f)

private const val TEXT_MIN_CONTRAST = 4.5f

/** [color] moved in lightness, away from [backgrounds], until it reaches [min] contrast on all of them. */
private fun withMinContrast(
    color: Color,
    backgrounds: List<Color>,
    min: Float,
    direction: Int,
): Color {
    val oklch = toOklch(color)
    var out = rgba(color.red * 255, color.green * 255, color.blue * 255, color.alpha)
    // Stop only at the end of the scale being moved toward: white sits at the
    // top of it and can still darken, black at the bottom and can still lighten.
    val hasRoom = { if (direction < 0) oklch.l > 0.0 else oklch.l < 1.0 }
    while (backgrounds.minOf { contrastRatio(out, it) } < min && hasRoom()) {
        oklch.l = (oklch.l + direction * 0.005).coerceIn(0.0, 1.0)
        out = fromOklch(oklch)
    }
    return out
}

/** [text] made readable on [bg], on the side of it it already sits on when that side has room. */
private fun readableOnSide(
    text: Color,
    bg: Color,
    min: Float,
): Color {
    val side = if (lightnessOf(text) > lightnessOf(bg)) 1 else -1
    val kept = withMinContrast(text, listOf(bg), min, side)
    return if (contrastRatio(kept, bg) >= min) kept else withMinContrast(text, listOf(bg), min, -side)
}

/** The full spec for an imported theme: desktop's deriveThemeTokens, mapped onto mobile's roles. */
internal fun customThemeSpec(source: CustomThemeSource): MobileThemeSpec {
    val (dark, bgApp, surface, sideNav, accentIn, text) = source
    val light = !dark
    val ladder = if (light) LightLadder else DarkLadder
    val list = lightnessOf(surface)
    val chat = list + ladder.chat
    val bgChat = withLightness(bgApp, chat)
    val border = withLightness(flatten(mix(surface, text, 0.12f), surface), list + ladder.border)
    val bubbleIn = withLightness(if (light) surface else lighten(surface, 0.08f), chat + ladder.bubbleIn)
    // Text darkens on a light theme and lightens on a dark one to gain contrast.
    val away = if (light) -1 else 1
    val accent = withMinContrast(accentIn, listOf(surface, bubbleIn), TEXT_MIN_CONTRAST, away)
    val bubbleOut = withLightness(if (light) mix(accentIn, Color.White, 0.8f) else mix(accentIn, surface, 0.55f), chat + ladder.bubbleOut)
    val bubbleOutText = readableOnSide(if (light) darken(accentIn, 0.45f) else mix(accentIn, Color.White, 0.75f), bubbleOut, TEXT_MIN_CONTRAST)
    val bgActive = withLightness(flatten(if (light) mix(surface, text, 0.12f) else lighten(surface, 0.1f), surface), list + ladder.active)
    return MobileThemeSpec(
        dark = dark,
        bgApp = bgApp,
        bgChats = surface,
        // Desktop's dark raised panel is translucent; mobile needs it opaque.
        bgRaised = if (light) mix(surface, bgApp, 0.5f) else flatten(surface.copy(alpha = 0.4f), bgApp),
        bgActive = bgActive,
        border = border,
        textPrimary = text,
        textSecondary = withMinContrast(mix(text, bgApp, 0.45f), listOf(surface, bgChat), TEXT_MIN_CONTRAST, away),
        accent = accent,
        accentContainer = bubbleOut,
        onAccentContainer = bubbleOutText,
        sidebar = sideNav,
        bubbleIn = bubbleIn,
        bubbleInText = readableOnSide(text, bubbleIn, TEXT_MIN_CONTRAST),
        bubbleOut = bubbleOut,
        bubbleOutText = bubbleOutText,
    )
}
