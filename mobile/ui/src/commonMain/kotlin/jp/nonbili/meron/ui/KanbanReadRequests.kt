// Builds a mark-read request across mobile board
// columns. Overlapping threads share one write and
// the caller receives the resulting successes and
// failures.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.ThreadSummary
import kotlinx.coroutines.CancellationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

// A column's mark-read targets, captured before the optimistic clear so a failed
// write can restore exactly what it cleared.
internal data class KanbanMarkReadPlan(
    val column: KanbanColumnSpec,
    val key: String,
    val unread: List<ThreadSummary>,
    /** Marked thread by thread (the starred aggregate, a saved search), not folder-wide. */
    val perItem: Boolean,
    val mailAccounts: List<AccountSummary>,
    val writes: Boolean,
    val unreadCountBefore: Int?,
    // Drawer folder badges this column's writes cover: the totals to show right
    // away, keyed by account and mailbox, and the ones to put back if it fails.
    val folderUnread: Map<Pair<String, String>, Int> = emptyMap(),
    val folderUnreadBefore: Map<Pair<String, String>, Int> = emptyMap(),
)

// One board action executes sequentially and shares both successful and failed
// writes across overlapping columns, so a failed target is not retried implicitly.
internal class KanbanReadRequests {
    private val results = mutableMapOf<String, Result<String>>()

    suspend fun run(
        key: String,
        write: suspend () -> String,
    ): String {
        val result =
            results[key] ?: runCatching {
                val response = write()
                val value = Json.parseToJsonElement(response) as? JsonObject
                check(value?.get("ok") != JsonPrimitive(false) && (value?.get("failures") as? JsonArray).isNullOrEmpty()) {
                    "Mark read failed: $response"
                }
                response
            }.also {
                val error = it.exceptionOrNull()
                if (error is CancellationException) throw error
                results[key] = it
            }
        return result.getOrThrow()
    }
}
