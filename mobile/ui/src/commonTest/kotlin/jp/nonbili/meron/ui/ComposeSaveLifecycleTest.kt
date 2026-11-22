// Several saves can overlap while editing or transferring
// ownership of a mobile draft. The fixtures control completion
// order and verify how the current remote draft is retained or
// discarded.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.ProxySpec
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.yield
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

internal class ComposeSaveLifecycleTest : ComposeLifecycleFixture() {
    @Test
    fun fullComposeSavesAreSerialized() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            state.openCompose()
            state.to = "you@example.com"
            state.subject = "First"
            state.body = "First body${state.body}"

            state.autoSaveComposeDraft()
            core.firstSaveStarted.await()
            state.subject = "Second"
            state.autoSaveComposeDraft()
            yield()

            assertEquals(1, core.saveCalls)
            core.releaseFirstSave.complete(Unit)
            core.secondSaveFinished.await()

            assertEquals(2, core.saveCalls)
            assertTrue(core.savedPayloads[0].contains("\"subject\":\"First\""))
            assertTrue(core.savedPayloads[1].contains("\"subject\":\"Second\""))
            assertEquals("draft-1@example.com", state.composeDraftId)
        }

    @Test
    fun obsoleteSaveDiscardsItsAllocatedRemoteDraft() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            state.openCompose()
            state.to = "old@example.com"
            state.subject = "Old"
            state.body = "Old body${state.body}"

            state.autoSaveComposeDraft()
            core.firstSaveStarted.await()
            state.openCompose()
            core.releaseFirstSave.complete(Unit)
            core.discardFinished.await()

            assertTrue(core.discardPayloads.single().contains("draft-1@example.com"))
            assertEquals("", state.composeDraftId)
            assertEquals("", state.to)
        }

    @Test
    fun closingSaveSurvivesOpeningAnotherComposer() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            state.openCompose()
            state.to = "saved@example.com"
            state.subject = "Keep me"
            state.body = "Closing body${state.body}"

            state.closeCompose()
            core.firstSaveStarted.await()
            state.openCompose()
            core.releaseFirstSave.complete(Unit)
            core.firstSaveFinished.await()
            yield()

            assertEquals(1, core.saveCalls)
            assertTrue(core.savedPayloads.single().contains("\"subject\":\"Keep me\""))
            assertTrue(core.discardPayloads.isEmpty())
            assertEquals("", state.to)
        }

    @Test
    fun saveResultCannotCrossAnAccountSwitch() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            state.openCompose()
            state.to = "old@example.com"
            state.subject = "Old account"
            state.body = "Old body${state.body}"

            state.autoSaveComposeDraft()
            core.firstSaveStarted.await()
            state.changeComposeIdentity("b", "b@example.com")
            core.releaseFirstSave.complete(Unit)
            core.discardFinished.await()

            assertEquals("b", state.composeFromAccountId)
            assertEquals("", state.composeDraftId)
            assertEquals("", state.composeDraftAccountId)
            assertTrue(core.discardPayloads.single().contains("draft-1@example.com"))
        }

    @Test
    fun existingIdAutosaveCannotResurrectAfterSend() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            state.openCompose()
            state.to = "you@example.com"
            state.subject = "Send"
            state.body = "Send body${state.body}"
            state.composeDraftId = "existing-a@example.com"
            state.composeDraftSaved = true
            state.composeDraftAccountId = "a"

            state.autoSaveComposeDraft()
            core.firstSaveStarted.await()
            state.sendMail()
            yield()
            assertEquals(0, core.sendCalls)
            core.releaseFirstSave.complete(Unit)
            core.sendFinished.await()
            core.discardFinished.await()
            awaitState { !state.composeSendInFlight && state.composeDraftId.isEmpty() }

            assertEquals("", state.composeDraftId)
            assertEquals(false, state.composeDraftSaved)
            assertEquals(1, core.discardPayloads.size)
            assertTrue(core.discardPayloads.single().contains("existing-a@example.com"))
        }

    @Test
    fun failedPostSendDraftDiscardIsRetried() =
        runBlocking {
            val core = SaveCore().apply { discardFailuresLeft = 1 }
            val state = state(core, this)
            state.sentDraftDiscardRetryDelaysMs = listOf(1L)
            state.openCompose()
            state.to = "you@example.com"
            state.subject = "Send"
            state.body = "Send body${state.body}"
            state.composeDraftId = "existing-a@example.com"
            state.composeDraftSaved = true
            state.composeDraftAccountId = "a"

            state.sendMail()
            core.sendFinished.await()
            core.discardFinished.await()
            withTimeout(1_000) {
                while (state.composeSendInFlight || core.discardPayloads.size < 2) yield()
            }
            awaitState { state.quickReplyConsumedDraftIds.isEmpty() }

            // Retried on its own rather than parked until some later draft save.
            assertTrue(core.discardPayloads.all { it.contains("existing-a@example.com") })
            assertTrue(state.composeDraftCleanupOwners.isEmpty())
            assertEquals("", state.composeDraftId)
        }

    @Test
    fun existingIdAutosaveCannotResurrectAfterDiscard() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            state.openCompose()
            state.to = "you@example.com"
            state.subject = "Discard"
            state.body = "Discard body${state.body}"
            state.composeDraftId = "existing-a@example.com"
            state.composeDraftSaved = true
            state.composeDraftAccountId = "a"

            state.autoSaveComposeDraft()
            core.firstSaveStarted.await()
            state.discardComposeDraft()
            yield()
            assertEquals(0, core.discardPayloads.size)
            core.releaseFirstSave.complete(Unit)
            core.discardFinished.await()
            awaitState { state.composeDraftId.isEmpty() && !state.composeDraftSaved }

            assertEquals("", state.composeDraftId)
            assertEquals(false, state.composeDraftSaved)
            assertEquals(1, core.discardPayloads.size)
            assertTrue(core.discardPayloads.single().contains("existing-a@example.com"))
        }

    @Test
    fun savedDraftSwitchingAccountsAllocatesBIdAndDiscardsExactAId() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            state.openCompose()
            state.to = "you@example.com"
            state.subject = "Move"
            state.body = "Move body${state.body}"

            state.autoSaveComposeDraft()
            core.firstSaveStarted.await()
            core.releaseFirstSave.complete(Unit)
            core.firstSaveFinished.await()
            withTimeout(1_000) {
                while (state.composeDraftId.isBlank()) yield()
            }
            assertEquals("draft-1@example.com", state.composeDraftId)

            state.changeComposeIdentity("b", "b@example.com")
            assertTrue(state.composeDraftId.startsWith("local-draft-"))
            assertEquals(false, state.composeDraftSaved)
            assertEquals("", state.composeDraftAccountId)
            state.autoSaveComposeDraft()
            core.secondSaveFinished.await()
            core.discardFinished.await()
            awaitState { state.composeDraftId == "draft-2@example.com" && state.composeDraftAccountId == "b" }

            assertEquals("draft-2@example.com", state.composeDraftId)
            assertEquals("b", state.composeDraftAccountId)
            assertTrue(core.savedPayloads[1].contains("\"account_id\":\"b\""))
            assertTrue(core.savedPayloads[1].contains("\"draft_id\":\"draft-2@example.com\""))
            assertEquals(1, core.discardPayloads.size)
            assertTrue(core.discardPayloads.single().contains("\"account_id\":\"a\""))
            assertTrue(core.discardPayloads.single().contains("draft-1@example.com"))
        }

    @Test
    fun signaturePendingBlocksSave() =
        runBlocking {
            val core = SaveCore()
            val state = state(core, this)
            state.openCompose()
            state.to = "you@example.com"
            state.subject = "Pending"
            state.body = "Body${state.body}"
            state.composeSignaturePending = true

            state.saveComposeDraft()
            yield()

            assertEquals(0, core.saveCalls)
            assertEquals("Waiting for signature before saving.", state.status)
        }

    @Test
    fun olderAccountAndProxyLoadsCannotApplyAfterNewerLoads() =
        runBlocking {
            val core = ReloadCore()
            val state = state(core, this)

            val oldAccounts = state.listAccounts()
            val oldProxy = state.loadAppProxy()
            core.oldAccountsStarted.await()
            core.oldProxyStarted.await()
            val newAccounts = state.listAccounts()
            val newProxy = state.loadAppProxy()
            core.newAccountsStarted.await()
            core.newProxyStarted.await()

            core.newAccounts.complete("""{"accounts":[{"id":"new","email":"new@example.com"}]}""")
            core.newProxy.complete("""{"proxy":{"mode":"http","host":"new.proxy","port":8080}}""")
            newAccounts?.join()
            newProxy?.join()
            core.oldAccounts.complete("""{"accounts":[{"id":"old","email":"old@example.com"}]}""")
            core.oldProxy.complete("""{"proxy":{"mode":"http","host":"old.proxy","port":8080}}""")
            oldAccounts?.join()
            oldProxy?.join()

            assertEquals("new", state.coreAccounts.single().id)
            assertEquals(ProxySpec("http", "new.proxy", 8080), state.appProxy)
        }
}
