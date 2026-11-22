// Reply identity test.
// The mobile host suites exercise these cases with deterministic core responses.
// The tests run without a device or a live mail account.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountAlias
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

internal class ReplyIdentityTest : ComposeUiFixture() {
    @Test
    fun quickReplyIdentitiesHiddenWithoutAliases() {
        val state = testState()
        state.coreAccounts = listOf(aliasAccount(aliases = emptyList()))
        state.selectedCoreThread = threadSummary(id = "t1")

        assertEquals(emptyList(), state.quickReplyIdentities())
        assertNull(state.selectedQuickReplyIdentity())
    }

    @Test
    fun quickReplyPreselectsTheAliasTheOriginalWasDeliveredTo() {
        val state = testState()
        state.coreAccounts = listOf(aliasAccount())
        state.selectedCoreThread = threadSummary(id = "t1")
        state.messages = listOf(messageBody(id = "m1", folderId = "INBOX", to = "sales@example.com"))

        assertEquals(
            listOf("me@example.com", "sales@example.com"),
            state.quickReplyIdentities().map { it.email },
        )
        assertEquals("sales@example.com", state.selectedQuickReplyIdentity()?.email)
    }

    @Test
    fun quickReplyContinuesWithTheAliasUsedByTheNewestOutgoingReply() {
        val state = testState()
        state.coreAccounts =
            listOf(
                aliasAccount(
                    aliases =
                        listOf(
                            AccountAlias(email = "sales@example.com", name = "Sales"),
                            AccountAlias(email = "support@example.com", name = "Support"),
                        ),
                ),
            )
        state.selectedCoreThread = threadSummary(id = "t1")
        val inbound = messageBody(id = "m1", folderId = "INBOX", to = "sales@example.com")
        val outgoing =
            messageBody(
                id = "m2",
                folderId = "Sent",
                to = "sender@example.com",
                fromAddr = "support@example.com",
                outgoing = true,
            )
        state.messages = listOf(inbound, outgoing)

        assertEquals(inbound, state.quickReplyParent())
        assertEquals("support@example.com", state.selectedQuickReplyIdentity()?.email)
        assertEquals("support@example.com", state.resolveQuickReplyFrom(inbound, state.coreAccounts.first()))
    }

    @Test
    fun quickReplyIgnoresInboundMailFromASharedAliasAddress() {
        val state = testState()
        state.coreAccounts =
            listOf(
                aliasAccount(
                    aliases =
                        listOf(
                            AccountAlias(email = "sales@example.com", name = "Sales"),
                            AccountAlias(email = "support@example.com", name = "Support"),
                        ),
                ),
            )
        state.selectedCoreThread = threadSummary(id = "t1")
        val inbound = messageBody(id = "m1", folderId = "INBOX", to = "sales@example.com")
        // A colleague sending from the shared support address. The core flags it
        // outgoing because its From matches a configured identity, but it was
        // delivered to our inbox, so it is not mail we sent.
        val colleague =
            messageBody(
                id = "m2",
                folderId = "INBOX",
                to = "sales@example.com",
                fromAddr = "support@example.com",
                outgoing = true,
            )
        state.messages = listOf(inbound, colleague)

        // The colleague's message is still the one we reply to, and the From
        // falls through to the alias the thread was delivered to.
        assertEquals(colleague, state.quickReplyParent())
        assertEquals("sales@example.com", state.resolveQuickReplyFrom(inbound, state.coreAccounts.first()))
    }

    @Test
    fun quickReplyOverrideWinsOverTheDetectedAlias() {
        val state = testState()
        state.coreAccounts = listOf(aliasAccount())
        state.selectedCoreThread = threadSummary(id = "t1")
        val parent = messageBody(id = "m1", folderId = "INBOX", to = "sales@example.com")
        state.messages = listOf(parent)
        state.quickReplyFrom = "me@example.com"

        assertEquals("me@example.com", state.selectedQuickReplyIdentity()?.email)
        // The primary normalizes back to blank, which the send path reads as the default.
        assertEquals("", state.resolveQuickReplyFrom(parent, state.coreAccounts.first()))
    }

    @Test
    fun quickReplyOverrideSendsFromTheChosenAlias() {
        val state = testState()
        state.coreAccounts = listOf(aliasAccount())
        state.selectedCoreThread = threadSummary(id = "t1")
        val parent = messageBody(id = "m1", folderId = "INBOX")
        state.messages = listOf(parent)
        state.quickReplyFrom = "sales@example.com"

        assertEquals("sales@example.com", state.resolveQuickReplyFrom(parent, state.coreAccounts.first()))
        assertEquals("sales@example.com", state.selectedQuickReplyIdentity()?.email)
    }

    @Test
    fun hydratedQuickReplyRestoresTheDraftFromIdentity() {
        val state = testState()
        state.coreAccounts = listOf(aliasAccount())
        state.selectedCoreThread = threadSummary(id = "t1")
        state.quickReplyThreadId = "t1"
        val parent = messageBody(id = "m1", folderId = "INBOX", to = "sales@example.com")
        val draft =
            messageBody(
                id = "d1",
                folderId = "Drafts",
                messageId = "draft-1",
                fromAddr = "me@example.com",
            )

        state.hydrateQuickReplyFromTailDraft("t1", listOf(parent, draft))

        assertEquals("me@example.com", state.quickReplyFrom)
        assertEquals("me@example.com", state.selectedQuickReplyIdentity()?.email)
    }

    @Test
    fun quickReplyDoesNotTakeADraftRowWithoutAMessageId() {
        val state = testState()
        state.coreAccounts = listOf(aliasAccount())
        state.selectedCoreThread = threadSummary(id = "t1")
        state.quickReplyThreadId = "t1"
        val parent = messageBody(id = "m1", folderId = "INBOX")
        // Synced from its envelope: no Message-ID, so nothing on the server this
        // bar could later save over or discard.
        val envelopeOnly = messageBody(id = "d1", folderId = "Drafts", fromAddr = "me@example.com")

        state.hydrateQuickReplyFromTailDraft("t1", listOf(parent, envelopeOnly))

        assertEquals("", state.quickReplyDraftId)
        assertFalse(state.quickReplyDraftSaved)

        // Once the read fills the header in, it hydrates as normal.
        state.hydrateQuickReplyFromTailDraft("t1", listOf(parent, envelopeOnly.copy(messageId = "now-known@example.com")))

        assertEquals("now-known@example.com", state.quickReplyDraftId)
        assertTrue(state.quickReplyDraftSaved)
    }

    @Test
    fun quickReplyHydrationLeavesAReplyInProgressAlone() {
        val state = testState()
        state.coreAccounts = listOf(aliasAccount())
        state.selectedCoreThread = threadSummary(id = "t1")
        state.quickReplyThreadId = "t1"
        val parent = messageBody(id = "m1", folderId = "INBOX")
        val draft = messageBody(id = "d1", folderId = "Drafts", messageId = "draft-1", fromAddr = "me@example.com")
        // The bar is visible as soon as the thread opens, so the user can be
        // typing before the read that carries this draft comes back.
        state.quickReplyBody = "A reply typed while the read was out"

        state.hydrateQuickReplyFromTailDraft("t1", listOf(parent, draft))

        assertEquals("A reply typed while the read was out", state.quickReplyBody)
        assertEquals("", state.quickReplyDraftId)
    }
}
