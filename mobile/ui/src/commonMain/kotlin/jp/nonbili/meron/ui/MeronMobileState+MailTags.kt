// Mobile mailbox state changes for mail tags.
// Called by the Compose routes and their thread actions.
// Server responses are reconciled with the local mailbox state here.
package jp.nonbili.meron.ui

import androidx.compose.material.icons.filled.Star
import jp.nonbili.meron.shared.MarkStarredParams
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.RssMarkStarredParams
import jp.nonbili.meron.shared.SetTagsParams
import jp.nonbili.meron.shared.TagCount
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.parseReplaceTagResponse
import jp.nonbili.meron.shared.parseReplacedQueries
import jp.nonbili.meron.shared.parseSetTagsResponse
import jp.nonbili.meron.shared.parseTagCountsResponse
import jp.nonbili.meron.shared.requireCoreOk
import jp.nonbili.meron.shared.threadIdIsRss
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal fun MeronMobileState.refreshUnifiedStarredAfterFlagChange() {
    mailboxCache = mailboxCache.filterKeys { it.accountId != UNIFIED_ACCOUNT_ID || !isUnifiedStarredFolder(it.folderId) }
    if (selectedCoreAccountId != UNIFIED_ACCOUNT_ID || !isUnifiedStarredFolder(selectedCoreFolder)) return
    syncCoreThreads(syncFirst = false)
}

internal fun MeronMobileState.toggleStar(thread: ThreadSummary) {
    val backendThreadId = thread.backendThreadId()
    val isRssThread = threadIdIsRss(backendThreadId)
    val itemIds = listOf(thread.id).takeIf { backendThreadId != thread.id }.orEmpty()
    runCoreThreadAction(
        thread = thread,
        label = if (thread.starred) "Unstar" else "Star",
        doneStatus = trs(if (thread.starred) "mobile.status.starRemoved" else "mobile.status.starAdded"),
        failedStatus = trs(if (thread.starred) "mobile.status.unstarFailed" else "mobile.status.starFailed"),
        action = {
            if (isRssThread) {
                markRssStarred(
                    RssMarkStarredParams(
                        threadId = backendThreadId,
                        starred = !thread.starred,
                        itemKeys = thread.rssItemKeys(),
                    ),
                )
            } else {
                markStarred(MarkStarredParams(threadId = backendThreadId, starred = !thread.starred, messageIds = itemIds))
            }
        },
        update = { threads -> threads.map { if (it.id == thread.id) it.copy(starred = !thread.starred) else it } },
        afterSuccess = {
            refreshUnifiedStarredAfterFlagChange()
        },
    )
}

/**
 * Save a tag change and show the tags the core answers with. Tags are stored
 * locally, so this waits on the database, never the server; a failure means the
 * core refused the tag.
 */
internal fun MeronMobileState.changeThreadTags(
    thread: ThreadSummary,
    add: List<String> = emptyList(),
    remove: List<String> = emptyList(),
    onDone: (List<String>?) -> Unit,
) {
    val backendThreadId = thread.backendThreadId()
    val itemIds = listOf(thread.id).takeIf { backendThreadId != thread.id }.orEmpty()
    scope.launch {
        val tags =
            runCatching {
                parseSetTagsResponse(
                    requireCoreOk(
                        withContext(ioDispatcher) {
                            MobileMailCommandClient(core).setTags(
                                SetTagsParams(threadId = backendThreadId, add = add, remove = remove, messageIds = itemIds),
                            )
                        },
                    ),
                )
            }.getOrNull()
        if (tags != null) {
            updateThreadEverywhere(thread) { it.copy(tags = tags) }
            reloadTagColumns()
        }
        onDone(tags)
    }
}

/** A tag column may gain or lose threads, which no sync announces. */
private fun MeronMobileState.reloadTagColumns() {
    kanbanBoards
        .firstOrNull { it.id == activeKanbanBoardId }
        ?.columns
        ?.filter { kanbanColumnReadsTags(it.copy(query = "${it.query} $kanbanSearch")) }
        ?.forEach { loadKanbanColumn(it) }
}

/**
 * Rename [tag] everywhere, or delete it with no [to], and report to [onDone] whether
 * the core took it. It handles mail and rules; boards live here, so their saved
 * searches go through it to be rewritten. Two columns that come to mean the same
 * search become one, and a search scoped to a column stays on it. On the app's
 * scope, not a dialog's: closing one mid-way must not leave the boards behind.
 */
internal fun MeronMobileState.replaceTag(
    tag: String,
    to: String?,
    onDone: (Boolean) -> Unit,
) {
    val queries = kanbanBoards.flatMap { board -> board.columns.map { it.query } }.filter { it.isNotBlank() }.distinct()
    scope.launch {
        val answer =
            runCatching {
                requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).replaceTag(tag, to, queries) })
            }.getOrNull()
        if (answer != null) applyTagReplacement(tag, parseReplaceTagResponse(answer), queries.zip(parseReplacedQueries(answer)).toMap())
        onDone(answer != null)
    }
}

private fun MeronMobileState.applyTagReplacement(
    tag: String,
    next: String?,
    renamed: Map<String, String>,
) {
    fun swap(thread: ThreadSummary): ThreadSummary =
        if (tag !in thread.tags) {
            thread
        } else {
            thread.copy(tags = (thread.tags - tag + listOfNotNull(next)).distinct().sorted())
        }
    coreThreads = coreThreads.map(::swap)
    selectedCoreThread = selectedCoreThread?.let(::swap)
    kanbanColumns = kanbanColumns.mapValues { (_, state) -> state.copy(threads = state.threads.map(::swap)) }
    // A cached search may no longer list the same mail; a cached mailbox does.
    mailboxCache =
        mailboxCache
            .filterKeys { it.query.isBlank() }
            .mapValues { (_, cached) -> cached.copy(threads = cached.threads.map(::swap)) }

    fun rewrite(column: KanbanColumnSpec) = column.copy(query = renamed[column.query] ?: column.query)
    kanbanBoards
        .flatMap { it.columns }
        .firstOrNull { kanbanColumnKey(it) == kanbanSearchScope }
        ?.let { persistKanbanSearchScope(kanbanColumnKey(rewrite(it))) }
    persistKanbanBoards(
        kanbanBoards.map { board ->
            board.copy(columns = board.columns.map(::rewrite).distinctBy(::kanbanColumnKey))
        },
    )
    val shown = kanbanBoards.flatMap { it.columns }.map(::kanbanColumnKey).toSet()
    kanbanColumns = kanbanColumns.filterKeys { it in shown }
    loadKanbanBoard()
    // Read the list again: a search may gain or lose results, and a read from
    // before this change must not be the last word.
    syncCoreThreads(
        accountOverride = selectedCoreAccountId,
        folderOverride = selectedCoreFolder,
        syncFirst = false,
    )
}

internal suspend fun MeronMobileState.loadTagCounts(): List<TagCount> =
    runCatching {
        parseTagCountsResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).tagList() }))
    }.getOrDefault(emptyList())
