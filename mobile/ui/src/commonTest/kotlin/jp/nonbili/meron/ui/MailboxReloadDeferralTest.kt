// A mailbox refresh may overlap an existing load or a
// search. These tests control when requests finish and
// check which deferred reload should run afterward.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.CloseableHandle
import jp.nonbili.meron.shared.CoreEvent
import jp.nonbili.meron.shared.CoreEventStream
import jp.nonbili.meron.shared.MeronCore
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.MobileCommand
import jp.nonbili.meron.shared.ThreadSummary
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * Verify that a mailbox reload requested during a read sees subsequent changes.
 *
 * Stepping aside for the load already out for a mailbox is not the same as being answered by it: that load
 * read the store before the change the reload is reacting to.
 *
 * A quick reply's post-send draft discard used to run its reload while the reload the send itself started
 * was still out, and the rows that one then painted still counted the discarded draft — the card showed one
 * message too many until something else reloaded the list.
 */
class MailboxReloadDeferralTest {
    @Test
    fun reloadThatSteppedAsideRunsOnceTheLoadInFlightSettles() =
        runBlocking {
            val core = GatedCore()
            val state = state(core, this)

            state.syncCoreThreads(syncFirst = false)
            waitUntil { core.threadListCalls == 1 }
            // The post-discard reload, while the first one is still out.
            state.syncCoreThreads(syncFirst = false)
            delay(20)
            assertEquals(1, core.threadListCalls)

            core.gate.complete(Unit)
            waitUntil { core.threadListCalls == 2 }
            waitUntil { !state.syncing }
            assertEquals(2, core.threadListCalls)
        }

    // A search paints cache hits first, then runs the live IMAP search from the
    // cache stage's completion. Re-running the deferred reload ahead of that
    // started a cache read the live search stepped aside for, whose completion
    // re-ran it again — the two took turns for good and the search never ran.
    @Test
    fun deferredReloadDuringSearchYieldsToTheLiveSearchAndStops() =
        runBlocking {
            val core = GatedCore()
            val state = state(core, this)
            state.mailSearch = "deploy"

            state.syncCoreThreads(syncFirst = false)
            waitUntil { core.threadListCalls == 1 }
            state.syncCoreThreads(syncFirst = false)
            delay(20)
            assertEquals(1, core.threadListCalls)

            core.gate.complete(Unit)
            waitUntil { core.threadListCalls == 2 && !state.syncing }
            delay(50)
            assertEquals(2, core.threadListCalls)
            assertEquals(1, core.liveSearchCalls)
        }

    // A live search that stepped aside is re-run as one: as a cache read it
    // would only schedule the search again.
    @Test
    fun deferredLiveSearchIsReRunAsALiveSearch() =
        runBlocking {
            val core = GatedCore()
            val state = state(core, this)
            state.mailSearch = "deploy"

            state.syncCoreThreads(syncFirst = false, refreshSearch = true)
            waitUntil { core.threadListCalls == 1 }
            state.syncCoreThreads(syncFirst = false, refreshSearch = true)
            delay(20)
            assertEquals(1, core.threadListCalls)

            core.gate.complete(Unit)
            waitUntil { core.threadListCalls == 2 && !state.syncing }
            delay(50)
            assertEquals(2, core.threadListCalls)
            assertEquals(2, core.liveSearchCalls)
        }

    @Test
    fun aReloadWithNothingInItsWayRunsOnce() =
        runBlocking {
            val core = GatedCore().apply { gate.complete(Unit) }
            val state = state(core, this)

            state.syncCoreThreads(syncFirst = false)
            waitUntil { core.threadListCalls == 1 && !state.syncing }
            delay(20)
            assertEquals(1, core.threadListCalls)
        }

    // The loader's watchdog can't cancel the core call. Clearing `syncing` let a
    // refresh start a second load whose token turned the first, valid result
    // into a stale one.
    @Test
    fun watchdogLeavesAPendingLoadInChargeOfItsResult() =
        runBlocking {
            val core = GatedCore()
            val state = state(core, this)

            state.syncCoreThreads(syncFirst = false)
            waitUntil { core.threadListCalls == 1 }
            state.expireBlockingMailboxLoad()
            assertEquals(true, state.syncing)
            assertEquals(true, state.blockingMailboxLoadSlow)
            assertEquals(null, state.errorBanner)

            state.syncCoreThreads(syncFirst = false)
            delay(20)
            assertEquals(1, core.threadListCalls)

            core.gate.complete(Unit)
            waitUntil { core.threadListCalls == 2 && !state.syncing }
            assertEquals(mailboxCacheKey("a", "INBOX", "", FilterMode.All), state.visibleMailboxKey)
        }

    @Test
    fun watchdogUnblocksALoaderWithNoRequestBehindIt() {
        val state = state(GatedCore(), CoroutineScope(Dispatchers.Unconfined))
        state.initialThreadsLoaded = false
        state.syncing = true

        state.expireBlockingMailboxLoad()

        assertEquals(false, state.syncing)
        assertEquals(true, state.initialThreadsLoaded)
        assertEquals(false, state.blockingMailboxLoadSlow)
        assertEquals(true, state.errorBanner != null)
    }

    @Test
    fun unstarRemovesOnlyStarredViewRowsAndRollsBackOnFailure() =
        runBlocking {
            val core = GatedCore()
            val state = state(core, this)
            val row = ThreadSummary(id = "a#INBOX#root", accountId = "a", folder = "INBOX", subject = "Release", sender = "Sender", starred = true)
            state.selectedCoreAccountId = UNIFIED_ACCOUNT_ID
            state.selectedCoreFolder = STARRED_FOLDER
            state.coreThreads = listOf(row)
            val starredKey = "$UNIFIED_ACCOUNT_ID\n$STARRED_FOLDER"
            val inboxKey = "a\nINBOX"
            state.kanbanColumns = mapOf(starredKey to KanbanColumnState(threads = listOf(row)), inboxKey to KanbanColumnState(threads = listOf(row)))
            core.starFailure = true

            state.toggleStar(row)

            assertEquals(emptyList(), state.coreThreads)
            assertEquals(emptyList(), state.kanbanColumns[starredKey]?.threads)
            assertEquals(listOf(row.copy(starred = false)), state.kanbanColumns[inboxKey]?.threads)
            waitUntil { state.status.startsWith("Couldn't remove star") }
            assertEquals(listOf(row), state.coreThreads)
            assertEquals(listOf(row), state.kanbanColumns[starredKey]?.threads)
            assertEquals(listOf(row), state.kanbanColumns[inboxKey]?.threads)
        }

    @Test
    fun unstarReloadsUnifiedStarredAfterCommit() =
        runBlocking {
            val core = GatedCore()
            val state = state(core, this)
            val row = ThreadSummary(id = "a#INBOX#root", accountId = "a", folder = "INBOX", subject = "Release", sender = "Sender", starred = true)
            state.selectedCoreAccountId = UNIFIED_ACCOUNT_ID
            state.selectedCoreFolder = STARRED_FOLDER
            state.coreThreads = listOf(row)

            state.toggleStar(row)
            assertEquals(emptyList(), state.coreThreads)
            waitUntil { core.starredListCalls == 1 && !state.syncing }
            assertEquals(emptyList(), state.coreThreads)
        }

    @Test
    fun unstarLastMessageRemovesRowAndRestoresItOnFailure() =
        runBlocking {
            val core = GatedCore().apply { starFailure = true }
            val state = state(core, this)
            val row = ThreadSummary(id = "a#INBOX#root", accountId = "a", folder = "INBOX", subject = "Release", sender = "Sender", starred = true)
            val message = MessageBody(id = "a#INBOX#1", folderId = "INBOX", from = "Sender", to = "Me", subject = "Release", body = "", starred = true)
            state.selectedCoreAccountId = UNIFIED_ACCOUNT_ID
            state.selectedCoreFolder = STARRED_FOLDER
            state.coreThreads = listOf(row)
            state.selectedCoreThread = row
            state.messages = listOf(message)

            state.toggleMessageStarred(message)

            assertEquals(emptyList(), state.coreThreads)
            assertEquals(false, state.messages.single().starred)
            assertEquals(false, state.selectedCoreThread?.starred)
            waitUntil { state.status.startsWith("Couldn't remove star") }
            assertEquals(listOf(row), state.coreThreads)
            assertEquals(listOf(message), state.messages)
            assertEquals(row, state.selectedCoreThread)
        }

    @Test
    fun starInInboxKeepsOtherMailboxCachesAndDoesNotReload() =
        runBlocking {
            val core = GatedCore().apply { gate.complete(Unit) }
            val state = state(core, this)
            val row = ThreadSummary(id = "a#INBOX#root", accountId = "a", folder = "INBOX", subject = "Release", sender = "Sender", starred = true)
            val message = MessageBody(id = "a#INBOX#1", folderId = "INBOX", from = "Sender", to = "Me", subject = "Release", body = "", starred = true)
            val cached = mapOf(mailboxCacheKey("a", "Archive", "", FilterMode.All) to MailboxLoadResult(folders = emptyList(), folder = "Archive", threads = listOf(row)))
            state.mailboxCache = cached
            state.coreThreads = listOf(row)
            state.selectedCoreThread = row
            state.messages = listOf(message)

            state.toggleStar(row)
            waitUntil { state.status == "Star removed" }
            assertEquals(cached, state.mailboxCache)
            assertEquals(0, core.threadListCalls)
            assertEquals(0, core.starredListCalls)

            state.toggleMessageStarred(message)
            waitUntil { state.status == "Star removed" }
            assertEquals(cached, state.mailboxCache)
            assertEquals(0, core.threadListCalls)
            assertEquals(0, core.starredListCalls)
        }

    private suspend fun waitUntil(condition: () -> Boolean) {
        withTimeout(5_000) {
            while (!condition()) delay(5)
        }
    }

    private fun state(
        core: MeronCore,
        scope: CoroutineScope,
    ): MeronMobileState =
        MeronMobileState(
            scope = scope,
            core = core,
            coreLoaded = true,
            prefs = MemoryPreferences(),
            kanbanPrefs = MemoryPreferences(),
            services = NoopPlatformServices(),
            locale = NoopLocaleController(),
            mobileHost = DefaultMobileHost(),
            settingsMirror = SettingsMirror(core, MemoryPreferences()) { true },
        ).apply {
            coreAccounts =
                listOf(
                    AccountSummary(
                        id = "a",
                        email = "a@example.com",
                        imapHost = "127.0.0.1",
                        imapPort = 1143,
                        smtpHost = "127.0.0.1",
                        smtpPort = 1025,
                        tls = false,
                        starttls = true,
                        smtpTls = false,
                        smtpStarttls = true,
                    ),
                )
            selectedCoreAccountId = "a"
            selectedCoreFolder = "INBOX"
            initialThreadsLoaded = true
        }

    /** Holds the first thread list read open until [gate] completes. */
    private class GatedCore : MeronCore {
        val gate = CompletableDeferred<Unit>()
        var threadListCalls = 0
        var liveSearchCalls = 0
        var starredListCalls = 0
        var starFailure = false

        override suspend fun invoke(
            command: String,
            payloadJson: String,
        ): String =
            when (command) {
                MobileCommand.MarkStarred -> {
                    if (starFailure) error("Star rejected")
                    "{\"ok\":true}"
                }

                MobileCommand.StarredItems -> {
                    starredListCalls += 1
                    "{\"items\":[]}"
                }

                MobileCommand.FolderList -> {
                    """{"folders":[{"account_id":"a","name":"INBOX","role":"inbox"}]}"""
                }

                MobileCommand.ThreadList -> {
                    threadListCalls += 1
                    if (payloadJson.contains("\"refresh\":true")) liveSearchCalls += 1
                    gate.await()
                    """{"threads":[]}"""
                }

                else -> {
                    "{}"
                }
            }

        override fun events(): CoreEventStream =
            object : CoreEventStream {
                override fun subscribe(listener: (CoreEvent) -> Unit): CloseableHandle = CloseableHandle {}
            }

        override suspend fun protocolVersion(): Int = 0
    }

    private class MemoryPreferences : AppPreferences {
        private val values = mutableMapOf<String, String>()

        override fun getString(
            key: String,
            default: String,
        ): String = values[key] ?: default

        override fun putString(
            key: String,
            value: String,
        ) {
            values[key] = value
        }

        override fun getBoolean(
            key: String,
            default: Boolean,
        ): Boolean = default

        override fun putBoolean(
            key: String,
            value: Boolean,
        ) {}

        override fun getInt(
            key: String,
            default: Int,
        ): Int = default

        override fun putInt(
            key: String,
            value: Int,
        ) {}

        override fun getStringSet(
            key: String,
            default: Set<String>,
        ): Set<String> = default

        override fun putStringSet(
            key: String,
            value: Set<String>,
        ) {}

        override fun remove(key: String) {
            values.remove(key)
        }
    }

    private class NoopPlatformServices : PlatformServices {
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

    private class NoopLocaleController : LocaleController {
        override fun systemLanguageTag(): String = ""

        override fun applySystem(tag: String) {}

        override fun deviceLanguageTag(): String = "en-US"

        override fun displayName(tag: String): String = tag
    }
}
