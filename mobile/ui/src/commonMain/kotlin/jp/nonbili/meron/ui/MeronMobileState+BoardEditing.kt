// Applies mobile board and column edits to the saved configuration.
// The editor creates, reorders and updates columns using their account and folder identity.
// Folder renames and saved searches update the corresponding board entries.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.size
import androidx.compose.ui.input.key.key
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.SyncMailParams
import jp.nonbili.meron.shared.accountSummaryIsRss
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Append a board holding [columns] and make it the active one. */
private fun MeronMobileState.addKanbanBoard(columns: List<KanbanColumnSpec>): KanbanBoardSpec {
    val board =
        defaultKanbanBoard(coreAccounts).copy(
            name = trs("mobile.ios.boardNumberName", mapOf("number" to kanbanBoards.size + 1)),
            columns = columns,
        )
    persistKanbanBoards(kanbanBoards + board)
    activeKanbanBoardId = board.id
    saveActiveKanbanBoardId(kanbanPrefs, board.id)
    return board
}

internal fun MeronMobileState.createKanbanBoard(): String {
    val board = addKanbanBoard(defaultKanbanBoard(coreAccounts).columns)
    loadKanbanBoard(refresh = false)
    return board.id
}

internal fun MeronMobileState.updateKanbanBoard(
    boardId: String,
    name: String,
    avatarUrl: String,
    wallpaperPresetId: String,
    wallpaperUrl: String,
) {
    val trimmedName = name.trim()
    if (trimmedName.isBlank()) return
    persistKanbanBoards(
        kanbanBoards.map { board ->
            if (board.id == boardId) {
                board.copy(
                    name = trimmedName,
                    avatarUrl = avatarUrl.trim(),
                    wallpaperPresetId = boardWallpaperPresetId(wallpaperPresetId),
                    wallpaperUrl = wallpaperUrl.trim(),
                )
            } else {
                board
            }
        },
    )
}

internal fun MeronMobileState.deleteKanbanBoard(boardId: String) {
    // Deleting the last board leaves no board at all; the kanban screen and the
    // drawer both render that empty state, and reseeding a default here would
    // make the delete look like it did nothing.
    val wasActive = boardId == activeKanbanBoardId
    persistKanbanBoards(kanbanBoards.filterNot { it.id == boardId })
    // persistKanbanBoards has already moved the selection off the deleted board,
    // so drop the cached columns and load whatever it landed on (if anything).
    if (wasActive) {
        kanbanColumns = emptyMap()
        loadKanbanBoard(refresh = false)
    }
}

internal fun MeronMobileState.addKanbanColumn(column: KanbanColumnSpec) {
    val board = kanbanBoards.firstOrNull { it.id == activeKanbanBoardId } ?: return
    if (board.columns.any { kanbanColumnKey(it) == kanbanColumnKey(column) }) return
    persistKanbanBoards(
        kanbanBoards.map {
            if (it.id == board.id) it.copy(columns = it.columns + column) else it
        },
    )
    loadKanbanColumn(column, refresh = true)
}

/**
 * Replace the active board's columns with [columns] (the selection from the add-column
 * dialog), preserving the relative order of existing columns and appending new ones.
 * Loads any newly added column and drops cached data for removed ones. With no board
 * left at all, the selection creates one.
 */
internal fun MeronMobileState.applyKanbanColumns(columns: List<KanbanColumnSpec>) {
    val deduped = columns.distinctBy(::kanbanColumnKey)
    // Every board can be deleted, and the empty kanban screen still offers "Add
    // column", so a selection made with no board left has to bring one with it.
    val active = kanbanBoards.firstOrNull { it.id == activeKanbanBoardId }
    if (active == null && deduped.isEmpty()) return
    val board = active ?: addKanbanBoard(emptyList())
    val nextKeys = deduped.map(::kanbanColumnKey).toSet()
    val existingKeys = board.columns.map(::kanbanColumnKey).toSet()
    if (nextKeys == existingKeys) return
    // Keep existing columns in their current order, then append newly selected ones.
    val ordered =
        board.columns.filter { kanbanColumnKey(it) in nextKeys } +
            deduped.filter { kanbanColumnKey(it) !in existingKeys }
    persistKanbanBoards(
        kanbanBoards.map { if (it.id == board.id) it.copy(columns = ordered) else it },
    )
    (existingKeys - nextKeys).forEach { kanbanColumns = kanbanColumns - it }
    ordered
        .filter { kanbanColumnKey(it) !in existingKeys }
        .forEach { loadKanbanColumn(it, refresh = true) }
}

internal fun MeronMobileState.removeKanbanColumn(column: KanbanColumnSpec) {
    val key = kanbanColumnKey(column)
    persistKanbanBoards(
        kanbanBoards.map {
            if (it.id ==
                activeKanbanBoardId
            ) {
                it.copy(columns = it.columns.filterNot { existing -> kanbanColumnKey(existing) == key })
            } else {
                it
            }
        },
    )
    kanbanColumns = kanbanColumns - key
}

/**
 * Drop every column showing a folder, saved searches in it included, on all
 * boards. Used once the folder is gone from the server: a column left behind
 * would only fail to load.
 */
internal fun MeronMobileState.removeKanbanColumnsForFolder(
    accountId: String,
    folderId: String,
) {
    val showsFolder = { column: KanbanColumnSpec -> column.accountId == accountId && column.folderId == folderId }
    val keys = kanbanBoards.flatMap { board -> board.columns.filter(showsFolder).map(::kanbanColumnKey) }.toSet()
    persistKanbanBoards(
        kanbanBoards.map { board ->
            board.copy(columns = board.columns.filterNot(showsFolder))
        },
    )
    kanbanColumns = kanbanColumns - keys
}

/**
 * Follow a folder rename on every board: columns on the old names, saved
 * searches in them included, show the new ones in the same slots. A board that
 * already shows the new name keeps that column alone. [renames] maps each old
 * folder name to its new one.
 */
internal fun MeronMobileState.renameKanbanColumnsForFolder(
    accountId: String,
    renames: Map<String, String>,
) {
    val renamedKeys = mutableMapOf<String, String>()
    persistKanbanBoards(
        kanbanBoards.map { board ->
            val columns = mutableListOf<KanbanColumnSpec>()
            board.columns.forEach { column ->
                val folderId = if (column.accountId == accountId) renames[column.folderId] else null
                val target = folderId?.let { column.copy(folderId = it) } ?: column
                if (folderId != null) renamedKeys[kanbanColumnKey(column)] = kanbanColumnKey(target)
                if (columns.none { kanbanColumnKey(it) == kanbanColumnKey(target) }) columns += target
            }
            board.copy(columns = columns)
        },
    )
    renamedKeys[kanbanSearchScope]?.let(::persistKanbanSearchScope)
    // Cached pages name the old folders, and the rename dropped the folders'
    // cached mail, so the renamed columns fetch afresh, on every board: opening
    // a board later reads its saved searches from the cache alone.
    // A load still in flight under an old name would bring it back.
    renamedKeys.keys.forEach { key -> kanbanColumnLoadTokens[key] = (kanbanColumnLoadTokens[key] ?: 0L) + 1 }
    kanbanColumns = kanbanColumns - renamedKeys.keys
    // Other columns (Starred, unified) may hold cards from the renamed
    // folders, under ids that name the old folders.
    val holdsRenamed =
        kanbanColumns
            .filterValues { state -> state.threads.any { it.accountId == accountId && it.folder in renames } }
            .keys
    kanbanBoards
        .flatMap { it.columns }
        .filter { kanbanColumnKey(it) in renamedKeys.values || kanbanColumnKey(it) in holdsRenamed }
        .distinctBy(::kanbanColumnKey)
        .forEach { loadKanbanColumn(it, refresh = true) }
}

/**
 * Turn the board search on [column] into a column of its own, placed right after it.
 *
 * A search scoped to just that column has done its job then, so it clears; a board-wide one stays.
 * [clearsScopedSearch] is false when [query] came from the search options rather than the board
 * search, which it then leaves alone.
 *
 * Does nothing when there is nothing to save or the board already has it, or for a saved search
 * narrowed by the board search: its two queries would have to share one line.
 */
internal fun MeronMobileState.saveKanbanSearchColumn(
    column: KanbanColumnSpec,
    query: String,
    clearsScopedSearch: Boolean = true,
) {
    val saved = normalizeKanbanColumnQuery(query)
    if (saved.isBlank() || column.query.isNotBlank()) return
    val board = kanbanBoards.firstOrNull { it.id == activeKanbanBoardId } ?: return
    val target = column.copy(query = saved)
    val sourceKey = kanbanColumnKey(column)
    if (board.columns.any { kanbanColumnKey(it) == kanbanColumnKey(target) }) return
    val at = board.columns.indexOfFirst { kanbanColumnKey(it) == sourceKey }
    val columns = board.columns.toMutableList().apply { add(if (at < 0) size else at + 1, target) }
    persistKanbanBoards(kanbanBoards.map { if (it.id == board.id) it.copy(columns = columns) else it })
    loadKanbanColumn(target, refresh = true)
    if (clearsScopedSearch && kanbanSearchScope == sourceKey) {
        persistKanbanSearch("")
        loadKanbanColumn(column, refresh = false)
    }
}

/**
 * Give a saved search column a new query, keeping its slot on the board. Does
 * nothing when the query is empty, unchanged, or already has a column here.
 */
internal fun MeronMobileState.editKanbanSearchColumn(
    column: KanbanColumnSpec,
    query: String,
) {
    val next = normalizeKanbanColumnQuery(query)
    if (column.query.isBlank() || next.isBlank() || next == column.query) return
    replaceKanbanColumn(column, column.copy(query = next))
}

/**
 * Point an existing column at another folder of the same account, keeping its slot
 * on the board. Does nothing when the folder is unchanged or already has its own
 * column here — the board must not end up with duplicates.
 */
internal fun MeronMobileState.switchKanbanColumnFolder(
    column: KanbanColumnSpec,
    folderId: String,
) {
    if (folderId.isBlank() || kanbanFolderIdsEqual(folderId, column.folderId)) return
    replaceKanbanColumn(column, column.copy(folderId = folderId))
}

// Swap [column] for [target] in place and load it. Does nothing when the board
// has no such column or already shows the target (a saved search only collides
// with the same search on the same folder).
private fun MeronMobileState.replaceKanbanColumn(
    column: KanbanColumnSpec,
    target: KanbanColumnSpec,
) {
    val board = kanbanBoards.firstOrNull { it.id == activeKanbanBoardId } ?: return
    val fromKey = kanbanColumnKey(column)
    val toKey = kanbanColumnKey(target)
    if (board.columns.none { kanbanColumnKey(it) == fromKey }) return
    if (board.columns.any { it.accountId == target.accountId && kanbanFolderIdsEqual(it.folderId, target.folderId) && it.query == target.query }) return
    persistKanbanBoards(
        kanbanBoards.map { existing ->
            if (existing.id != board.id) {
                existing
            } else {
                existing.copy(columns = existing.columns.map { if (kanbanColumnKey(it) == fromKey) target else it })
            }
        },
    )
    if (kanbanSearchScope == fromKey) persistKanbanSearchScope(toKey)
    // Keep the old column's cached page only while another board still shows it.
    if (kanbanBoards.none { it.columns.any { existing -> kanbanColumnKey(existing) == fromKey } }) {
        kanbanColumns = kanbanColumns - fromKey
    }
    loadKanbanColumn(target, refresh = true)
}

/**
 * Fetch an account's folder list for a folder picker (a kanban column's or the
 * mail list's) when only the bootstrap inbox is cached, so the picker isn't
 * limited to what a sync happened to surface.
 */
internal fun MeronMobileState.ensureAccountFolders(accountId: String) {
    if (!coreLoaded || accountId == UNIFIED_ACCOUNT_ID) return
    val account = coreAccounts.firstOrNull { it.id == accountId } ?: return
    if (accountSummaryIsRss(account)) return
    if (foldersByAccount[accountId].orEmpty().size > 1) return
    scope.launch {
        val folderReadVersion = folderReadGuard.version
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                withManagedGoogleAuth(client, account.id) {
                    client.sync(
                        SyncMailParams(
                            accountId = account.id,
                            folderId = INBOX_FOLDER,
                            limit = 1,
                            folders = true,
                            deferTail = true,
                        ),
                    )
                }
                loadAccountFolders(client, account)
            }
        }.onSuccess { folders ->
            if (folders.isNotEmpty()) foldersByAccount = foldersByAccount + (accountId to reconcileFolderUnread(folders, folderReadVersion))
        }
    }
}

internal fun MeronMobileState.moveKanbanColumn(
    column: KanbanColumnSpec,
    delta: Int,
) {
    persistKanbanBoards(
        kanbanBoards.map { board ->
            if (board.id != activeKanbanBoardId) return@map board
            val columns = board.columns.toMutableList()
            val index = columns.indexOfFirst { kanbanColumnKey(it) == kanbanColumnKey(column) }
            val target = (index + delta).coerceIn(0, columns.lastIndex)
            if (index < 0 || index == target) {
                board
            } else {
                val item = columns.removeAt(index)
                columns.add(target, item)
                board.copy(columns = columns)
            }
        },
    )
}
