// Mobile text size preferences use discrete scale
// steps. The tests cover storing an exact step and
// resolving an intermediate value to the nearest
// available size.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals

class AppFontsTest {
    @Test
    fun testKeepsAStepScale() {
        assertEquals(130, coerceMessageFontScale(130))
    }

    @Test
    fun testSnapsOffStepScaleToNearestStep() {
        assertEquals(115, coerceMessageFontScale(112))
        assertEquals(80, coerceMessageFontScale(0))
        assertEquals(200, coerceMessageFontScale(10_000))
    }

    @Test
    fun testEveryStepIsItsOwnSliderStop() {
        assertEquals(MESSAGE_FONT_SCALE_STEPS, MESSAGE_FONT_SCALE_STEPS.map { coerceMessageFontScale(it) })
        assertEquals(MESSAGE_FONT_SCALE_STEPS.sorted(), MESSAGE_FONT_SCALE_STEPS)
    }

    @Test
    fun testDefaultIsAStep() {
        assertEquals(DEFAULT_MESSAGE_FONT_SCALE, coerceMessageFontScale(DEFAULT_MESSAGE_FONT_SCALE))
    }

    @Test
    fun testScalesCssPixels() {
        assertEquals("16px", scaledCssPx(MESSAGE_HTML_BASE_PX, DEFAULT_MESSAGE_FONT_SCALE))
        assertEquals("32px", scaledCssPx(MESSAGE_HTML_BASE_PX, 200))
        assertEquals("12.8px", scaledCssPx(MESSAGE_HTML_BASE_PX, 80))
        assertEquals("18.4px", scaledCssPx(MESSAGE_HTML_BASE_PX, 115))
    }

    @Test
    fun testFoldsInSystemFontScaleWhereTheWebViewIgnoresIt() {
        // What HtmlMessageBody computes on a platform whose web view leaves the
        // system font setting to us (iOS): a 150% accessibility size and a 130%
        // preference compound, as they do for the sp-sized plain-text bodies.
        assertEquals("31.2px", scaledCssPx(MESSAGE_HTML_BASE_PX * 1.5f, 130))
        assertEquals("24px", scaledCssPx(MESSAGE_HTML_BASE_PX * 1.5f, DEFAULT_MESSAGE_FONT_SCALE))
    }
}
