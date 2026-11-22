// A new profile receives a starter board, while a
// deliberately empty board list must remain empty. These
// cases exercise that distinction across persistence and
// reload.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.CloseableHandle
import jp.nonbili.meron.shared.CoreEvent
import jp.nonbili.meron.shared.CoreEventStream
import jp.nonbili.meron.shared.MeronCore
import kotlinx.coroutines.CoroutineScope
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class KanbanBoardPersistenceTest {
    @Test
    fun seedsAStarterBoardForAProfileThatNeverStoredAny() {
        val prefs = FakePreferences()

        val boards = ensureKanbanDefaults(prefs, emptyList(), emptyList())

        assertEquals(1, boards.size)
        assertTrue(parseKanbanBoards(prefs.getString(KANBAN_BOARDS_PREF, "")).size == 1)
    }

    @Test
    fun keepsAnEmptyBoardListTheUserDeletedDownTo() {
        val prefs = FakePreferences()
        saveKanbanBoards(prefs, emptyList())

        assertEquals(emptyList(), ensureKanbanDefaults(prefs, emptyList(), emptyList()))
    }

    @Test
    fun leavesStoredBoardsAlone() {
        val prefs = FakePreferences()
        val stored = listOf(defaultKanbanBoard(emptyList<AccountSummary>()))
        saveKanbanBoards(prefs, stored)

        assertEquals(stored.map { it.id }, ensureKanbanDefaults(prefs, stored, emptyList()).map { it.id })
    }

    @Test
    fun deletingTheLastBoardLeavesNoBoard() {
        val prefs = FakePreferences()
        saveKanbanBoards(prefs, listOf(defaultKanbanBoard(emptyList<AccountSummary>())))
        val state = testState(prefs)
        val board = state.kanbanBoards.single()
        state.activeKanbanBoardId = board.id

        state.deleteKanbanBoard(board.id)

        assertEquals(emptyList(), state.kanbanBoards)
        assertEquals("", state.activeKanbanBoardId)
        assertEquals(emptyList(), parseKanbanBoards(prefs.getString(KANBAN_BOARDS_PREF, "")))
    }

    @Test
    fun aColumnSelectionMadeWithNoBoardLeftCreatesOne() {
        val state = testState(FakePreferences())
        val column = KanbanColumnSpec(UNIFIED_ACCOUNT_ID, INBOX_FOLDER)

        state.applyKanbanColumns(listOf(column))

        val board = state.kanbanBoards.single()
        assertEquals(listOf(column), board.columns)
        assertEquals(board.id, state.activeKanbanBoardId)
    }

    @Test
    fun anEmptySelectionWithNoBoardLeftCreatesNothing() {
        val state = testState(FakePreferences())

        state.applyKanbanColumns(emptyList())

        assertEquals(emptyList(), state.kanbanBoards)
        assertEquals("", state.activeKanbanBoardId)
    }

    @Test
    fun pickingPlainClearsTheBoardBackground() {
        val prefs = FakePreferences()
        saveKanbanBoards(prefs, listOf(defaultKanbanBoard(emptyList<AccountSummary>()).copy(wallpaperPresetId = "dots")))
        val state = testState(prefs)
        val board = state.kanbanBoards.single()

        state.updateKanbanBoard(board.id, board.name, board.avatarUrl, "plain", "")

        assertEquals("", state.kanbanBoards.single().wallpaperPresetId)
        assertEquals(false, state.kanbanBoards.single().hasBoardStyle())
    }

    @Test
    fun aStoredPlainBackgroundReadsAsNone() {
        val raw = """[{"id":"kb-1","name":"Board","columns":[],"wallpaper":{"kind":"preset","presetId":"plain"}}]"""

        assertEquals("", parseKanbanBoards(raw).single().wallpaperPresetId)
    }

    @Test
    fun aSavedSearchColumnRoundTripsBesideItsFoldersPlainColumn() {
        val raw =
            """[{"id":"kb-1","name":"Board","columns":[""" +
                """{"accountId":"a","folderId":"INBOX"},""" +
                """{"accountId":"a","folderId":"INBOX","query":"  invoice\n due "},""" +
                """{"accountId":"a","folderId":"INBOX","query":"   "}]}]"""

        val columns = parseKanbanBoards(raw).single().columns

        // The blank query reads as the plain column, which the board already has.
        assertEquals(listOf(KanbanColumnSpec("a", "INBOX"), KanbanColumnSpec("a", "INBOX", "invoice due")), columns)
        val board = KanbanBoardSpec(id = "kb-1", name = "Board", columns = columns)
        assertEquals(columns, parseKanbanBoards(serializeKanbanBoards(listOf(board))).single().columns)
    }

    @Test
    fun aColumnKeyStillNamesItsFolderWhenAQueryFollows() {
        val saved = KanbanColumnSpec(UNIFIED_ACCOUNT_ID, STARRED_FOLDER, "invoice")
        assertEquals(UNIFIED_ACCOUNT_ID to STARRED_FOLDER, kanbanKeyAccountFolder(kanbanColumnKey(saved)))
        assertEquals("a" to "INBOX", kanbanKeyAccountFolder(kanbanColumnKey(KanbanColumnSpec("a", "INBOX"))))
    }

    @Test
    fun savingASearchPlacesItsColumnAfterTheOneItCameFrom() {
        val inbox = KanbanColumnSpec("a", "INBOX")
        val sent = KanbanColumnSpec("a", "Sent")
        val state = boardState(listOf(inbox, sent))

        state.saveKanbanSearchColumn(inbox, " invoice ")
        state.saveKanbanSearchColumn(inbox, "invoice")
        state.saveKanbanSearchColumn(inbox, "   ")

        assertEquals(listOf(inbox, inbox.copy(query = "invoice"), sent), state.kanbanBoards.single().columns)
    }

    @Test
    fun aSavedSearchIsNotSavedAgain() {
        val inbox = KanbanColumnSpec("a", "INBOX")
        val saved = inbox.copy(query = "invoice")
        val state = boardState(listOf(inbox, saved))

        state.saveKanbanSearchColumn(saved, "due")

        assertEquals(listOf(inbox, saved), state.kanbanBoards.single().columns)
    }

    @Test
    fun theBoardSearchNarrowsASavedSearchColumn() {
        val inbox = KanbanColumnSpec("a", "INBOX")
        val saved = inbox.copy(query = "from:boss")
        val state = boardState(listOf(inbox, saved))

        assertEquals("from:boss", state.kanbanColumnSearchQuery(saved))
        state.kanbanSearch = " budget "
        assertEquals("from:boss\nAND budget", state.kanbanColumnSearchQuery(saved))
        assertEquals("budget", state.kanbanColumnSearchQuery(inbox))
        state.kanbanSearchScope = kanbanColumnKey(inbox)
        assertEquals("from:boss", state.kanbanColumnSearchQuery(saved))
        state.kanbanSearchScope = kanbanColumnKey(saved)
        assertEquals("from:boss\nAND budget", state.kanbanColumnSearchQuery(saved))
        assertEquals("", state.kanbanColumnSearchQuery(inbox))
    }

    @Test
    fun narrowingDropsALeadingOrLeftStrayAfterTheAnd() {
        assertEquals("from:a\nAND from:b", narrowKanbanQuery("from:a", "OR from:b"))
        assertEquals("from:a", narrowKanbanQuery("from:a", "OR OR"))
        assertEquals("from:a\nAND ORACLE", narrowKanbanQuery("from:a", "ORACLE"))
        assertEquals("OR x", narrowKanbanQuery("", " OR x "))
    }

    @Test
    fun narrowingLeavesTheSavedQueryAsItIs() {
        assertEquals("subject:\"Q3 AND budget\nAND paid", narrowKanbanQuery("subject:\"Q3 AND budget", "paid"))
        assertEquals("27\" monitor\nAND 24\" monitor", narrowKanbanQuery("27\" monitor", "24\" monitor"))
    }

    @Test
    fun editingASavedSearchKeepsItsSlotAndRefusesADuplicate() {
        val inbox = KanbanColumnSpec("a", "INBOX")
        val saved = inbox.copy(query = "invoice")
        val other = inbox.copy(query = "receipt")
        val state = boardState(listOf(inbox, saved, other))

        state.editKanbanSearchColumn(saved, "receipt")
        state.editKanbanSearchColumn(inbox, "invoice")
        assertEquals(listOf(inbox, saved, other), state.kanbanBoards.single().columns)

        state.editKanbanSearchColumn(saved, " overdue ")
        assertEquals(listOf(inbox, inbox.copy(query = "overdue"), other), state.kanbanBoards.single().columns)
    }

    @Test
    fun renamingAFolderRepointsItsColumnsInTheirSlots() {
        val inbox = KanbanColumnSpec("a", "INBOX")
        val reports = KanbanColumnSpec("a", "Work/Reports")
        val saved = reports.copy(query = "invoice")
        val otherAccount = KanbanColumnSpec("b", "Work/Reports")
        val state = boardState(listOf(inbox, reports, saved, otherAccount))
        state.kanbanSearchScope = kanbanColumnKey(saved)

        state.renameKanbanColumnsForFolder("a", mapOf("Work" to "Jobs", "Work/Reports" to "Jobs/Reports"))

        val renamed = reports.copy(folderId = "Jobs/Reports")
        assertEquals(
            listOf(inbox, renamed, renamed.copy(query = "invoice"), otherAccount),
            state.kanbanBoards.single().columns,
        )
        assertEquals(kanbanColumnKey(renamed.copy(query = "invoice")), state.kanbanSearchScope)
    }

    @Test
    fun renamingOntoAShownFolderKeepsThatColumnAlone() {
        val work = KanbanColumnSpec("a", "Work")
        val jobs = KanbanColumnSpec("a", "Jobs")
        val state = boardState(listOf(work, jobs))

        state.renameKanbanColumnsForFolder("a", mapOf("Work" to "Jobs"))

        assertEquals(listOf(jobs), state.kanbanBoards.single().columns)
    }

    private fun boardState(columns: List<KanbanColumnSpec>): MeronMobileState {
        val prefs = FakePreferences()
        saveKanbanBoards(prefs, listOf(KanbanBoardSpec(id = "kb-1", name = "Board", columns = columns)))
        return testState(prefs).apply { activeKanbanBoardId = "kb-1" }
    }

    private fun testState(kanbanPrefs: AppPreferences): MeronMobileState {
        val core = FakeCore()
        return MeronMobileState(
            scope = CoroutineScope(EmptyCoroutineContext),
            core = core,
            coreLoaded = true,
            prefs = FakePreferences(),
            kanbanPrefs = kanbanPrefs,
            services = FakePlatformServices(),
            locale = FakeLocaleController(),
            mobileHost = DefaultMobileHost(),
            settingsMirror = SettingsMirror(core, FakePreferences()) { true },
        )
    }

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

    private class FakePreferences : AppPreferences {
        private val strings = mutableMapOf<String, String>()
        private val booleans = mutableMapOf<String, Boolean>()
        private val ints = mutableMapOf<String, Int>()
        private val stringSets = mutableMapOf<String, Set<String>>()

        override fun getString(
            key: String,
            default: String,
        ): String = strings[key] ?: default

        override fun putString(
            key: String,
            value: String,
        ) {
            strings[key] = value
        }

        override fun getBoolean(
            key: String,
            default: Boolean,
        ): Boolean = booleans[key] ?: default

        override fun putBoolean(
            key: String,
            value: Boolean,
        ) {
            booleans[key] = value
        }

        override fun getInt(
            key: String,
            default: Int,
        ): Int = ints[key] ?: default

        override fun putInt(
            key: String,
            value: Int,
        ) {
            ints[key] = value
        }

        override fun getStringSet(
            key: String,
            default: Set<String>,
        ): Set<String> = stringSets[key] ?: default

        override fun putStringSet(
            key: String,
            value: Set<String>,
        ) {
            stringSets[key] = value
        }

        override fun remove(key: String) {
            strings.remove(key)
            booleans.remove(key)
            ints.remove(key)
            stringSets.remove(key)
        }
    }

    private class FakeLocaleController : LocaleController {
        override fun systemLanguageTag(): String = ""

        override fun applySystem(tag: String) {}

        override fun deviceLanguageTag(): String = "en-US"

        override fun displayName(tag: String): String = tag
    }

    private class FakeCore : MeronCore {
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
