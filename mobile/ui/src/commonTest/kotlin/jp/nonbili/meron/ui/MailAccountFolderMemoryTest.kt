// The mailbox remembers a folder for each account. Tests
// recreate the state across account switches and check
// when an explicit selection supersedes the remembered
// folder.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.CloseableHandle
import jp.nonbili.meron.shared.CoreEvent
import jp.nonbili.meron.shared.CoreEventStream
import jp.nonbili.meron.shared.MeronCore
import jp.nonbili.meron.shared.MobileCommand
import jp.nonbili.meron.shared.parseAccountListResponse
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class MailAccountFolderMemoryTest {
    @Test
    fun switchingAccountsRestoresTheirFoldersAcrossStateRecreation() {
        val prefs = MemoryAppPreferences()
        val state = testState(prefs)

        state.selectCoreMailbox("acct-1", "Projects/Work")
        state.selectCoreMailbox("acct-2", "Archive")
        state.selectCoreMailbox("acct-1")
        assertEquals("Projects/Work", state.selectedCoreFolder)
        state.selectCoreMailbox("acct-2")
        assertEquals("Archive", state.selectedCoreFolder)

        val restarted = testState(prefs)
        assertEquals("acct-2", restarted.selectedCoreAccountId)
        assertEquals("Archive", restarted.selectedCoreFolder)
        restarted.selectCoreMailbox("acct-1")
        assertEquals("Projects/Work", restarted.selectedCoreFolder)
        restarted.selectCoreMailbox("new-account")
        assertEquals(INBOX_FOLDER, restarted.selectedCoreFolder)
    }

    @Test
    fun explicitFolderSelectionOverridesRememberedFolder() {
        val state = testState(MemoryAppPreferences())
        state.selectCoreMailbox("acct-1", "Archive")
        state.selectCoreMailbox("acct-2")
        state.selectCoreMailbox("acct-1", INBOX_FOLDER)
        assertEquals(INBOX_FOLDER, state.selectedCoreFolder)
        state.selectCoreMailbox("acct-2")
        state.selectCoreMailbox("acct-1")
        assertEquals(INBOX_FOLDER, state.selectedCoreFolder)
    }

    @Test
    fun accountRemovedDuringSyncDoesNotOverwriteUnifiedFolder() =
        runBlocking {
            assertMismatchedSyncDoesNotPersist(changeAccountDuringSync = true)
        }

    @Test
    fun accountSpecificReloadWhileViewingUnifiedDoesNotOverwriteUnifiedFolder() =
        runBlocking {
            assertMismatchedSyncDoesNotPersist(changeAccountDuringSync = false)
        }

    @Test
    fun removingAccountsPreservesTheSelectedMailboxAndReloadsIt() =
        runBlocking {
            for (selectedAccount in listOf(UNIFIED_ACCOUNT_ID, "acct-1", "acct-2")) {
                val prefs = MemoryAppPreferences()
                val core = RemovalCore()
                val state = testState(prefs, core, this)
                state.coreAccounts = parseAccountListResponse(TWO_ACCOUNTS)
                val removed = state.coreAccounts.first()
                state.selectCoreMailbox(UNIFIED_ACCOUNT_ID, "starred")
                if (selectedAccount != UNIFIED_ACCOUNT_ID) state.selectCoreMailbox(selectedAccount, "Archive")

                state.removeAccount(removed)
                withTimeout(5_000) {
                    while (state.coreAccounts.any { it.id == removed.id } || state.syncing) delay(5)
                }

                val expectedAccount = if (selectedAccount == "acct-2") "acct-2" else UNIFIED_ACCOUNT_ID
                val expectedFolder = if (expectedAccount == "acct-2") "Archive" else "starred"
                assertEquals(expectedAccount, state.selectedCoreAccountId)
                assertEquals(expectedFolder, state.selectedCoreFolder)
                assertEquals("starred", loadMailFolderForAccount(prefs, UNIFIED_ACCOUNT_ID))
                assertEquals(INBOX_FOLDER, loadMailFolderForAccount(prefs, removed.id))
                assertEquals(expectedFolder, testState(prefs).selectedCoreFolder)
                assertTrue(state.initialThreadsLoaded)
                assertEquals(expectedAccount, state.visibleMailboxKey?.accountId)
                if (expectedAccount == "acct-2") {
                    assertEquals("acct-2", state.coreThreads.single().accountId)
                } else {
                    assertEquals(1, core.starredLoads)
                }
            }
        }

    @Test
    fun olderRequestCachesItsResultWithoutResettingNewerLoadFlags() =
        runBlocking {
            val prefs = MemoryAppPreferences()
            val core = GatedCore()
            val state = testState(prefs, core, this)
            state.coreAccounts = parseAccountListResponse(TWO_ACCOUNTS)
            state.selectCoreMailbox("acct-1", "Projects/Work")
            state.syncCoreThreads(syncFirst = false)
            withTimeout(5_000) { core.started.await() }
            val oldLoad = coroutineContext[Job]!!.children.single()

            state.selectCoreMailbox(UNIFIED_ACCOUNT_ID, "starred")
            state.syncCoreThreads(syncFirst = false)
            withTimeout(5_000) { core.starredStarted.await() }
            val newKey = state.activeMailboxLoadKey
            val newStartedAt = state.activeMailboxLoadStartedAtMillis
            core.release.complete(Unit)
            withTimeout(5_000) { oldLoad.join() }

            assertTrue(state.syncing)
            assertEquals(newKey, state.activeMailboxLoadKey)
            assertEquals(newStartedAt, state.activeMailboxLoadStartedAtMillis)
            assertEquals("starred", state.selectedCoreFolder)
            core.starredRelease.complete(Unit)
            withTimeout(5_000) {
                while (state.syncing) delay(5)
            }
            state.selectCoreMailbox("acct-1")
            assertTrue(state.initialThreadsLoaded)
            assertEquals("Projects/Work", state.selectedCoreFolder)
            assertEquals("acct-1", state.coreThreads.single().accountId)
        }

    private suspend fun CoroutineScope.assertMismatchedSyncDoesNotPersist(changeAccountDuringSync: Boolean) {
        val prefs = MemoryAppPreferences()
        val core = GatedCore()
        val state = testState(prefs, core, this)
        state.coreAccounts = parseAccountListResponse("""{"accounts":[{"id":"acct-1","email":"one@example.com"}]}""")
        state.selectCoreMailbox(UNIFIED_ACCOUNT_ID, "starred")
        if (changeAccountDuringSync) state.selectCoreMailbox("acct-1", "Projects/Work")

        state.syncCoreThreads(accountOverride = "acct-1", folderOverride = "Projects/Work", syncFirst = false)
        withTimeout(5_000) { core.started.await() }
        if (changeAccountDuringSync) state.applyAccounts("""{"accounts":[]}""")
        val foldersBefore = state.coreFolders
        val threadsBefore = state.coreThreads
        val mailboxBefore = state.visibleMailboxKey
        val cursorBefore = state.mailboxCursor
        state.blockingMailboxLoadWarned = true
        state.blockingMailboxLoadSlow = true
        core.release.complete(Unit)
        withTimeout(5_000) {
            while (state.syncing) delay(5)
        }

        assertEquals("starred", state.selectedCoreFolder)
        assertEquals(foldersBefore, state.coreFolders)
        assertEquals(threadsBefore, state.coreThreads)
        assertEquals(mailboxBefore, state.visibleMailboxKey)
        assertEquals(cursorBefore, state.mailboxCursor)
        assertNull(state.activeMailboxLoadKey)
        assertEquals(0L, state.activeMailboxLoadStartedAtMillis)
        assertFalse(state.blockingMailboxLoadWarned)
        assertFalse(state.blockingMailboxLoadSlow)
        assertTrue(state.initialThreadsLoaded)
        assertNull(state.deferredMailboxReload)

        // A later account refresh must not persist the rejected request's folder.
        state.applyAccounts("""{"accounts":[]}""")
        assertEquals("starred", state.selectedCoreFolder)
        assertEquals(UNIFIED_ACCOUNT_ID, loadLastMailAccountId(prefs))
        assertEquals("starred", loadMailFolderForAccount(prefs, UNIFIED_ACCOUNT_ID))
        assertEquals("starred", testState(prefs).selectedCoreFolder)
    }

    private fun testState(
        prefs: AppPreferences,
        core: MeronCore = FakeCore(),
        scope: CoroutineScope = CoroutineScope(EmptyCoroutineContext),
    ) = MeronMobileState(
        scope = scope,
        core = core,
        coreLoaded = true,
        prefs = prefs,
        kanbanPrefs = MemoryAppPreferences(),
        services = FakePlatformServices(),
        locale = FakeLocaleController(),
        mobileHost = DefaultMobileHost(),
        settingsMirror = SettingsMirror(core, prefs) { true },
    )

    private class FakePlatformServices : PlatformServices {
        override fun openUrl(url: String) {}

        override fun openOAuthUrl(
            url: String,
            callbackScheme: String,
            onCallback: (String) -> Unit,
            onFailure: (String) -> Unit,
        ) {}

        override fun copyText(
            label: String,
            value: String,
        ) {}

        override fun copyImage(
            bytes: ByteArray,
            mimeType: String,
            label: String,
        ) {}

        override fun shareFile(
            bytes: ByteArray,
            fileName: String,
            mimeType: String,
        ) {}

        override fun saveFile(
            bytes: ByteArray,
            fileName: String,
            mimeType: String,
        ) {}

        override fun pickFile(
            mimeTypes: List<String>,
            onPicked: (PickedFile?) -> Unit,
        ) {}

        override fun pickImage(onPicked: (PickedFile?) -> Unit) {}
    }

    private class FakeLocaleController : LocaleController {
        override fun systemLanguageTag(): String = ""

        override fun applySystem(tag: String) {}

        override fun deviceLanguageTag(): String = "en-US"

        override fun displayName(tag: String): String = tag
    }

    private class GatedCore : FakeCore() {
        val started = CompletableDeferred<Unit>()
        val release = CompletableDeferred<Unit>()
        val starredStarted = CompletableDeferred<Unit>()
        val starredRelease = CompletableDeferred<Unit>()

        override suspend fun invoke(
            command: String,
            payloadJson: String,
        ): String =
            when (command) {
                MobileCommand.FolderList -> {
                    """{"folders":[{"account_id":"acct-1","name":"Projects/Work"}]}"""
                }

                MobileCommand.ThreadList -> {
                    started.complete(Unit)
                    release.await()
                    """{"threads":[{"id":"acct-1:Projects/Work:t1","account_id":"acct-1","folder_id":"Projects/Work","subject":"Other account","message_count":1,"date":1}]}"""
                }

                MobileCommand.StarredItems -> {
                    starredStarted.complete(Unit)
                    starredRelease.await()
                    """{"items":[]}"""
                }

                else -> {
                    super.invoke(command, payloadJson)
                }
            }
    }

    private class RemovalCore : FakeCore() {
        var starredLoads = 0

        override suspend fun invoke(
            command: String,
            payloadJson: String,
        ): String =
            when (command) {
                MobileCommand.AccountList -> {
                    """{"accounts":[{"id":"acct-2","email":"two@example.com"}]}"""
                }

                MobileCommand.FolderList -> {
                    """{"folders":[{"account_id":"acct-2","name":"Archive"}]}"""
                }

                MobileCommand.ThreadList -> {
                    """{"threads":[{"id":"acct-2:Archive:t1","account_id":"acct-2","folder_id":"Archive","subject":"Remaining account","message_count":1,"date":1}]}"""
                }

                MobileCommand.StarredItems -> {
                    starredLoads += 1
                    """{"items":[]}"""
                }

                else -> {
                    super.invoke(command, payloadJson)
                }
            }
    }

    private companion object {
        const val TWO_ACCOUNTS = """{"accounts":[{"id":"acct-1","email":"one@example.com"},{"id":"acct-2","email":"two@example.com"}]}"""
    }

    private open class FakeCore : MeronCore {
        override suspend fun invoke(
            command: String,
            payloadJson: String,
        ): String = "{}"

        override fun events(): CoreEventStream =
            object : CoreEventStream {
                override fun subscribe(listener: (CoreEvent) -> Unit): CloseableHandle = CloseableHandle {}
            }

        override suspend fun protocolVersion(): Int = 0
    }
}
