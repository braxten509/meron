// Serializes mobile boards and restores their columns
// and appearance. A never-configured installation and
// an explicitly empty board list have different
// meanings.

package jp.nonbili.meron.ui

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.addJsonObject
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import kotlin.uuid.ExperimentalUuidApi
import kotlin.uuid.Uuid

private val kanbanJson = Json { ignoreUnknownKeys = true }

private fun String.orFallback(fallback: String) = ifBlank { fallback }

/**
 * A board's stored preset, blank for none. Plain is the board's unset state, not
 * a style of its own: the picker shows it selected when nothing is set, and any
 * preset makes the board styled, so a stored Plain would leave no way back to
 * the default board. Desktop's boardWallpaper() makes the same call.
 */
internal fun boardWallpaperPresetId(presetId: String): String = presetId.trim().takeUnless { it == "plain" }.orEmpty()

@OptIn(ExperimentalUuidApi::class)
internal fun parseKanbanBoards(raw: String): List<KanbanBoardSpec> {
    if (raw.isBlank()) return emptyList()
    return runCatching {
        val array = kanbanJson.parseToJsonElement(raw) as? JsonArray ?: return emptyList()
        array.mapNotNull { element ->
            val fields = element.jsonObject
            val id = (fields["id"]?.asStringOrEmpty()).orEmpty().orFallback("kb-${Uuid.random()}")
            val name = (fields["name"]?.asStringOrEmpty()).orEmpty().orFallback("Kanban board")
            val avatarUrl = fields["avatarUrl"]?.asStringOrEmpty().orEmpty()
            val wallpaper = fields["wallpaper"]?.let { runCatching { it.jsonObject }.getOrNull() }
            val wallpaperPresetId = boardWallpaperPresetId(wallpaper?.get("presetId")?.asStringOrEmpty().orEmpty())
            val wallpaperUrl = wallpaper?.get("url")?.asStringOrEmpty().orEmpty()
            val columns =
                (fields["columns"]?.let { runCatching { it.jsonArray }.getOrNull() } ?: JsonArray(emptyList()))
                    .mapNotNull { col ->
                        val columnFields = runCatching { col.jsonObject }.getOrNull() ?: return@mapNotNull null
                        val accountId = columnFields["accountId"]?.asStringOrEmpty().orEmpty()
                        val folderId = columnFields["folderId"]?.asStringOrEmpty().orEmpty()
                        val query = normalizeKanbanColumnQuery(columnFields["query"]?.asStringOrEmpty().orEmpty())
                        if (accountId.isNotBlank() && folderId.isNotBlank()) KanbanColumnSpec(accountId, folderId, query) else null
                    }.distinctBy(::kanbanColumnKey)
            KanbanBoardSpec(id, name, columns, avatarUrl, wallpaperPresetId, wallpaperUrl)
        }
    }.getOrDefault(emptyList())
}

private fun kotlinx.serialization.json.JsonElement.asStringOrEmpty(): String = runCatching { (this as kotlinx.serialization.json.JsonPrimitive).content }.getOrDefault("")

internal fun serializeKanbanBoards(boards: List<KanbanBoardSpec>): String =
    buildJsonArray {
        boards.forEach { board ->
            addJsonObject {
                put("id", board.id)
                put("name", board.name)
                put(
                    "columns",
                    buildJsonArray {
                        board.columns.forEach { column ->
                            addJsonObject {
                                put("accountId", column.accountId)
                                put("folderId", column.folderId)
                                if (column.query.isNotBlank()) put("query", column.query)
                            }
                        }
                    },
                )
                if (board.avatarUrl.isNotBlank()) put("avatarUrl", board.avatarUrl)
                when {
                    board.wallpaperUrl.isNotBlank() -> {
                        put(
                            "wallpaper",
                            buildJsonObject {
                                put("kind", "custom")
                                put("url", board.wallpaperUrl)
                            },
                        )
                    }

                    board.wallpaperPresetId.isNotBlank() -> {
                        put(
                            "wallpaper",
                            buildJsonObject {
                                put("kind", "preset")
                                put("presetId", board.wallpaperPresetId)
                            },
                        )
                    }
                }
            }
        }
    }.toString()

internal fun ensureKanbanDefaults(
    prefs: AppPreferences,
    boards: List<KanbanBoardSpec>,
    accounts: List<jp.nonbili.meron.shared.AccountSummary>,
): List<KanbanBoardSpec> {
    // Only seed default columns for a brand-new board. Existing boards are left
    // exactly as the user arranged them — re-adding the unified inbox (or a
    // per-account inbox) here would undo deliberate removals on every restart.
    if (boards.isNotEmpty()) return boards
    // A stored (even empty) list means the user has boards of their own, or
    // deleted every one of them; only a profile that never stored any gets the
    // starter board, otherwise deleting the last board would undo itself.
    if (prefs.getString(KANBAN_BOARDS_PREF, "").isNotBlank()) return boards
    // The starter board is stored, so it is named in the app's language once.
    val name = localizedString(loadAppLanguageTag(prefs).ifBlank { "en" }, "kanban.board.defaultName")
    val next = listOf(defaultKanbanBoard(accounts, name))
    saveKanbanBoards(prefs, next)
    return next
}

internal fun saveKanbanBoards(
    prefs: AppPreferences,
    boards: List<KanbanBoardSpec>,
) = prefs.putString(KANBAN_BOARDS_PREF, serializeKanbanBoards(boards))

internal fun loadActiveKanbanBoardId(prefs: AppPreferences): String = prefs.getString(ACTIVE_KANBAN_BOARD_PREF, "")

internal fun saveActiveKanbanBoardId(
    prefs: AppPreferences,
    boardId: String,
) = prefs.putString(ACTIVE_KANBAN_BOARD_PREF, boardId)

internal fun loadKanbanFilter(prefs: AppPreferences): FilterMode =
    when (prefs.getString(KANBAN_FILTER_PREF, "all")) {
        "unread" -> FilterMode.Unread
        "starred" -> FilterMode.Starred
        else -> FilterMode.All
    }

internal fun saveKanbanFilter(
    prefs: AppPreferences,
    filter: FilterMode,
) = prefs.putString(
    KANBAN_FILTER_PREF,
    when (filter) {
        FilterMode.All -> "all"
        FilterMode.Unread -> "unread"
        FilterMode.Starred -> "starred"
    },
)

internal fun loadKanbanAttachmentsOnly(prefs: AppPreferences): Boolean = prefs.getBoolean(KANBAN_ATTACHMENTS_PREF, false)

internal fun saveKanbanAttachmentsOnly(
    prefs: AppPreferences,
    on: Boolean,
) = prefs.putBoolean(KANBAN_ATTACHMENTS_PREF, on)

internal fun loadKanbanSearch(prefs: AppPreferences): String = prefs.getString(KANBAN_SEARCH_PREF, "")

internal fun saveKanbanSearch(
    prefs: AppPreferences,
    search: String,
) = prefs.putString(KANBAN_SEARCH_PREF, search)

internal fun loadKanbanSearchScope(prefs: AppPreferences): String = prefs.getString(KANBAN_SEARCH_SCOPE_PREF, "all").ifBlank { "all" }

internal fun saveKanbanSearchScope(
    prefs: AppPreferences,
    scope: String,
) = prefs.putString(KANBAN_SEARCH_SCOPE_PREF, scope.ifBlank { "all" })
