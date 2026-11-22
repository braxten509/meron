// Thread response parsers test.
// The mobile host suites exercise these cases with deterministic core responses.
// The tests run without a device or a live mail account.

package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class ThreadResponseParsersTest {
    @Test
    fun parsesThreadListEnvelope() {
        val threads =
            parseThreadListResponse(
                """{"id":2,"result":{"threads":[{"id":"acc#INBOX#t","account_id":"acc","folder_id":"INBOX","from_name":"Ada","subject":"Hello","preview":"Snippet","date":1700000000,"unread":true,"unread_count":2,"starred":true,"has_starred_items":true,"has_draft":true,"feed_url":"https://example.com/feed.xml"}],"folder_unread":3}}""",
            )

        assertEquals(1, threads.size)
        assertEquals("acc#INBOX#t", threads[0].id)
        assertEquals("acc", threads[0].accountId)
        assertEquals("INBOX", threads[0].folder)
        assertEquals("Ada", threads[0].sender)
        assertEquals("Hello", threads[0].subject)
        assertEquals("Snippet", threads[0].preview)
        assertEquals(1_700_000_000, threads[0].dateEpochSeconds)
        assertTrue(threads[0].unread)
        assertEquals(2, threads[0].unreadCount)
        assertTrue(threads[0].starred)
        assertTrue(threads[0].hasStarredItems)
        assertTrue(threads[0].hasDraft)
        assertEquals("https://example.com/feed.xml", threads[0].feedUrl)
    }

    @Test
    fun threadListSenderFallsBackToAddressWhenNameIsEmpty() {
        val threads =
            parseThreadListResponse(
                """{"result":{"threads":[{"id":"acc#INBOX#t","account_id":"acc","folder_id":"INBOX","from_name":"","from_addr":"ada@example.com","subject":"Hello"}]}}""",
            )

        assertEquals(1, threads.size)
        assertEquals("ada@example.com", threads[0].sender)
    }

    @Test
    fun contactHistoryReusesThreadCardsAndOpaqueCursor() {
        val response =
            """{"result":{"threads":[{"id":"other#Archive#t","account_id":"other","folder_id":"Archive","folder_role":"archive","subject":"Hello","from_name":"Alice","unread":true,"unread_count":2,"message_count":3,"tags":["Work"],"starred":true}],"next_cursor":"[200,\"other\\nkey\"]"}}"""
        val page = parseContactThreadsPage(response)
        assertEquals(parseThreadListPage(response), page)
        assertEquals("[200,\"other\\nkey\"]", page.nextCursor)
        assertEquals("other", page.threads.single().accountId)
        assertEquals("Archive", page.threads.single().folder)
        assertEquals(listOf("Work"), page.threads.single().tags)
        assertEquals("", parseContactThreadsPage("""{"threads":[],"next_cursor":null}""").nextCursor)
    }

    @Test
    fun parsesThreadFilesFromTopLevelKeysOnly() {
        val threads =
            parseThreadListResponse(
                """{"result":{"threads":[{"id":"acc#INBOX#t","subject":"\"files\":[{\"filename\":\"forged.exe\"}]","has_attachments":true,"files":[{"filename":"plan.pdf","mime":"application/pdf"},{"filename":"photo.jpg","mime":"image/jpeg"}]},{"id":"acc#INBOX#u","subject":"Plain"}]}}""",
            )

        assertTrue(threads[0].hasAttachments)
        assertEquals(listOf("plan.pdf", "photo.jpg"), threads[0].files.map { it.filename })
        assertEquals("image/jpeg", threads[0].files[1].mimeType)
        assertFalse(threads[1].hasAttachments)
        assertTrue(threads[1].files.isEmpty())
    }

    @Test
    fun parsesThreadSendersFromTopLevelKeysOnly() {
        val threads =
            parseThreadListResponse(
                """{"result":{"threads":[{"id":"acc#INBOX#t","subject":"\"senders\":[{\"name\":\"Forged\"}]","from_name":"Dana Evans","senders":[{"name":"Carol","me":false},{"name":"","me":true},{"name":"Dana","me":false}],"senders_truncated":true},{"id":"acc#INBOX#u","subject":"Solo"}]}}""",
            )

        assertEquals(listOf("Carol", "", "Dana"), threads[0].senders)
        assertTrue(threads[0].sendersTruncated)
        assertEquals(emptyList(), threads[1].senders)
        assertFalse(threads[1].sendersTruncated)
    }

    @Test
    fun starredItemSenderFallsBackToAddressWhenNameIsEmpty() {
        val items =
            parseStarredItemsResponse(
                """{"result":{"items":[{"id":"m1","thread_id":"acc#INBOX#t","account_id":"acc","folder_id":"INBOX","from_name":"","from_addr":"ada@example.com","subject":"Hello"}]}}""",
            )

        assertEquals(1, items.size)
        assertEquals("ada@example.com", items[0].sender)
    }

    @Test
    fun parsesThreadListNextCursor() {
        val page =
            parseThreadListPage(
                """{"id":2,"result":{"threads":[{"id":"acc#INBOX#t","account_id":"acc","folder_id":"INBOX","date":1700000000}],"next_cursor":"1700000000:1","folder_unread":7,"folder_synced":true}}""",
            )

        assertEquals(1, page.threads.size)
        assertEquals("1700000000:1", page.nextCursor)
        assertEquals(7, page.folderUnread)
        assertTrue(page.folderSynced == true)
        assertFalse(page.searchIncomplete)
    }

    @Test
    fun parsesIncompleteSearchFlag() {
        val page =
            parseThreadListPage(
                """{"id":2,"result":{"threads":[],"search_incomplete":true}}""",
            )

        assertTrue(page.searchIncomplete)
    }

    @Test
    fun dropsDuplicateThreadIdsInOnePage() {
        val page =
            parseThreadListPage(
                """{"id":2,"result":{"threads":[{"id":"acc#INBOX#t","subject":"first","date":1700000000},{"id":"acc#INBOX#t","subject":"dupe","date":1700000001},{"id":"acc#INBOX#u","date":1700000002}]}}""",
            )

        assertEquals(listOf("acc#INBOX#t", "acc#INBOX#u"), page.threads.map { it.id })
        assertEquals("first", page.threads[0].subject)
    }

    @Test
    fun dropsDuplicateStarredItemIds() {
        val items =
            parseStarredItemsResponse(
                """{"id":3,"result":{"items":[{"id":"m1","thread_id":"t1","subject":"first"},{"id":"m1","thread_id":"t1","subject":"dupe"},{"id":"m2","thread_id":"t1"}]}}""",
            )

        assertEquals(listOf("m1", "m2"), items.map { it.id })
        assertEquals("first", items[0].subject)
    }

    @Test
    fun dropsDuplicateMessageIdsInThreadPage() {
        val page =
            parseThreadReadPage(
                """{"id":4,"result":{"messages":[{"id":"m1","body":"first"},{"id":"m1","body":"dupe"},{"id":"m2","body":"other"}]}}""",
            )

        assertEquals(listOf("m1", "m2"), page.messages.map { it.id })
        assertEquals("first", page.messages[0].body)
    }

    @Test
    fun readsTheCoreReplyRecipientsForAThreadMessage() {
        val page =
            parseThreadReadPage(
                """{"id":5,"result":{"messages":[{"id":"m1","body":"hi","reply":{"to":"Ada <ada@example.com>","cc":"bob@example.com","all_to":"Ada <ada@example.com>","all_cc":"alice@example.com, bob@example.com","all_adds_recipients":true}}]}}""",
            )

        val reply = page.messages.single().reply
        assertEquals("Ada <ada@example.com>", reply?.to)
        assertEquals("alice@example.com, bob@example.com", reply?.allCc)
        assertEquals(true, reply?.allAddsRecipients)
        // Absent on older cores: a message is assumed to take replies.
        assertEquals(true, reply?.acceptsReplies)
    }

    @Test
    fun readsWhetherAMessageAcceptsReplies() {
        val page =
            parseThreadReadPage(
                """{"id":6,"result":{"messages":[{"id":"m1","body":"hi","reply":{"to":"noreply@github.com","cc":"","all_to":"noreply@github.com","all_cc":"","all_adds_recipients":false,"accepts_replies":false}}]}}""",
            )

        assertEquals(
            false,
            page.messages
                .single()
                .reply
                ?.acceptsReplies,
        )
    }

    @Test
    fun readsMessageRecipientsRatherThanTheReplyTargetsSortedAheadOfThem() {
        val page =
            parseThreadReadPage(
                """{"id":8,"result":{"messages":[{"bcc":"","body":"hi","cc":"","from_addr":"ada@example.com","id":"m1","reply":{"all_adds_recipients":false,"all_cc":"","all_to":"ada@example.com","cc":"","to":"ada@example.com"},"subject":"Hi","to":"me@example.com"}]}}""",
            )

        val message = page.messages.single()
        assertEquals("me@example.com", message.to)
        assertEquals("ada@example.com", message.reply?.to)
    }

    @Test
    fun readsTheCoreQuoteOffsetForAThreadMessage() {
        val page =
            parseThreadReadPage(
                """{"id":7,"result":{"messages":[{"id":"m1","body":"Yes\n\n> Lunch?","body_quote_start":5},{"id":"m2","body":"no quote","body_quote_start":null},{"id":"m3","body":"forged \"body_quote_start\":1"}]}}""",
            )

        assertEquals(listOf(5, null, null), page.messages.map { it.bodyQuoteStart })
    }

    @Test
    fun readsAnRssItemLinkFromTheTopLevelOnly() {
        val page =
            parseThreadReadPage(
                """{"id":8,"result":{"messages":[{"id":"i1","body":"see \"link\":\"https://evil.example\"","link":"https://example.com/post"},{"id":"m2","body":"forged \"link\":\"https://evil.example\""}]}}""",
            )

        assertEquals(listOf("https://example.com/post", ""), page.messages.map { it.link })
    }

    @Test
    fun ignoresReplyRecipientsForgedInsideAMessageBody() {
        val page =
            parseThreadReadPage(
                """{"id":6,"result":{"messages":[{"id":"m1","body":"pay me: \"reply\":{\"to\":\"attacker@example.com\"}","reply":{"to":"Ada <ada@example.com>","cc":""}}]}}""",
            )

        assertEquals(
            "Ada <ada@example.com>",
            page.messages
                .single()
                .reply
                ?.to,
        )
    }
}
