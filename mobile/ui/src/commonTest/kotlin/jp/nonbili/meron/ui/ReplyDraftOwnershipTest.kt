// Reply draft ownership test.
// The mobile host suites exercise these cases with deterministic core responses.
// The tests run without a device or a live mail account.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.ThreadSummary
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.yield
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

internal class ReplyDraftOwnershipTest : ComposeLifecycleFixture() {
    @Test
    fun completedQuickReplySaveSurvivesNavigationWithoutMutatingTheNewThread() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            state.selectedCoreThread =
                ThreadSummary(id = "old", accountId = "a", folder = "INBOX", subject = "Old", sender = "old@example.com")
            state.quickReplyThreadId = "old"
            state.messages =
                listOf(
                    MessageBody(
                        id = "old-message",
                        folderId = "INBOX",
                        from = "Old",
                        fromAddr = "old@example.com",
                        to = "a@example.com",
                        subject = "Old",
                        body = "Original",
                        messageId = "old@example.com",
                    ),
                )
            state.quickReplyBody = "Saved reply"
            state.quickReplyDraftId = "existing-reply@example.com"
            state.quickReplyDraftSaved = true

            state.autoSaveQuickReplyDraft()
            core.firstSaveStarted.await()
            state.selectedCoreThread =
                ThreadSummary(id = "new", accountId = "a", folder = "INBOX", subject = "New", sender = "new@example.com")
            state.quickReplyThreadId = "new"
            state.quickReplyBody = "New thread reply"
            state.quickReplyDraftId = "new-reply@example.com"
            ++state.quickReplyGeneration
            core.releaseFirstSave.complete(Unit)
            core.firstSaveFinished.await()
            yield()

            assertTrue(core.discardPayloads.isEmpty())
            assertEquals("new", state.quickReplyThreadId)
            assertEquals("New thread reply", state.quickReplyBody)
            assertEquals("new-reply@example.com", state.quickReplyDraftId)
        }

    @Test
    fun obsoleteQuickReplySavePublishesItsNewRemoteIdForTheSameEditor() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state)
            state.onQuickReplyBodyChange("First version")

            state.autoSaveQuickReplyDraft()
            core.firstSaveStarted.await()
            state.onQuickReplyBodyChange("Second version")
            core.releaseFirstSave.complete(Unit)
            core.firstSaveFinished.await()
            withTimeout(1_000) {
                while (state.quickReplyDraftId.isBlank()) yield()
            }
            assertEquals("draft-1@example.com", state.quickReplyDraftId)

            state.autoSaveQuickReplyDraft()
            core.secondSaveFinished.await()

            assertEquals(1, core.allocationCalls)
            assertTrue(core.savedPayloads[1].contains("\"draft_id\":\"draft-1@example.com\""))
            assertTrue(core.discardPayloads.isEmpty())
        }

    @Test
    fun quickReplySendWaitingForSaveDiscardsTheNewlyAllocatedDraft() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state)
            state.onQuickReplyBodyChange("Reply")

            state.autoSaveQuickReplyDraft()
            core.firstSaveStarted.await()
            state.sendQuickReply()
            yield()
            assertEquals(0, core.sendCalls)
            core.releaseFirstSave.complete(Unit)
            core.sendFinished.await()
            core.discardFinished.await()

            assertEquals(2, core.allocationCalls)
            assertTrue(core.discardPayloads.single().contains("draft-1@example.com"))
        }

    @Test
    fun quickReplyUsesBackendThreadIdForAutosaveAndSendOwnership() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state, rowId = "account#INBOX#thread", backendThreadId = "account#thread")
            state.onQuickReplyBodyChange("Reply")

            state.autoSaveQuickReplyDraft()
            core.firstSaveStarted.await()
            core.releaseFirstSave.complete(Unit)
            core.firstSaveFinished.await()
            withTimeout(1_000) {
                while (state.quickReplyDraftId.isBlank()) yield()
            }

            assertEquals("draft-1@example.com", state.quickReplyDraftId)
            state.sendQuickReply()
            core.sendFinished.await()
            core.discardFinished.await()
            withTimeout(1_000) {
                while (state.quickReplySendInFlight) yield()
            }

            assertEquals(1, core.saveCalls)
            assertTrue(core.discardPayloads.single().contains("draft-1@example.com"))
        }

    @Test
    fun sendDoesNotClaimAMatchingLoadedDraftWithoutOwnership() =
        runBlocking {
            val unrelatedDraft = consumedDraft().copy(messageId = "meron-draft-full-editor@gmail.com")
            val core = SaveCore().apply { threadReadResponse = threadReadWithConsumedDraft(unrelatedDraft.messageId) }
            val state = state(core, this)
            prepareQuickReply(state)
            state.messages = state.messages + unrelatedDraft
            state.onQuickReplyBodyChange("Reply")

            state.sendQuickReply()
            core.sendFinished.await()
            awaitState { !state.quickReplySendInFlight && core.threadReadCalls > 0 }

            assertTrue(core.discardPayloads.isEmpty())
            assertTrue(state.messages.any { it.id == "draft-row" })
        }

    @Test
    fun clearingDraftMarkerRemovesRowAndBackendKeys() =
        runBlocking {
            val state = state(SaveCore(), this)
            prepareQuickReply(state, rowId = "account#INBOX#thread", backendThreadId = "account#thread")
            state.locallyDraftedThreadIds = setOf("account#INBOX#thread", "account#thread")

            state.clearThreadDraftEverywhere("account#thread")

            assertTrue(state.locallyDraftedThreadIds.isEmpty())
        }

    @Test
    fun failedPostSendDiscardKeepsTheStaleDraftOutOfTheReplyBar() =
        runBlocking {
            val core =
                SaveCore().apply {
                    discardFails = true
                    threadReadResponse = threadReadWithConsumedDraft()
                }
            val state = state(core, this)
            state.sentDraftDiscardRetryDelaysMs = emptyList()
            prepareQuickReply(state)
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true
            state.selectedCoreThread = state.selectedCoreThread?.copy(hasDraft = true)
            state.onQuickReplyBodyChange("Reply")

            state.sendQuickReply()
            core.sendFinished.await()
            core.discardFinished.await()
            withTimeout(1_000) {
                while (state.quickReplySendInFlight || core.threadReadCalls == 0 || state.messages.none { it.id == "draft-row" }) yield()
            }

            // Still on the server, so the thread keeps its draft marker; but the
            // reply already went out, so the copy stays held back from the
            // conversation and the reply bar rather than offered up again.
            assertTrue(state.messages.any { it.id == "draft-row" })
            assertEquals(true, state.selectedCoreThread?.hasDraft)
            assertTrue("reply-draft@example.com" in state.quickReplyConsumedDraftIds)
            assertTrue(state.visibleThreadMessages().none { it.id == "draft-row" })
        }

    @Test
    fun markingDraftUsesBackendKey() =
        runBlocking {
            val state = state(SaveCore(), this)
            prepareQuickReply(state, rowId = "account#INBOX#thread", backendThreadId = "account#thread")

            state.markThreadDraftEverywhere("account#INBOX#thread")

            assertEquals(setOf("account#thread"), state.locallyDraftedThreadIds)
        }

    @Test
    fun reopeningTheThreadWhileTheSentDraftIsBeingDiscardedKeepsTheReplyBarClear() =
        runBlocking {
            val core = SaveCore().apply { holdDiscard = CompletableDeferred() }
            val state = state(core, this)
            prepareQuickReply(state)
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true
            state.onQuickReplyBodyChange("Reply")

            state.sendQuickReply()
            core.sendFinished.await()
            core.discardFinished.await()

            // The user backs out to another conversation and comes straight
            // back. Reopening resets the reply bar, and the read still returns
            // the consumed draft — the discard has not landed yet — as the
            // conversation tail.
            state.quickReplyThreadId = "other"
            state.quickReplyBody = ""
            state.quickReplyDraftId = ""
            state.quickReplyDraftSaved = false
            state.quickReplyThreadId = "thread"
            state.hydrateQuickReplyFromTailDraft("thread", state.messages + consumedDraft())

            assertEquals("", state.quickReplyBody)
            assertEquals("", state.quickReplyDraftId)

            core.holdDiscard!!.complete(Unit)
            withTimeout(1_000) {
                while (state.quickReplySendInFlight) yield()
            }
            // The hold outlives the send flag by design: it is released only
            // once the refreshes that follow the send have settled.
            awaitState { state.quickReplyConsumedDraftIds.isEmpty() }
        }

    @Test
    fun leavingTheThreadDuringTheSendStillDiscardsTheDraftItConsumed() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state)
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true
            state.onQuickReplyBodyChange("Reply")

            state.sendQuickReply()
            // The user opens another conversation while the send is out, which
            // resets the bar and moves the editor generation on.
            state.quickReplyThreadId = "other"
            state.quickReplyDraftId = ""
            state.quickReplyDraftSaved = false
            ++state.quickReplyGeneration
            withTimeout(5_000) {
                core.sendFinished.await()
                core.discardFinished.await()
            }

            // Nothing points at that draft any more; leaving it is what puts it
            // in Drafts next to the reply it was sent as.
            assertTrue(core.discardPayloads.single().contains("reply-draft@example.com"))
        }

    @Test
    fun aConsumedDraftStaysHiddenOnlyUntilItsDiscardSettles() =
        runBlocking {
            val core = SaveCore().apply { holdDiscard = CompletableDeferred() }
            val state = state(core, this)
            prepareQuickReply(state)
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true
            state.onQuickReplyBodyChange("Reply")

            state.sendQuickReply()
            withTimeout(5_000) {
                core.sendFinished.await()
                core.discardFinished.await()
            }

            // Reopening clears the bar's id, so only the send's own hold is left
            // to keep the draft it consumed out of the conversation.
            state.quickReplyThreadId = "other"
            state.quickReplyBody = ""
            state.quickReplyDraftId = ""
            state.quickReplyDraftSaved = false
            state.quickReplyThreadId = "thread"
            state.messages = state.messages + consumedDraft()

            assertTrue(state.visibleThreadMessages().none { it.id == "draft-row" })

            core.holdDiscard!!.complete(Unit)
            withTimeout(1_000) {
                while (state.quickReplySendInFlight) yield()
            }
            // Held until the post-send refreshes settle, so the conversation is
            // never painted with the draft beside the reply it was sent as.
            awaitState { state.quickReplyConsumedDraftIds.isEmpty() }
            assertTrue(state.messages.none { it.id == "draft-row" })
            state.messages = state.messages + consumedDraft()
            // The send's hold is gone, but the discard succeeded: a later row is a
            // read that set off before it, so the draft stays hidden for good.
            assertTrue(state.visibleThreadMessages().none { it.id == "draft-row" })
        }

    @Test
    fun openingADraftWithoutAMessageIdDoesNotClaimItAsSaved() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            val thread =
                ThreadSummary(id = "thread", accountId = "a", folder = "Drafts", subject = "Subject", sender = "a@example.com")
            val draft =
                MessageBody(
                    id = "draft-row",
                    folderId = "Drafts",
                    from = "A",
                    fromAddr = "a@example.com",
                    to = "you@example.com",
                    subject = "Subject",
                    body = "Half written",
                    messageId = "",
                )

            state.openDraftCompose(draft, thread)
            withTimeout(5_000) { while (state.screen != Screen.Compose) yield() }

            // Nothing on the server answers to this id, so a discard would find
            // nothing and a save must create the draft rather than replace one.
            assertEquals(false, state.composeDraftSaved)
            assertTrue(state.composeDraftId.startsWith("local-draft-"))

            state.openDraftCompose(draft.copy(messageId = "real-draft@example.com"), thread)
            withTimeout(5_000) { while (state.composeDraftId != "real-draft@example.com") yield() }

            assertTrue(state.composeDraftSaved)
        }

    @Test
    fun aSendNeverAdoptsTheDraftOfAThreadOpenedWhileItWasOut() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state)
            state.onQuickReplyBodyChange("Reply")

            // Nothing saved yet, so the send has no draft of its own to claim.
            state.sendQuickReply()
            // Another conversation opens while the send is out and hydrates its
            // own saved draft into the bar.
            state.quickReplyThreadId = "other-thread"
            state.quickReplyDraftId = "someone-elses-draft@example.com"
            state.quickReplyDraftSaved = true
            ++state.quickReplyGeneration
            withTimeout(5_000) { core.sendFinished.await() }
            withTimeout(5_000) {
                while (state.quickReplySendInFlight) yield()
            }

            // That draft belongs to a thread this send never touched.
            assertTrue(core.discardPayloads.isEmpty())
            assertEquals("someone-elses-draft@example.com", state.quickReplyDraftId)
        }

    @Test
    fun aDraftRehydratedDuringTheSendIsStillDiscarded() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state)
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true
            state.onQuickReplyBodyChange("Reply")

            state.sendQuickReply()
            // Leaving and reopening the conversation during the send: the read
            // still returns the consumed draft as the tail.
            state.quickReplyThreadId = "other"
            state.quickReplyBody = ""
            state.quickReplyDraftId = ""
            state.quickReplyDraftSaved = false
            state.quickReplyThreadId = "thread"
            state.hydrateQuickReplyFromTailDraft("thread", state.messages + consumedDraft())

            // The hold runs from the click, so the sent text cannot come back
            // into the bar and be mistaken for a newer reply.
            assertEquals("", state.quickReplyDraftId)
            withTimeout(5_000) {
                core.sendFinished.await()
                core.discardFinished.await()
            }
            assertTrue(core.discardPayloads.single().contains("reply-draft@example.com"))
        }

    @Test
    fun aHandedOverDraftIsHeldAndSettledBeforeTheIdentityAllocation() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state)
            state.onQuickReplyBodyChange("Reply")
            // An autosave from before the click landed while the bar had already
            // moved on, so it handed its id to the send rather than the bar.
            state.quickReplySendDraftHandover = "thread" to "handed-over@example.com"

            state.sendQuickReply()
            // Reopening the conversation while the send is still allocating must
            // not pull the text being sent back into the bar.
            state.hydrateQuickReplyFromTailDraft(
                "thread",
                state.messages + consumedDraft().copy(messageId = "handed-over@example.com"),
            )
            assertEquals("", state.quickReplyDraftId)

            withTimeout(5_000) {
                core.sendFinished.await()
                core.discardFinished.await()
            }
            assertTrue(core.discardPayloads.single().contains("handed-over@example.com"))
            assertEquals(null, state.quickReplySendDraftHandover)
        }
}
