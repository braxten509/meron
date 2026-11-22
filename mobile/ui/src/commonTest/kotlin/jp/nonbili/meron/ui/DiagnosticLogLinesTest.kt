// The diagnostic screen presents a log as individual
// rows. Cases exercise blank input, line boundaries
// and the display limit for a single oversized line.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class DiagnosticLogLinesTest {
    @Test
    fun blankLogHasNoLines() {
        assertEquals(emptyList(), diagnosticLogLines(""))
        assertEquals(emptyList(), diagnosticLogLines("\n\n"))
    }

    @Test
    fun splitsIntoOneRowPerLine() {
        assertEquals(listOf("a", "b"), diagnosticLogLines("a\nb\n"))
    }

    @Test
    fun capsARunawayLine() {
        val line = diagnosticLogLines("x".repeat(1 shl 20)).single()
        assertTrue(line.length < 10_000)
        assertTrue(line.endsWith("…"))
    }
}
