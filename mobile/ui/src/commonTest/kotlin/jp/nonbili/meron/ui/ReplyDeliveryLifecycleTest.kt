// Reply delivery lifecycle test.
// The mobile host suites exercise these cases with deterministic core responses.
// The tests run without a device or a live mail account.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.SendStatus
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.yield
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

internal class ReplyDeliveryLifecycleTest : ComposeLifecycleFixture() {
    @Test
    fun theClickEmptiesTheBarAndPutsTheBubbleUpBeforeAnyRoundTrip() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state)
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true
            state.onQuickReplyBodyChange("Reply")

            state.sendQuickReply()

            // Nothing has suspended yet, so this is what the user sees the
            // instant they tap: an empty box and their reply already in the
            // conversation, not a bar that sits full while the network works.
            assertEquals(0, core.allocationCalls)
            assertTrue(state.quickReplyIsBlank())
            assertEquals(SendStatus.Sending, state.messages.last().sendStatus)
            assertEquals("Reply", state.messages.last().body)
            // The draft stays claimed: a failure falls back on it, and edits
            // made after one have to write into that same copy.
            assertEquals("reply-draft@example.com", state.quickReplyDraftId)

            withTimeout(5_000) {
                core.sendFinished.await()
                core.discardFinished.await()
            }
            awaitState { state.quickReplyDraftId.isEmpty() }
        }

    @Test
    fun aFailedAllocationTakesTheOptimisticBubbleBackDown() =
        runBlocking {
            val core = SaveCore().apply { allocationFails = true }
            val state = state(core, this)
            prepareQuickReply(state)
            state.onQuickReplyBodyChange("Reply")

            state.sendQuickReply()
            assertEquals(SendStatus.Sending, state.messages.last().sendStatus)
            withTimeout(5_000) { while (state.quickReplySendInFlight) yield() }

            // Nothing was sent, so no bubble should be left claiming otherwise.
            assertTrue(state.messages.none { it.sendStatus == SendStatus.Sending })
        }

    @Test
    fun aFailedAllocationPutsTheReplyBackInTheBar() =
        runBlocking {
            val core = SaveCore().apply { allocationFails = true }
            core.releaseFirstSave.complete(Unit)
            val state = state(core, this)
            prepareQuickReply(state)
            state.onQuickReplyBodyChange("Reply")

            state.sendQuickReply()
            assertTrue(state.quickReplyIsBlank())
            withTimeout(5_000) { while (state.quickReplySendInFlight) yield() }

            // No send was dispatched, so Retry has nothing to resend. The reply
            // has to come back to the box or it is simply gone.
            assertEquals("Reply", state.quickReplyBody)
            assertTrue(state.quickReplyFailure.isNotBlank())
        }

    @Test
    fun aReplyStartedDuringTheSendIsSavedOnceTheSendSettles() =
        runBlocking {
            val core = SaveCore().apply { holdSend = CompletableDeferred() }
            core.releaseFirstSave.complete(Unit)
            val state = state(core, this)
            prepareQuickReply(state)
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true
            state.onQuickReplyBodyChange("First")

            state.sendQuickReply()
            withTimeout(5_000) { core.sendFinished.await() }
            // The bar is empty from the click, so the next reply gets typed while
            // the first is still going out — and its debounce expires in there,
            // where saving is refused.
            state.onQuickReplyBodyChange("Second")
            delay(1_600)
            assertTrue(state.quickReplyAutosaveDeferred)

            core.holdSend!!.complete(Unit)
            withTimeout(5_000) { while (state.quickReplySendInFlight) yield() }

            // The bar kept the draft for this newer reply, so the send left it
            // alone — which is only correct if the newer text actually reaches it.
            awaitState { core.savedPayloads.any { it.contains("Second") } }
            assertTrue(core.discardPayloads.isEmpty())
        }

    @Test
    fun clearingANewerReplyMidSendKeepsTheSafetyDraft() =
        runBlocking {
            val core =
                SaveCore().apply {
                    sendFails = true
                    holdSend = CompletableDeferred()
                }
            core.releaseFirstSave.complete(Unit)
            val state = state(core, this)
            prepareQuickReply(state)
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true
            state.onQuickReplyBodyChange("First")

            state.sendQuickReply()
            withTimeout(5_000) { core.sendFinished.await() }
            // Typed and then thought better of, while the first is still out.
            state.onQuickReplyBodyChange("Second")
            state.onQuickReplyBodyChange("")
            state.discardQuickReplyDraftIfEmpty()

            // That draft is what the failing send falls back on. Refusing it is
            // synchronous — the id stays claimed and nothing is launched.
            assertEquals("reply-draft@example.com", state.quickReplyDraftId)
            yield()
            assertTrue(core.discardPayloads.isEmpty())
            core.holdSend!!.complete(Unit)
            withTimeout(5_000) { while (state.quickReplySendInFlight) yield() }
            assertTrue(core.discardPayloads.isEmpty())
        }

    @Test
    fun aFailedSendMarksItsBubbleEvenAfterTheBarMovesOn() =
        runBlocking {
            val core =
                SaveCore().apply {
                    sendFails = true
                    holdSend = CompletableDeferred()
                }
            core.releaseFirstSave.complete(Unit)
            val state = state(core, this)
            prepareQuickReply(state)
            state.onQuickReplyBodyChange("First")

            state.sendQuickReply()
            val bubbleId = state.messages.last().id
            withTimeout(5_000) { core.sendFinished.await() }
            // The user starts their next reply while the first is still out.
            state.onQuickReplyBodyChange("Second")
            core.holdSend!!.complete(Unit)
            withTimeout(5_000) { while (state.quickReplySendInFlight) yield() }

            // "Sending…" is the one thing this bubble must not still claim.
            assertEquals(SendStatus.Failed, state.messages.single { it.id == bubbleId }.sendStatus)
            assertEquals(true, state.pendingQuickReplySend != null)
        }

    @Test
    fun deletingAVisibleDraftKeepsTheBadgeWhileTheBarHoldsItsOwn() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state)
            val draftRow = consumedDraft()
            state.messages = state.messages + draftRow
            state.coreThreads = listOfNotNull(state.selectedCoreThread)
            state.markThreadDraftEverywhere("thread")
            // Saved from the bar, so it is on the server with no row of its own.
            state.quickReplyDraftId = "bar-draft@example.com"
            state.quickReplyDraftSaved = true

            state.deleteMessage(draftRow)
            awaitState { state.status == "Delete complete" }

            assertTrue(state.coreThreads.single().hasDraft)
        }

    @Test
    fun typingAndClearingDuringTheSendStillDiscardsTheConsumedDraft() =
        runBlocking {
            val core = SaveCore().apply { holdSend = CompletableDeferred() }
            core.releaseFirstSave.complete(Unit)
            val state = state(core, this)
            prepareQuickReply(state)
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true
            state.onQuickReplyBodyChange("First")

            state.sendQuickReply()
            withTimeout(5_000) { core.sendFinished.await() }
            // Started and thought better of: the generation moves on, but an
            // empty box is not a newer reply holding this draft.
            state.onQuickReplyBodyChange("Second")
            state.onQuickReplyBodyChange("")
            core.holdSend!!.complete(Unit)
            withTimeout(5_000) { while (state.quickReplySendInFlight) yield() }

            // Checked the moment the send settles: the blank bar's own debounced
            // discard is still 1.2s away, so a payload here is the send's doing
            // and nothing else's.
            assertTrue(core.discardPayloads.single().contains("reply-draft@example.com"))
            // And the bar must stop claiming the copy that was just deleted.
            awaitState { state.quickReplyDraftId.isEmpty() }
            assertEquals(false, state.quickReplyDraftSaved)
        }

    @Test
    fun aRetryOwnsTheConversationsDraftTheWayTheFirstAttemptDid() =
        runBlocking {
            val core =
                SaveCore().apply {
                    sendFails = true
                    holdSend = CompletableDeferred()
                }
            core.releaseFirstSave.complete(Unit)
            val state = state(core, this)
            prepareQuickReply(state)
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true
            state.onQuickReplyBodyChange("First")

            state.sendQuickReply()
            withTimeout(5_000) { core.sendFinished.await() }
            core.holdSend!!.complete(Unit)
            withTimeout(5_000) { while (state.quickReplySendInFlight) yield() }
            val pending = assertNotNull(state.pendingQuickReplySend)

            // The retry is out; its safety draft is no more disposable than the
            // first attempt's was.
            core.holdSend = CompletableDeferred()
            state.retryQuickReplySend(pending)
            assertEquals("thread", state.quickReplySendThreadId)
            state.quickReplyBody = ""
            state.discardQuickReplyDraftIfEmpty()
            assertEquals("reply-draft@example.com", state.quickReplyDraftId)

            core.holdSend!!.complete(Unit)
            withTimeout(5_000) { while (state.quickReplySendInFlight) yield() }
            assertTrue(core.discardPayloads.isEmpty())
        }

    @Test
    fun sendingWhileAnAutosaveIsMidWriteStillDiscardsTheDraftItWrote() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state)
            // Let the autosave get all the way into the core and block there.
            state.onQuickReplyBodyChange("Reply")
            core.firstSaveStarted.await()

            // The click cancels the autosave job with its write already in
            // flight. The write still lands, so its id has to reach the send —
            // as the bar's draft or as a handover — or nothing discards it.
            state.sendQuickReply()
            core.releaseFirstSave.complete(Unit)

            withTimeout(5_000) {
                core.sendFinished.await()
                core.discardFinished.await()
            }
            assertTrue(core.discardPayloads.single().contains("draft-1@example.com"))
        }

    @Test
    fun deletingTheLastDraftTakesTheThreadsDraftMarkerWithIt() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state)
            val draftRow = consumedDraft()
            state.messages = state.messages + draftRow
            state.coreThreads = listOfNotNull(state.selectedCoreThread)
            state.markThreadDraftEverywhere("thread")
            assertTrue(state.coreThreads.single().hasDraft)

            state.deleteMessage(draftRow)
            awaitState { state.status == "Delete complete" }

            // Returning to the list reloads nothing, so a marker left set here
            // keeps the badge on a row whose draft is gone.
            assertTrue(state.locallyDraftedThreadIds.isEmpty())
            assertEquals(false, state.coreThreads.single().hasDraft)
            assertEquals(false, state.selectedCoreThread?.hasDraft)
        }

    @Test
    fun emptyingTheReplyBarTakesTheThreadsDraftMarkerWithIt() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state)
            state.coreThreads = listOfNotNull(state.selectedCoreThread)
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true
            state.markThreadDraftEverywhere("thread")
            assertTrue(state.coreThreads.single().hasDraft)

            state.quickReplyBody = ""
            state.discardQuickReplyDraftIfEmpty()
            withTimeout(5_000) { core.discardFinished.await() }
            awaitState { state.locallyDraftedThreadIds.isEmpty() }

            // The list refresh that follows re-applies the local markers, so a
            // marker left behind here puts the badge straight back on the row.
            assertEquals(false, state.coreThreads.single().hasDraft)
            assertEquals(false, state.selectedCoreThread?.hasDraft)
        }

    @Test
    fun deletingOneOfTwoDraftsKeepsTheThreadsDraftMarker() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            prepareQuickReply(state)
            val draftRow = consumedDraft()
            val otherDraft = draftRow.copy(id = "draft-row-2", messageId = "second-draft@example.com")
            state.messages = state.messages + draftRow + otherDraft
            state.coreThreads = listOfNotNull(state.selectedCoreThread)
            state.markThreadDraftEverywhere("thread")

            state.deleteMessage(draftRow)
            awaitState { state.status == "Delete complete" }

            assertTrue(state.coreThreads.single().hasDraft)
        }

    @Test
    fun aFailedAllocationDoesNotLeaveAHandoverForTheNextSend() =
        runBlocking {
            val core = SaveCore().apply { allocationFails = true }
            val state = state(core, this)
            prepareQuickReply(state)
            state.onQuickReplyBodyChange("Reply")
            state.quickReplySendDraftHandover = "thread" to "handed-over@example.com"

            state.sendQuickReply()
            withTimeout(5_000) {
                while (state.quickReplySendInFlight) yield()
            }

            // Left behind, a later send in this thread would consume it and
            // delete the safety copy of a reply that never went out.
            assertEquals(null, state.quickReplySendDraftHandover)
            assertTrue(state.quickReplyConsumedDraftIds.isEmpty())
            assertTrue(core.discardPayloads.isEmpty())
        }

    @Test
    fun aFailedSendLeavesItsDraftReachableAgain() =
        runBlocking {
            val core = SaveCore().apply { sendFails = true }
            val state = state(core, this)
            prepareQuickReply(state)
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true
            state.onQuickReplyBodyChange("Reply")

            state.sendQuickReply()
            core.sendFinished.await()
            withTimeout(1_000) {
                while (state.quickReplySendInFlight) yield()
            }

            assertTrue(state.quickReplyConsumedDraftIds.isEmpty())
            assertTrue(core.discardPayloads.isEmpty())

            state.quickReplyThreadId = "other"
            state.quickReplyBody = ""
            state.quickReplyDraftId = ""
            state.quickReplyDraftSaved = false
            state.quickReplyThreadId = "thread"
            state.hydrateQuickReplyFromTailDraft("thread", state.messages + consumedDraft())

            assertEquals("Reply", state.quickReplyBody)
            assertEquals("reply-draft@example.com", state.quickReplyDraftId)
        }
}
