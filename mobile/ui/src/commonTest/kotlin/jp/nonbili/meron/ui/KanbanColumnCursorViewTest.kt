// An issued board cursor belongs to a specific
// search and filter. The tests exercise loading its
// next page while the same column can also be
// reloading.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.CloseableHandle
import jp.nonbili.meron.shared.CoreEvent
import jp.nonbili.meron.shared.CoreEventStream
import jp.nonbili.meron.shared.MeronCore
import jp.nonbili.meron.shared.MobileCommand
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * A column's cursor continues the listing that produced it. The board's search
 * and filters can change before the reload replacing that listing lands, so the
 * next page is asked for with the view the cursor was issued for, and not at
 * all while a reload is out.
 */
class KanbanColumnCursorViewTest {
    @Test
    fun loadMorePagesWithTheSearchAndFiltersTheCursorWasIssuedFor() =
        runBlocking {
            val core = RecordingCore()
            val state = state(core, this)
            val column = KanbanColumnSpec(accountId = "a", folderId = "INBOX")
            val key = kanbanColumnKey(column)
            state.kanbanSearch = "alpha"
            state.kanbanSearchScope = "all"
            state.kanbanAttachmentsOnly = true
            state.kanbanFilter = FilterMode.All

            state.loadKanbanColumn(column)
            waitUntil { state.kanbanColumns[key]?.nextCursor == "cursor-a" }
            assertEquals(
                KanbanColumnView(query = "alpha", filter = FilterMode.All, attachmentsOnly = true),
                state.kanbanColumns[key]?.cursorView,
            )

            // The board moves on before search B's first page replaces the list.
            state.kanbanSearch = "beta"
            state.kanbanAttachmentsOnly = false
            state.kanbanFilter = FilterMode.Unread
            state.loadMoreKanbanColumn(column)
            waitUntil { core.threadLists.size == 2 }

            val page = core.threadLists.last()
            assertEquals("alpha", page["query"]?.jsonPrimitive?.content)
            assertEquals("all", page["filter"]?.jsonPrimitive?.content)
            assertEquals("true", page["attachments"]?.jsonPrimitive?.content)
            assertEquals("cursor-a", page["before_cursor"]?.jsonPrimitive?.content)
        }

    @Test
    fun loadMoreWaitsOutAReloadInFlight() =
        runBlocking {
            val core = RecordingCore()
            val state = state(core, this)
            val column = KanbanColumnSpec(accountId = "a", folderId = "INBOX")
            val key = kanbanColumnKey(column)
            state.updateKanbanColumn(key) { it.copy(loading = true, nextCursor = "cursor-a") }

            state.loadMoreKanbanColumn(column)
            delay(50)

            assertEquals(0, core.threadLists.size)
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

    /** Answers every thread list with one card and a continuation cursor,
     *  recording each request's payload. */
    private class RecordingCore : MeronCore {
        val threadLists = mutableListOf<JsonObject>()

        override suspend fun invoke(
            command: String,
            payloadJson: String,
        ): String =
            when (command) {
                MobileCommand.FolderList -> {
                    """{"folders":[{"account_id":"a","name":"INBOX","role":"inbox"}]}"""
                }

                MobileCommand.ThreadList -> {
                    threadLists += Json.parseToJsonElement(payloadJson).jsonObject
                    """{"threads":[{"id":"a:INBOX:t${threadLists.size}","account_id":"a","folder_id":"INBOX",""" +
                        """"subject":"hello","date":${threadLists.size}}],"next_cursor":"cursor-a"}"""
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
