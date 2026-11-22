// Quoted and forwarded messages in the mobile editor.
// Fixtures exercise HTML, plain text and references to inline media.
// Saving and reopening a draft must retain the same quoted content.
package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class ForwardAndQuoteTest {
    @Test
    fun messageForwardAndEditDraftsCarryBodyHeadersAndAttachments() {
        val message =
            MessageBody(
                id = "m3",
                from = "Ada",
                fromAddr = "ada@example.com",
                to = "Me <me@example.com>",
                cc = "Project <project@example.com>",
                subject = "Design",
                body = "Original",
                bodyHtml = """<p>Original</p><img src="/media/acc/INBOX/3/inline.png">""",
                attachments =
                    listOf(
                        MessageAttachment(filename = "note.txt", mimeType = "text/plain", sizeBytes = 2, key = "acc/INBOX/3/note.txt"),
                        MessageAttachment(filename = "inline.png", mimeType = "image/png", sizeBytes = 4, key = "acc/INBOX/3/inline.png"),
                        MessageAttachment(filename = "remote.jpg", mimeType = "image/jpeg", url = "https://example.com/remote.jpg"),
                    ),
            )
        val copied =
            forwardableAttachments(message).map {
                attachmentToDraftAttachment(it, "SGk=")
            }
        val forward = messageForwardDraft(message, copied)
        val edit = messageEditAsNewDraft(message, copied)

        assertEquals("Fwd: Design", forward.subject)
        assertEquals("", forward.to)
        assertEquals(1, forward.attachments.size)
        assertEquals("note.txt", forward.attachments.single().displayName)
        assertEquals(true, forward.body.contains("---------- Forwarded message ---------"))
        assertEquals(true, forward.body.contains("Cc: Project <project@example.com>"))
        assertEquals("Me <me@example.com>", edit.to)
        assertEquals("Project <project@example.com>", edit.cc)
        assertEquals("Design", edit.subject)
        assertEquals("Original", edit.body)
        assertEquals(1, edit.attachments.size)
    }

    @Test
    fun forwardDraftQuotesOriginalHtmlAndReInlinesItsImages() {
        val message =
            MessageBody(
                id = "m4",
                from = "Ada <ada@example.com>",
                fromAddr = "ada@example.com",
                to = "Me <me@example.com>",
                subject = "Design",
                body = "Original",
                bodyHtml = """<p>Original &amp; more</p><img src="/media/acc/INBOX/4/inline.png">""",
                attachments =
                    listOf(
                        MessageAttachment(filename = "inline.png", mimeType = "image/png", sizeBytes = 4, key = "acc/INBOX/4/inline.png"),
                    ),
            )
        val inlineImages = forwardInlineImages(message)
        val draft =
            messageForwardDraft(
                message = message,
                attachments = emptyList(),
                dateLabel = "Jul 27, 2026, 10:00",
                inlineImages = inlineImages,
            )

        assertEquals(1, inlineImages.size)
        assertEquals(true, draft.body.contains("Date: Jul 27, 2026, 10:00"))
        assertEquals(true, draft.html.contains("<strong>Date:</strong> Jul 27, 2026, 10:00"))
        assertEquals(true, draft.html.contains("<p>Original &amp; more</p>"))
        // The reader's local media path must not survive into the sent message.
        assertEquals(true, draft.html.contains("""<img src="cid:meron-forward-0@meron">"""))
        assertEquals(false, draft.html.contains("/media/"))
    }

    @Test
    fun forwardDraftStripsReaderDocumentAndUnavailableInlineMedia() {
        val message =
            MessageBody(
                id = "m-reader",
                from = "Ada <ada@example.com>",
                to = "Me <me@example.com>",
                subject = "Prepared",
                body = "Original",
                bodyHtml =
                    """<!doctype html><html><head><meta charset="utf-8">""" +
                        """<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src 'self' data:;">""" +
                        """<style>img,video{max-width:100%;height:auto}img{cursor:zoom-in}</style>""" +
                        """<style>.original{color:red}</style></head><body>""" +
                        """<p class="original">Original</p><img src="/media/acc/INBOX/5/missing.png">""" +
                        """</body></html>""",
                attachments =
                    listOf(
                        MessageAttachment(
                            filename = "missing.png",
                            mimeType = "image/png",
                            key = "acc/INBOX/5/missing.png",
                        ),
                    ),
            )

        val html = messageForwardDraft(message, inlineImages = emptyList()).html

        assertEquals(false, html.contains("<!doctype", ignoreCase = true))
        assertEquals(false, html.contains("Content-Security-Policy", ignoreCase = true))
        assertEquals(false, html.contains("cursor:zoom-in"))
        assertEquals(true, html.contains("<style>.original{color:red}</style>"))
        assertEquals(true, html.contains("""<p class="original">Original</p>"""))
        assertEquals(false, html.contains("/media/"))
    }

    @Test
    fun forwardDraftOfTextOnlyMessageStaysPlain() {
        val message =
            MessageBody(
                id = "m5",
                from = "Ada <ada@example.com>",
                to = "Me <me@example.com>",
                subject = "Design",
                body = "Original",
            )

        assertEquals("", messageForwardDraft(message).html)
    }

    @Test
    fun replyQuoteQuotesThePlainBodyUnderAnAttribution() {
        val message = MessageBody(id = "m", from = "Jane", fromAddr = "jane@example.com", to = "", subject = "Hi", body = "Lunch?\n\nJane\n")

        assertEquals("On Mon, Jane <jane@example.com> wrote:", replyQuoteAttribution(message, "Mon"))
        assertEquals("\n\nOn Mon, Jane <jane@example.com> wrote:\n> Lunch?\n>\n> Jane", replyQuotedPlainBody(message, "Mon"))
        // A text-only original stays a text-only reply.
        assertEquals("", replyQuotedHtmlBody(message, "Mon"))
        assertEquals("On Mon, jane@example.com wrote:", replyQuoteAttribution(message.copy(from = ""), "Mon"))
    }

    @Test
    fun replyQuoteCitesAnHtmlOriginalAndRebuildsItsInlineImages() {
        val image = MessageAttachment(filename = "a.png", mimeType = "image/png", key = "acct/1/0.png")
        val message =
            MessageBody(
                id = "m",
                from = "Jane",
                fromAddr = "jane@example.com",
                to = "",
                subject = "Hi",
                body = "Lunch?",
                bodyHtml = "<p>Lunch? <img src=\"/media/acct/1/0.png\"></p>",
                attachments = listOf(image),
            )

        assertEquals(
            "<p>On Mon, Jane &lt;jane@example.com&gt; wrote:</p><blockquote type=\"cite\"><p>Lunch? <img src=\"cid:meron-forward-0@meron\"></p></blockquote>",
            replyQuotedHtmlBody(message, "Mon", forwardInlineImages(message)),
        )
        // An image whose bytes could not be read never points at this device.
        assertTrue(!replyQuotedHtmlBody(message, "Mon").contains("/media/"))
    }

    @Test
    fun withReplyQuoteAppendsTheQuoteToTheSentReply() {
        val params = SendMailParams(accountId = "a", to = "jane@example.com", subject = "Re: Hi", body = "Sounds good")
        val quote = "\n\nOn Mon, Jane wrote:\n> Lunch?"

        val plain = params.withReplyQuote(quote, "", "On Mon, Jane wrote:")
        assertEquals("Sounds good\n\nOn Mon, Jane wrote:\n> Lunch?", plain.body)
        assertEquals("", plain.html)

        val inline = DraftAttachment(id = "i", displayName = "a.png", mimeType = "image/png", dataBase64 = "AA==", inlineId = "meron-forward-0@meron")
        val rich = params.withReplyQuote(quote, "<p>On Mon, Jane wrote:</p><blockquote type=\"cite\"><p>Lunch?</p></blockquote>", "On Mon, Jane wrote:", listOf(inline))
        assertEquals("<p>Sounds good</p><p>On Mon, Jane wrote:</p><blockquote type=\"cite\"><p>Lunch?</p></blockquote>", rich.html)
        assertEquals(listOf("meron-forward-0@meron"), rich.attachments.map { it.inlineId })

        assertEquals("On Mon, Jane wrote:\n> Lunch?", params.copy(body = "").withReplyQuote(quote, "", "On Mon, Jane wrote:").body)

        // A reopened quoted reply already carries its quote.
        val saved = params.copy(body = "Sounds good$quote").withReplyQuote(quote, "<blockquote type=\"cite\"><p>Lunch?</p></blockquote>", "On Mon, Jane wrote:")
        assertEquals("Sounds good$quote", saved.body)
        assertEquals("<p>Sounds good</p><blockquote type=\"cite\"><p>Lunch?</p></blockquote>", saved.html)

        // One whose quote the user trimmed goes out as they left it.
        val trimmed = params.copy(body = "Sounds good\n\nOn Mon, Jane wrote:\n> Lun").withReplyQuote(quote, "<p>x</p>", "On Mon, Jane wrote:")
        assertEquals("Sounds good\n\nOn Mon, Jane wrote:\n> Lun", trimmed.body)
        assertEquals("", trimmed.html)
    }

    @Test
    fun forwardHtmlForSendFindsAReplyQuoteByItsAttribution() {
        val quote = "<p>On Mon, Jane wrote:</p><blockquote type=\"cite\"><p>Lunch?</p></blockquote>"
        val body = "Sounds good\n\nOn Mon, Jane wrote:\n> Lunch?"

        assertEquals("<p>Sounds good</p>$quote", forwardHtmlForSend(body, quote, "On Mon, Jane wrote:"))
        assertEquals("", forwardHtmlForSend("Sounds good", quote, "On Mon, Jane wrote:"))
    }

    @Test
    fun forwardHtmlForSendPrependsTypedTextAndDropsHtmlWithoutQuote() {
        val quote = """<p><br></p><div class="meron-forwarded-message"><p>$FORWARD_QUOTE_MARKER</p><p>Original</p></div>"""
        val body = "See below.\n\n$FORWARD_QUOTE_MARKER\nFrom: Ada\n\nOriginal"

        assertEquals("<p>See below.</p>$quote", forwardHtmlForSend(body, quote))
        // The user deleted the quote: there is no boundary left, so the send
        // falls back to plain text rather than shipping a stale HTML part.
        assertEquals("", forwardHtmlForSend("Just my own words", quote))
        assertEquals("", forwardHtmlForSend(body, ""))
    }

    @Test
    fun savedForwardDraftKeepsItsQuoteOnReopen() {
        val savedHtml =
            """<p>My note</p><p><br></p><div class="meron-forwarded-message"><p>$FORWARD_QUOTE_MARKER</p><p>Original</p></div>"""

        assertEquals(
            """<p><br></p><div class="meron-forwarded-message"><p>$FORWARD_QUOTE_MARKER</p><p>Original</p></div>""",
            forwardedHtmlQuote(savedHtml),
        )
        assertEquals("", forwardedHtmlQuote("<p>A plain draft</p>"))
    }

    @Test
    fun savedForwardDraftExtractsQuoteFromReaderDocument() {
        val quote = """$FORWARD_QUOTE_MARKER</p><p>Original</p></div>"""
        val savedHtml =
            """<!doctype html><html><head><meta charset="utf-8">""" +
                """<meta http-equiv="Content-Security-Policy" content="default-src 'none';">""" +
                """<style>img,video{max-width:100%;height:auto}img{cursor:zoom-in}</style>""" +
                """</head><body><p>My note</p><p><br></p><div class="meron-forwarded-message"><p>""" +
                quote +
                """</body></html>"""

        assertEquals(
            """<p><br></p><div class="meron-forwarded-message"><p>$quote""",
            forwardedHtmlQuote(savedHtml),
        )
    }

    @Test
    fun forwardDraftSendsHtmlAlternativeAndInlineParts() {
        val draft =
            ComposeDraft(
                to = "bob@example.com",
                subject = "Fwd: Design",
                body = "See below.",
                attachments =
                    listOf(
                        DraftAttachment(
                            id = "acc/INBOX/4/inline.png",
                            displayName = "inline.png",
                            mimeType = "image/png",
                            dataBase64 = "SGk=",
                            inlineId = "meron-forward-0@meron",
                        ),
                    ),
                html = "<p>See below.</p><p>Original</p>",
            )

        assertEquals(
            """{"account_id":"acc1","from":"me@example.com","to":"bob@example.com","cc":"","bcc":"","reply_to":"",""" +
                """"subject":"Fwd: Design","body":"See below.","html":"<p>See below.</p><p>Original</p>",""" +
                """"in_reply_to":"","references":"","message_id":"","attachments":[""" +
                """{"filename":"inline.png","mime":"image/png","data":"SGk=","inline_id":"meron-forward-0@meron"}]}""",
            draft.toSendMailParams(accountId = "acc1", from = "me@example.com").toJson(),
        )
    }
}
