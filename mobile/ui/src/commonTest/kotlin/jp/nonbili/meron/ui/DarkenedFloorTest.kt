// Checks how message rendering chooses a lower brightness
// bound. The test compares transformed colors with the
// active surface and exercises the black-surface boundary.

package jp.nonbili.meron.ui

import androidx.compose.ui.graphics.Color
import kotlin.math.abs
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class DarkenedFloorTest {
    // The darkening filter, run forwards: invert, then hue-rotate(180deg).
    private fun darkenedTo(color: Color): Color {
        val (r, g, b) = listOf(1f - color.red, 1f - color.green, 1f - color.blue)
        return Color(
            (-0.574f * r + 1.43f * g + 0.144f * b).coerceIn(0f, 1f),
            (0.426f * r + 0.43f * g + 0.144f * b).coerceIn(0f, 1f),
            (0.426f * r + 1.43f * g - 0.856f * b).coerceIn(0f, 1f),
        )
    }

    private fun distance(
        a: Color,
        b: Color,
    ): Float = maxOf(abs(a.red - b.red), abs(a.green - b.green), abs(a.blue - b.blue)) * 255f

    @Test
    fun darkensBackToTheSurface() {
        for (surface in listOf(Color(0xFF1E1E1E), Color(0xFF2B2D31), Color(0xFF26323A))) {
            assertTrue(distance(darkenedTo(darkenedFloor(surface)), surface) <= 2f, "surface ${surface.toHex()}")
        }
    }

    @Test
    fun capsNothingForABlackSurface() {
        assertEquals(Color.White, darkenedFloor(Color.Black))
    }

    // A surface this saturated is outside what the filter can produce.
    @Test
    fun getsAsNearAsTheFilterReachesForASaturatedSurface() {
        val indigo = Color(0xFF4F46E5)
        val reached = darkenedTo(darkenedFloor(indigo))
        assertTrue(distance(reached, indigo) > 2f)
        assertTrue(distance(reached, indigo) <= 64f)
    }
}
