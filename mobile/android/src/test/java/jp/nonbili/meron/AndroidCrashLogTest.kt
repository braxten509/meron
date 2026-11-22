// Crash summaries combine an exception type with its message
// for the Android log. Cases cover missing detail so an
// exception without a message still produces a readable
// entry.

package jp.nonbili.meron

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidCrashLogTest {
    @Test
    fun testCrashSummaryKeepsTypeAndMessage() {
        assertEquals(
            "java.lang.IllegalStateException: engine not started",
            crashSummary(IllegalStateException("engine not started")),
        )
    }

    @Test
    fun testCrashSummaryDropsDanglingColonWhenMessageIsMissing() {
        assertEquals("java.lang.NullPointerException", crashSummary(NullPointerException()))
    }

    @Test
    fun testCrashSummaryIsRedactedBeforeItLeavesTheDevice() {
        // The summary reaches the prompt and the marker file; redaction is what
        // keeps a crash message carrying an address from being shared verbatim.
        val summary = crashSummary(IllegalArgumentException("no mailbox for alice@example.com"))
        assertEquals("java.lang.IllegalArgumentException: no mailbox for a***@example.com", redactMessage(summary))
    }

    @Test
    fun testTruncateTraceKeepsShortTracesIntact() {
        val trace = "java.lang.IllegalStateException: boom\n\tat jp.nonbili.meron.Foo.bar(Foo.kt:12)"
        assertEquals(trace, truncateTrace(trace))
    }

    @Test
    fun testTruncateTraceCapsLongTraces() {
        val trace = (1..200).joinToString("\n") { "\tat jp.nonbili.meron.Frame$it.run(Frame.kt:$it)" }
        val truncated = truncateTrace(trace).lines()
        assertEquals(61, truncated.size)
        assertEquals("\tat jp.nonbili.meron.Frame1.run(Frame.kt:1)", truncated.first())
        assertEquals("... 140 frames omitted", truncated[30])
        assertEquals("\tat jp.nonbili.meron.Frame200.run(Frame.kt:200)", truncated.last())
    }

    @Test
    fun testTruncateTraceKeepsDeepestCauseAndItsFirstFrames() {
        val outer = (1..80).map { "\tat jp.nonbili.meron.Outer$it.run(Outer.kt:$it)" }
        val cause =
            listOf("Caused by: java.lang.IllegalArgumentException: root cause") +
                (1..80).map { "\tat jp.nonbili.meron.Cause$it.run(Cause.kt:$it)" }
        val truncated = truncateTrace((outer + cause).joinToString("\n")).lines()

        assertEquals(61, truncated.size)
        assertEquals("Caused by: java.lang.IllegalArgumentException: root cause", truncated[31])
        assertEquals("\tat jp.nonbili.meron.Cause1.run(Cause.kt:1)", truncated[32])
        assertTrue(truncated.none { it.contains("Cause80") })
    }
}
