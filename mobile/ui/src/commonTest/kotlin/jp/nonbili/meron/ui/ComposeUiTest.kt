// Compose fields accept typed recipient lists as well
// as individual addresses. These tests exercise
// parsing and editing behavior used by the mobile
// composer.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.DraftAttachment
import jp.nonbili.meron.shared.SignatureMark
import jp.nonbili.meron.shared.SignaturePlacement
import jp.nonbili.meron.shared.bodyWithSignature
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

internal class ComposeUiTest : ComposeUiFixture() {
    @Test
    fun testParseRecipientsEmpty() {
        val (completed, active) = parseRecipients("")
        assertEquals(emptyList(), completed)
        assertEquals("", active)
    }

    @Test
    fun testParseRecipientsSingleNoComma() {
        val (completed, active) = parseRecipients("alice@example.com")
        assertEquals(emptyList(), completed)
        assertEquals("alice@example.com", active)
    }

    @Test
    fun testParseRecipientsWithCommas() {
        val (completed, active) = parseRecipients("alice@example.com, bob@example.com, ch")
        assertEquals(listOf("alice@example.com", "bob@example.com"), completed)
        assertEquals("ch", active)
    }

    @Test
    fun testParseRecipientsWithTrailingComma() {
        val (completed, active) = parseRecipients("alice@example.com, ")
        assertEquals(listOf("alice@example.com"), completed)
        assertEquals("", active)
    }

    @Test
    fun testParseRecipientsKeepsQuotedCommaNameAsOneChip() {
        val (completed, active) = parseRecipients("\"Doe, Jane\" <jane@example.com>, bo")
        assertEquals(listOf("\"Doe, Jane\" <jane@example.com>"), completed)
        assertEquals("bo", active)
    }

    @Test
    fun testParseEmailRecipientRawEmail() {
        val (name, email) = parseEmailRecipient("alice@example.com")
        assertEquals("", name)
        assertEquals("alice@example.com", email)
    }

    @Test
    fun testParseEmailRecipientWithNameAngleBrackets() {
        val (name, email) = parseEmailRecipient("Alice Smith <alice@example.com>")
        assertEquals("Alice Smith", name)
        assertEquals("alice@example.com", email)
    }

    @Test
    fun testParseEmailRecipientWithNameParentheses() {
        val (name, email) = parseEmailRecipient("alice@example.com (Alice Smith)")
        assertEquals("Alice Smith", name)
        assertEquals("alice@example.com", email)
    }

    @Test
    fun composeAutosaveSnapshotIgnoresEmptyDrafts() {
        assertNull(
            composeAutosaveSnapshot(
                selectedFromKey = "acc|me@example.com",
                to = " ",
                cc = "",
                bcc = "",
                subject = "",
                body = "",
                attachments = emptyList(),
            ),
        )
    }

    @Test
    fun composeAutosaveSnapshotIncludesRecipientSubjectAndBodyContent() {
        val snapshot =
            composeAutosaveSnapshot(
                selectedFromKey = "acc|me@example.com",
                to = "you@example.com",
                cc = "",
                bcc = "",
                subject = "Hello",
                body = "Body",
                attachments = emptyList(),
            )

        assertNotNull(snapshot)
        assertEquals("you@example.com", snapshot.to)
        assertEquals("Hello", snapshot.subject)
        assertEquals("Body", snapshot.body)
    }

    @Test
    fun composeAutosaveSnapshotIncludesAttachmentOnlyDrafts() {
        val attachment = DraftAttachment(id = "att1", displayName = "note.txt")
        val snapshot =
            composeAutosaveSnapshot(
                selectedFromKey = "acc|me@example.com",
                to = "",
                cc = "",
                bcc = "",
                subject = "",
                body = "",
                attachments = listOf(attachment),
            )

        assertNotNull(snapshot)
        assertEquals(listOf(attachment), snapshot.attachments)
    }

    @Test
    fun composeAutosaveSnapshotChangesWhenContentChanges() {
        val first =
            composeAutosaveSnapshot(
                selectedFromKey = "acc|me@example.com",
                to = "you@example.com",
                cc = "",
                bcc = "",
                subject = "Hello",
                body = "First",
                attachments = emptyList(),
            )
        val second =
            composeAutosaveSnapshot(
                selectedFromKey = "acc|me@example.com",
                to = "you@example.com",
                cc = "",
                bcc = "",
                subject = "Hello",
                body = "Second",
                attachments = emptyList(),
            )

        assertNotEquals(first, second)
    }

    @Test
    fun freshComposeClearsAReplyLeftByThePreviouslyOpenedThread() {
        val state = testState()
        state.appSignatureLoaded = true
        state.to = "sender@example.com"
        state.cc = "copy@example.com"
        state.subject = "Re: Subject"
        state.body = "Reply body"
        state.composeInReplyTo = "message@example.com"
        state.composeReferences = "root@example.com message@example.com"
        state.composeDraftId = "reply-draft@example.com"
        state.composeDraftSaved = true

        state.openCompose()

        assertEquals("", state.to)
        assertEquals("", state.cc)
        assertEquals("", state.subject)
        assertEquals("", state.composeInReplyTo)
        assertEquals("", state.composeReferences)
        assertEquals("", state.composeDraftId)
        assertFalse(state.composeDraftSaved)
    }

    @Test
    fun threadsWithDraftFlagMarksVisibleThreadListRow() {
        val threads =
            listOf(
                threadSummary(id = "acc#INBOX#one"),
                threadSummary(id = "acc#INBOX#two"),
            )

        val updated = threadsWithDraftFlag(threads, "acc#INBOX#two")

        assertFalse(updated[0].hasDraft)
        assertTrue(updated[1].hasDraft)
    }

    @Test
    fun threadsWithDraftFlagCanMatchThreadIdAlias() {
        val threads =
            listOf(
                threadSummary(id = "row-1", threadId = "acc#INBOX#one"),
            )

        val updated = threadsWithDraftFlag(threads, "acc#INBOX#one")

        assertTrue(updated.single().hasDraft)
    }

    @Test
    fun draftThreadWithCachedAncestorOpensConversation() {
        val messages =
            listOf(
                messageBody(id = "m1", folderId = "INBOX"),
                messageBody(id = "d1", folderId = "Drafts"),
            )

        assertTrue(draftThreadShouldOpenConversation(messages))
    }

    @Test
    fun referencedDraftThreadOpensConversation() {
        val messages =
            listOf(
                messageBody(id = "d1", folderId = "Drafts", references = "root@example.com"),
            )

        assertTrue(draftThreadShouldOpenConversation(messages))
    }

    @Test
    fun inReplyToDraftThreadOpensConversation() {
        val messages =
            listOf(
                messageBody(id = "d1", folderId = "Drafts", inReplyTo = "root@example.com"),
            )

        assertTrue(draftThreadShouldOpenConversation(messages))
    }

    @Test
    fun standaloneDraftThreadOpensComposer() {
        val messages =
            listOf(
                messageBody(id = "d1", folderId = "Drafts"),
            )

        assertFalse(draftThreadShouldOpenConversation(messages))
    }

    @Test
    fun visibleThreadMessagesHidesTailDraftHydratedIntoQuickReply() {
        val state = testState()
        state.messages =
            listOf(
                messageBody(id = "m1", folderId = "INBOX"),
                messageBody(id = "d1", folderId = "Drafts", messageId = "draft-1"),
            )
        state.quickReplyDraftId = "draft-1"

        val visible = state.visibleThreadMessages()

        assertEquals(listOf("m1"), visible.map { it.id })
    }

    @Test
    fun visibleThreadMessagesHidesHydratedDraftBeforeOptimisticSend() {
        val state = testState()
        state.messages =
            listOf(
                messageBody(id = "d1", folderId = "Drafts", messageId = "draft-1"),
                messageBody(id = "m2", folderId = "INBOX"),
            )
        state.quickReplyDraftId = "draft-1"

        val visible = state.visibleThreadMessages()

        assertEquals(listOf("m2"), visible.map { it.id })
    }

    @Test
    fun visibleThreadMessagesKeepsUnrelatedOlderDraftVisible() {
        val state = testState()
        state.messages =
            listOf(
                messageBody(id = "d0", folderId = "Drafts", messageId = "draft-0"),
                messageBody(id = "d1", folderId = "Drafts", messageId = "draft-1"),
            )
        state.quickReplyDraftId = "draft-1"

        val visible = state.visibleThreadMessages()

        assertEquals(listOf("d0"), visible.map { it.id })
    }

    @Test
    fun composeIsBlankIgnoresTheSeededSignature() {
        val state = testState()
        state.composeSignature = SignatureMark("-- \nPing", SignaturePlacement.BelowText)
        state.body = bodyWithSignature("", "-- \nPing")

        assertTrue(state.composeIsBlank())
    }

    @Test
    fun composeIsBlankSeesTextWrittenAboveTheSignature() {
        val state = testState()
        state.composeSignature = SignatureMark("-- \nPing", SignaturePlacement.BelowText)
        state.body = bodyWithSignature("Hello", "-- \nPing")

        assertFalse(state.composeIsBlank())
    }

    @Test
    fun composeIsBlankIgnoresAReplysPrefilledHeaders() {
        val state = testState()
        state.composeSignature = SignatureMark("-- \nPing", SignaturePlacement.BelowText)
        state.body = bodyWithSignature("", "-- \nPing")
        state.to = "noreply@example.com"
        state.subject = "Re: Hello"
        state.rememberComposeSeed()

        assertTrue(state.composeIsBlank())
    }

    @Test
    fun composeIsBlankIgnoresTheChipFieldsTrailingSeparator() {
        val state = testState()
        state.to = "Ray Dalio <ray@example.com>"
        state.subject = "Re: Hello"
        state.rememberComposeSeed()
        // What RecipientChipsInput does to a prefilled field on first render.
        state.to = "Ray Dalio <ray@example.com>, "

        assertTrue(state.composeIsBlank())
    }

    @Test
    fun composeIsBlankSeesARecipientTheUserAdded() {
        val state = testState()
        state.composeSignature = SignatureMark("-- \nPing", SignaturePlacement.BelowText)
        state.body = bodyWithSignature("", "-- \nPing")
        state.to = "noreply@example.com"
        state.subject = "Re: Hello"
        state.rememberComposeSeed()

        state.cc = "someone@example.com"

        assertFalse(state.composeIsBlank())
    }

    @Test
    fun composeIsBlankSeesAnAttachmentOnItsOwn() {
        val state = testState()
        state.composeSignature = SignatureMark("-- \nPing", SignaturePlacement.BelowText)
        state.body = bodyWithSignature("", "-- \nPing")
        state.attachments = listOf(DraftAttachment(id = "a1", displayName = "a.txt", mimeType = "text/plain", sizeBytes = 1))

        assertFalse(state.composeIsBlank())
    }

    @Test
    fun removeDiscardedDraftFromOpenThreadDropsCachedDraftAfterSend() {
        val state = testState()
        state.selectedCoreThread = threadSummary(id = "t1").copy(hasDraft = true)
        state.coreThreads = listOf(threadSummary(id = "t1").copy(hasDraft = true))
        state.locallyDraftedThreadIds = setOf("t1")
        state.messages =
            listOf(
                messageBody(id = "m1", folderId = "INBOX"),
                messageBody(id = "d1", folderId = "Drafts", messageId = "draft-1"),
                messageBody(id = "local-send-1", folderId = "INBOX"),
            )

        val cleared = state.removeDiscardedDraftFromOpenThread("draft-1")

        assertEquals(listOf("m1", "local-send-1"), state.messages.map { it.id })
        assertEquals("t1", cleared)
        assertFalse(state.selectedCoreThread!!.hasDraft)
        assertFalse(state.coreThreads.single().hasDraft)
        assertTrue(state.locallyDraftedThreadIds.isEmpty())
    }

    @Test
    fun removeDiscardedDraftFromOpenThreadClearsMarkerForAnAutosavedQuickReply() {
        val state = testState()
        state.selectedCoreThread = threadSummary(id = "t1").copy(hasDraft = true)
        state.coreThreads = listOf(threadSummary(id = "t1").copy(hasDraft = true))
        state.locallyDraftedThreadIds = setOf("t1")
        // A quick-reply autosave marks the thread without adding a draft message.
        state.messages = listOf(messageBody(id = "m1", folderId = "INBOX"))

        val cleared = state.removeDiscardedDraftFromOpenThread("draft-1", "t1")

        assertEquals("t1", cleared)
        assertEquals(listOf("m1"), state.messages.map { it.id })
        assertFalse(state.selectedCoreThread!!.hasDraft)
        assertFalse(state.coreThreads.single().hasDraft)
        assertTrue(state.locallyDraftedThreadIds.isEmpty())
    }

    @Test
    fun removeDiscardedDraftFromOpenThreadKeepsMarkerWhenAnotherDraftRemains() {
        val state = testState()
        state.selectedCoreThread = threadSummary(id = "t1").copy(hasDraft = true)
        state.locallyDraftedThreadIds = setOf("t1")
        state.messages =
            listOf(
                messageBody(id = "d0", folderId = "Drafts", messageId = "draft-0"),
                messageBody(id = "d1", folderId = "Drafts", messageId = "draft-1"),
            )

        val cleared = state.removeDiscardedDraftFromOpenThread("draft-1", "t1")

        assertNull(cleared)
        assertEquals(listOf("d0"), state.messages.map { it.id })
        assertTrue(state.selectedCoreThread!!.hasDraft)
        assertEquals(setOf("t1"), state.locallyDraftedThreadIds)
    }

    @Test
    fun removeDiscardedDraftFromOpenThreadKeepsMarkerWhileMessagesAreStillLoading() {
        val state = testState()
        state.selectedCoreThread = threadSummary(id = "t1").copy(hasDraft = true)
        state.coreThreads = listOf(threadSummary(id = "t1").copy(hasDraft = true))
        state.locallyDraftedThreadIds = setOf("t1")
        state.messages = emptyList()

        val cleared = state.removeDiscardedDraftFromOpenThread("draft-1")

        assertNull(cleared)
        assertTrue(state.selectedCoreThread!!.hasDraft)
        assertTrue(state.coreThreads.single().hasDraft)
        assertEquals(setOf("t1"), state.locallyDraftedThreadIds)
    }

    @Test
    fun removeDiscardedDraftFromOpenThreadLeavesAnotherThreadsDraftAlone() {
        val state = testState()
        state.selectedCoreThread = threadSummary(id = "t1").copy(hasDraft = true)
        state.coreThreads = listOf(threadSummary(id = "t1").copy(hasDraft = true))
        state.locallyDraftedThreadIds = setOf("t1")
        state.messages = listOf(messageBody(id = "d1", folderId = "Drafts", messageId = "draft-1"))

        val cleared = state.removeDiscardedDraftFromOpenThread("draft-elsewhere")

        assertNull(cleared)
        assertEquals(listOf("d1"), state.messages.map { it.id })
        assertTrue(state.selectedCoreThread!!.hasDraft)
        assertEquals(setOf("t1"), state.locallyDraftedThreadIds)
    }

    @Test
    fun visibleThreadMessagesReturnsAllWhenNoQuickReplyDraftHydrated() {
        val state = testState()
        state.messages =
            listOf(
                messageBody(id = "m1", folderId = "INBOX"),
                messageBody(id = "d1", folderId = "Drafts", messageId = "draft-1"),
            )

        assertEquals(state.messages, state.visibleThreadMessages())
    }
}
