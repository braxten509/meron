// The core supplies an offset separating a plain
// message from quoted history. These tests check the
// split and how to display a body when the offset is
// unusable.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class QuoteFoldTest {
    private val reply = "See you Friday.\n\n"
    private val quote = "On Mon, Jane wrote:\n> Lunch on Friday?\n"

    @Test
    fun splitsAPlainBodyAtTheCoreOffset() {
        val split = splitQuotedBody(reply + quote, reply.length)

        assertEquals("See you Friday.", split.reply)
        assertEquals("On Mon, Jane wrote:\n> Lunch on Friday?", split.quote)
    }

    @Test
    fun leavesTheBodyWholeWithoutAUsableOffset() {
        val body = reply + quote
        for (start in listOf(null, 0, -1, body.length, body.length + 3)) {
            assertEquals(QuotedBody(body, ""), splitQuotedBody(body, start), "offset $start")
        }
    }

    @Test
    fun opensTheQuoteOnlyForASearchMatchInsideIt() {
        assertTrue(quoteMatchesSearch(quote, "LUNCH"))
        assertFalse(quoteMatchesSearch(quote, "see you"))
        assertFalse(quoteMatchesSearch(quote, "  "))
        assertFalse(quoteMatchesSearch("", "lunch"))
    }

    @Test
    fun remembersWhichQuotesWereOpened() {
        val key = "quote-fold-test#m1"
        assertFalse(QuoteFoldMemory.isOpen(key))
        QuoteFoldMemory.setOpen(key, true)
        assertTrue(QuoteFoldMemory.isOpen(key))
        assertFalse(QuoteFoldMemory.isOpen("quote-fold-test#m2"))
        QuoteFoldMemory.setOpen(key, false)
        assertFalse(QuoteFoldMemory.isOpen(key))
    }

    @Test
    fun escapesLabelsForAnInlineScript() {
        assertEquals("\"Show quoted text\"", jsStringLiteral("Show quoted text"))
        assertEquals(
            "\"a\\\"b\\\\c\\n\\u003c/script>\\u2028\"",
            jsStringLiteral("a\"b\\c\n</script>\u2028"),
        )
    }
}
