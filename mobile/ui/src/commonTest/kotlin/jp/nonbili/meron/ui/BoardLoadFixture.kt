// Board load fixture.
// The mobile host suites exercise these cases with deterministic core responses.
// The tests run without a device or a live mail account.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.CloseableHandle
import jp.nonbili.meron.shared.CoreEvent
import jp.nonbili.meron.shared.CoreEventStream
import jp.nonbili.meron.shared.MeronCore
import jp.nonbili.meron.shared.MobileCommand
import jp.nonbili.meron.shared.ThreadSummary
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeout
import kotlin.concurrent.Volatile
import kotlin.test.assertEquals
import kotlin.test.assertFalse

/**
 * Two reloads of one Kanban column can be out at once — the Sent copy event's
 * and a quick reply's post-discard one — and the earlier read the store before
 * the discard. Landing last, it used to put the card back with the draft
 * counted and badged; only the newest request may write.
 */

internal abstract class BoardLoadFixture {
    protected fun checkCachedRemoval(success: Boolean) =
        runBlocking {
            val state = state(GatedCore(), this)
            val thread = ThreadSummary(id = "a:INBOX:t1", accountId = "a", folder = "INBOX", subject = "Hello", sender = "Ada")
            val other = thread.copy(id = "a:INBOX:t2", dateEpochSeconds = 2)
            val keys = listOf(mailboxCacheKey("a", "INBOX", "", FilterMode.All), mailboxCacheKey(UNIFIED_ACCOUNT_ID, "INBOX", "", FilterMode.All))
            val cached = MailboxLoadResult(emptyList(), "INBOX", listOf(other, thread), nextCursor = "older", pageDepth = 100)
            state.mailboxCache = keys.associateWith { cached }
            val first = state.suppressRemovedThread(thread.id)
            val second = state.suppressRemovedThread(thread.id)
            // Another read caches only the visible rows while both writes are out.
            state.mailboxCache = keys.associateWith { cached.copy(threads = listOf(other)) }
            if (success) first.complete() else first.rollback()
            second.rollback()
            keys.forEach { key ->
                assertEquals(if (success) listOf(other) else listOf(other, thread), state.mailboxCache[key]?.threads)
                assertEquals("older", state.mailboxCache[key]?.nextCursor)
                assertEquals(100, state.mailboxCache[key]?.pageDepth)
            }
        }

    protected fun removalSurvivesLateRead(
        kanban: Boolean,
        loadMore: Boolean,
        actionFails: Boolean = false,
        notification: Boolean = false,
        permanentDelete: Boolean = false,
    ) = runBlocking {
        val childScope = CoroutineScope(coroutineContext + Job(coroutineContext[Job]))
        try {
            val folder = if (permanentDelete) "Trash" else "INBOX"
            val core = GatedCore(gateActions = true, actionFails = actionFails, permanentDelete = permanentDelete, listedFolder = folder)
            val state = state(core, childScope)
            val thread = ThreadSummary(id = "a:$folder:t1", accountId = "a", folder = folder, subject = "Hello", sender = "Ada")
            val column = KanbanColumnSpec(accountId = "a", folderId = folder)
            val key = kanbanColumnKey(column)
            val cacheKey = mailboxCacheKey("a", folder, "", FilterMode.All)
            state.selectedCoreFolder = folder
            state.coreThreads = listOf(thread)
            state.visibleMailboxKey = cacheKey
            state.mailboxCursor = "older"
            state.mailboxCache = mapOf(cacheKey to MailboxLoadResult(emptyList(), folder, listOf(thread)))
            state.kanbanColumns = mapOf(key to KanbanColumnState(threads = listOf(thread), nextCursor = "older"))
            if (notification) {
                state.openNotificationThread(NotificationThreadTarget(accountId = "a", folder = folder))
            } else if (kanban) {
                if (loadMore) state.loadMoreKanbanColumn(column) else state.loadKanbanColumn(column)
            } else {
                if (loadMore) state.loadMoreCoreThreads() else state.syncCoreThreads(syncFirst = false)
            }
            waitUntil { core.threadListCalls == 1 }
            when {
                kanban && loadMore -> state.moveThreadToFolder(thread, "Archive")
                loadMore -> state.deleteThread(thread)
                else -> state.archiveOrRemove(thread)
            }
            waitUntil { core.actionCalls == 1 }
            assertEquals(emptyList(), state.coreThreads)
            assertEquals(emptyList(), state.threadRemovalGuard.filter(state.mailboxCache[cacheKey]!!.threads))
            core.actionGate.complete(Unit)
            waitUntil { state.threadRemovalGuard.filter(listOf(thread)).isNotEmpty() }
            // The backend write is done, but the old read still holds its row.
            core.gate.complete(Unit)
            waitUntil { !state.syncing && !state.loadingMoreThreads && state.kanbanColumns[key]?.loading != true && state.kanbanColumns[key]?.loadingMore != true }
            if (actionFails) {
                assertEquals(thread.id, state.coreThreads.single().id)
                assertEquals(
                    thread.id,
                    state.kanbanColumns[key]
                        ?.threads
                        ?.single()
                        ?.id,
                )
            } else {
                assertEquals(emptyList(), state.coreThreads)
                assertEquals(emptyList(), state.kanbanColumns[key]?.threads)
                assertFalse(state.mailboxCache.values.any { cached -> cached.threads.any { it.id == thread.id } })
            }
            // A subsequent valid read can display a moved-back copy or new reply.
            if (kanban) state.loadKanbanColumn(column) else state.syncCoreThreads(syncFirst = false)
            waitUntil { core.threadListCalls >= 2 && !state.syncing && state.kanbanColumns[key]?.loading != true }
            assertEquals(
                thread.id,
                if (kanban) {
                    state.kanbanColumns[key]
                        ?.threads
                        ?.single()
                        ?.id
                } else {
                    state.coreThreads.single().id
                },
            )
        } finally {
            childScope.cancel()
        }
    }

    protected suspend fun waitUntil(condition: () -> Boolean) {
        withTimeout(5_000) {
            while (!condition()) delay(5)
        }
    }

    protected fun state(
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

    /** Holds the first thread list read open until [gate] completes; every
     *  later read answers at once with one message fewer on the card. */

    protected class GatedCore(
        private val gateActions: Boolean = false,
        private val actionFails: Boolean = false,
        private val permanentDelete: Boolean = false,
        private val emptyAfterActions: Boolean = false,
        private val listedFolder: String = "INBOX",
    ) : MeronCore {
        var zeroMove = false
        var folderListCalls = 0
        val actionGate = CompletableDeferred<Unit>()
        val actionGates = mutableMapOf<String, CompletableDeferred<Unit>>()

        // Commands arrive on the IO dispatcher, two at once when actions
        // overlap, while the test polls the counts from its own thread.
        private val bookkeeping = Mutex()
        val actionRequests = mutableListOf<Pair<String, String>>()

        @Volatile
        var actionCalls = 0

        @Volatile
        private var actionsCompleted = false
        val gate = CompletableDeferred<Unit>()

        @Volatile
        var threadListCalls = 0

        override suspend fun invoke(
            command: String,
            payloadJson: String,
        ): String =
            when (command) {
                MobileCommand.FolderList -> {
                    folderListCalls++
                    """{"folders":[{"account_id":"a","name":"INBOX","role":"inbox"}]}"""
                }

                MobileCommand.ThreadList -> {
                    val call =
                        bookkeeping.withLock {
                            threadListCalls += 1
                            threadListCalls
                        }
                    if (emptyAfterActions && actionsCompleted) {
                        """{"threads":[]}"""
                    } else if (call == 1) {
                        gate.await()
                        card(messageCount = 3, hasDraft = true)
                    } else {
                        card(messageCount = 2, hasDraft = false)
                    }
                }

                MobileCommand.Archive, MobileCommand.Delete, MobileCommand.Move, MobileCommand.EmptyFolder -> {
                    bookkeeping.withLock {
                        actionRequests.add(command to payloadJson)
                        actionCalls += 1
                    }
                    val gate = actionGates[command]
                    if (gate != null) {
                        gate.await()
                    } else if (gateActions) {
                        actionGate.await()
                    }
                    actionsCompleted = true
                    when {
                        zeroMove && command == MobileCommand.Move -> {
                            """{"ok":true,"moved":0}"""
                        }

                        actionFails -> {
                            """{"error":{"message":"Action rejected"}}"""
                        }

                        else -> {
                            val sourceId = payloadJson.jsonStringValue("thread_id")
                            val destination =
                                when (command) {
                                    MobileCommand.Archive -> "Archive"
                                    MobileCommand.Delete -> "Trash"
                                    MobileCommand.Move -> payloadJson.jsonStringValue("target_folder_id")
                                    else -> ""
                                }
                            val change = """{"account_id":"a","removed":true,"source_folder":"$listedFolder","thread_id":"$sourceId"}"""
                            when {
                                command == MobileCommand.EmptyFolder -> {
                                    """{"ok":true}"""
                                }

                                command == MobileCommand.Delete && permanentDelete -> {
                                    """{"change":$change,"deleted":1,"ok":true,"permanent":true}"""
                                }

                                else -> {
                                    val destinationId =
                                        if (sourceId.contains('#')) {
                                            "${sourceId.substringBefore('#')}#$destination#${sourceId.substringAfterLast('#')}"
                                        } else {
                                            "a:$destination:t1"
                                        }
                                    if (command == MobileCommand.Delete) {
                                        """{"change":$change,"deleted":1,"ok":true,"thread_id":"$destinationId","trash":"$destination"}"""
                                    } else {
                                        """{"change":$change,"folder":"$destination","moved":1,"ok":true,"thread_id":"$destinationId"}"""
                                    }
                                }
                            }
                        }
                    }
                }

                else -> {
                    "{}"
                }
            }

        private fun card(
            messageCount: Int,
            hasDraft: Boolean,
        ): String =
            """{"threads":[{"id":"a:$listedFolder:t1","account_id":"a","folder_id":"$listedFolder","subject":"Re: hello",""" +
                """"message_count":$messageCount,"has_draft":$hasDraft,"date":1}]}"""

        override fun events(): CoreEventStream =
            object : CoreEventStream {
                override fun subscribe(listener: (CoreEvent) -> Unit): CloseableHandle = CloseableHandle {}
            }

        override suspend fun protocolVersion(): Int = 0
    }

    protected class MemoryPreferences : AppPreferences {
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

    protected class NoopPlatformServices : PlatformServices {
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

    protected class NoopLocaleController : LocaleController {
        override fun systemLanguageTag(): String = ""

        override fun applySystem(tag: String) {}

        override fun deviceLanguageTag(): String = "en-US"

        override fun displayName(tag: String): String = tag
    }
}
