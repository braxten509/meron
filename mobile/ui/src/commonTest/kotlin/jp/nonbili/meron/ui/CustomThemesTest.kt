// Mobile themes can be shared with desktop
// installations. The fixtures round-trip the shared
// format and exercise flexible color notation used by
// imported themes.

package jp.nonbili.meron.ui

import androidx.compose.ui.graphics.Color
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

class CustomThemesTest {
    private val sample = "light,#f0f2f1,#ffffff,#121a16,#0e7a58,#1b211e"

    @Test
    fun shareStringRoundTripsInDesktopFormat() {
        val source = parseThemeSource(sample)!!
        assertEquals(false, source.dark)
        assertEquals(Color(0xFF121A16), source.sideNav)
        assertEquals(sample, serializeThemeSource(source))
    }

    @Test
    fun parsingToleratesSpacingCaseAndShortHex() {
        val source = parseThemeSource("  Dark, #000 ,#fff,#123,#40A984,#f2f5f3 ")!!
        assertEquals(true, source.dark)
        assertEquals("dark,#000000,#ffffff,#112233,#40a984,#f2f5f3", serializeThemeSource(source))
    }

    @Test
    fun parsingRejectsAnythingElse() {
        assertNull(parseThemeSource(""))
        assertNull(parseThemeSource("light,#f0f2f1,#ffffff,#121a16,#0e7a58"))
        assertNull(parseThemeSource("dim,#f0f2f1,#ffffff,#121a16,#0e7a58,#1b211e"))
        assertNull(parseThemeSource("light,f0f2f1,#ffffff,#121a16,#0e7a58,#1b211e"))
        assertNull(parseThemeSource("light,#f0f2f1,#ffffff,#121a16,#0e7a58,tomato"))
        assertNull(parseThemeSource("light,#f0f2f1,#ffffff,#121a16,#0e7a58,#1b211e,#000000"))
    }

    @Test
    fun storedListRoundTripsAndDropsBadEntries() {
        val theme = CustomTheme(newCustomThemeId(), "Moss", parseThemeSource(sample)!!)
        assertEquals(listOf(theme), parseCustomThemes(serializeCustomThemes(listOf(theme))))
        val raw =
            """[{"id":"${theme.id}","name":"Moss","source":"$sample"},""" +
                """{"id":"${theme.id}","name":"Duplicate","source":"$sample"},""" +
                """{"id":"nope","name":"Bad id","source":"$sample"},""" +
                """{"id":"custom-x","name":"Bad source","source":"light"},""" +
                """{"id":"custom-y","name":" ","source":"$sample"}]"""
        val parsed = parseCustomThemes(raw)
        assertEquals(listOf(theme.id, "custom-y"), parsed.map { it.id })
        assertEquals("Custom theme", parsed[1].name)
        assertEquals(emptyList(), parseCustomThemes("not json"))
    }

    @Test
    fun derivedThemeKeepsTextReadable() {
        for (text in listOf(sample, "dark,#0c100e,#151b18,#060908,#40a984,#f2f5f3", "dark,#101010,#202020,#000000,#303030,#808080")) {
            val spec = customThemeSpec(parseThemeSource(text)!!)
            assertTrue(contrastRatio(spec.accent, spec.bgChats) >= 4.5f, "accent on list in $text")
            assertTrue(contrastRatio(spec.textSecondary, spec.bgChats) >= 4.5f, "secondary text in $text")
            assertTrue(contrastRatio(spec.bubbleInText, spec.bubbleIn) >= 4.5f, "incoming bubble in $text")
            assertTrue(contrastRatio(spec.bubbleOutText, spec.bubbleOut) >= 4.5f, "outgoing bubble in $text")
        }
    }

    @Test
    fun accentsAtTheEndsOfTheScaleStillMoveForContrast() {
        // Desktop darkens a white accent on a white list to the same gray.
        val white = customThemeSpec(parseThemeSource("light,#f0f2f1,#ffffff,#121a16,#ffffff,#1b211e")!!)
        assertEquals("#767676", white.accent.toHex())
        val black = customThemeSpec(parseThemeSource("dark,#000000,#000000,#000000,#000000,#ffffff")!!)
        assertTrue(contrastRatio(black.accent, black.bgChats) >= 4.5f)
    }

    @Test
    fun deletingAPickedThemeFallsBackToTheDefaults() {
        val light = CustomTheme("custom-a", "A", parseThemeSource(sample)!!)
        val dark = CustomTheme("custom-b", "B", parseThemeSource("dark,#0c100e,#151b18,#060908,#40a984,#f2f5f3")!!)
        val choice = ThemeChoice(fixed = dark, followSystem = true, light = light, dark = dark)
        val defaults = ThemeChoice()
        assertEquals(ThemeChoice(fixed = defaults.dark, followSystem = true, light = light, dark = defaults.dark), choice.without(dark))
        assertEquals(ThemeChoice(fixed = dark, followSystem = true, light = defaults.light, dark = dark), choice.without(light))
    }
}
