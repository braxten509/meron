// Message text can contain markdown links and bare
// addresses. The fixtures exercise the display
// ranges and normalized destinations used by tap
// handlers.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class MessageTextTest {
    @Test
    fun parsesMarkdownAndBareLinksIntoDisplayRanges() {
        val parsed =
            parseInlineMessageText(
                "See [docs](example.com/docs) and https://example.com/raw",
            )

        assertEquals("See docs and https://example.com/raw", parsed.text)
        assertEquals(
            listOf(
                InlineMessageLink(4, 8, "https://example.com/docs"),
                InlineMessageLink(13, 36, "https://example.com/raw"),
            ),
            parsed.links,
        )
    }

    @Test
    fun normalizesWwwLinks() {
        val parsed = parseInlineMessageText("Open www.example.com/path")

        assertEquals("Open www.example.com/path", parsed.text)
        assertEquals(
            listOf(InlineMessageLink(5, 25, "https://www.example.com/path")),
            parsed.links,
        )
    }

    @Test
    fun leavesOrdinaryAndMalformedTextSelectableAsWritten() {
        val text = "Not a link: example and [unfinished](example.com"
        val parsed = parseInlineMessageText(text)

        assertEquals(text, parsed.text)
        assertTrue(parsed.links.isEmpty())
    }

    @Test
    fun findsTheLinkALongPressLandsOn() {
        val parsed = parseInlineMessageText("See [docs](example.com/docs) and https://example.com/raw")

        assertEquals(null, messageLinkAtCharOffset(parsed.links, 3))
        assertEquals(parsed.links[0], messageLinkAtCharOffset(parsed.links, 4))
        assertEquals(parsed.links[0], messageLinkAtCharOffset(parsed.links, 7))
        // The end offset is the first character past the link.
        assertEquals(null, messageLinkAtCharOffset(parsed.links, 8))
        assertEquals(parsed.links[1], messageLinkAtCharOffset(parsed.links, 20))
    }

    @Test
    fun leavesUnsafeLinkSchemesAsPlainSelectableText() {
        val text = "Do not open [this](javascript:alert(1))"
        val parsed = parseInlineMessageText(text)

        assertEquals(text, parsed.text)
        assertTrue(parsed.links.isEmpty())
    }
}
