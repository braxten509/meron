// Refresh diagnostics read a bounded tail of
// an Android log file. Fixtures cover a
// missing file and logs small enough to return
// whole.

package jp.nonbili.meron

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class AndroidSyncDiagnosticLogTest {
    @Test
    fun testReadTailOfMissingFileIsEmpty() {
        assertEquals("", AndroidSyncDiagnosticLog.readTail(File("does-not-exist-${System.nanoTime()}.log")))
    }

    @Test
    fun testReadTailKeepsASmallLogWhole() {
        val file = File.createTempFile("diag", ".log").apply { deleteOnExit() }
        file.writeText("a\nb\n")
        assertEquals("a\nb\n", AndroidSyncDiagnosticLog.readTail(file))
    }

    @Test
    fun testReadTailOfALargeLogStartsOnAWholeLine() {
        val file = File.createTempFile("diag", ".log").apply { deleteOnExit() }
        file.writeText("x".repeat(1 shl 20) + "\nnewest\n")
        assertEquals("newest\n", AndroidSyncDiagnosticLog.readTail(file, maxBytes = 1024))
    }

    @Test
    fun testReadTailNeverStartsInsideAUtf8Sequence() {
        // One long line of 3-byte characters: no newline to cut at, so the
        // cut lands mid-character and must skip to the next lead byte.
        val file = File.createTempFile("diag", ".log").apply { deleteOnExit() }
        file.writeText("あ".repeat(1000))
        val tail = AndroidSyncDiagnosticLog.readTail(file, maxBytes = 1000)
        assertTrue(tail.isNotEmpty())
        assertTrue(tail.all { it == 'あ' })
    }
}
