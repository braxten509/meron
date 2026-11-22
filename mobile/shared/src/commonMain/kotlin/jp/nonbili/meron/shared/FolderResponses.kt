// Folder changes and Meron folder membership returned to mobile callers.
// The parsers preserve stable wire identities through creation, rename and move operations.
// The public response functions expose the resulting folder records to the UI.
package jp.nonbili.meron.shared

internal fun decodeFolderListResponse(responseJson: String): List<FolderSummary> {
    val foldersJson = responseJson.findJsonArrayProperty("folders") ?: return emptyList()
    return foldersJson.jsonArrayElements().mapNotNull { item ->
        val name = item.findJsonStringProperty("name") ?: item.findJsonStringProperty("id").orEmpty()
        if (name.isBlank()) return@mapNotNull null
        FolderSummary(
            accountId = item.findJsonStringProperty("account_id").orEmpty(),
            name = name,
            unread = item.findJsonLongProperty("unread")?.toInt() ?: 0,
            role = item.findJsonStringProperty("role") ?: "folder",
            // Absent for RSS folders and older cores; the wire name is then the
            // best label available.
            displayName = item.findJsonStringProperty("display_name")?.takeIf { it.isNotBlank() } ?: name,
            delimiter = item.findJsonStringProperty("delimiter").orEmpty(),
            notify = item.findJsonBooleanProperty("notify") ?: false,
            notifyLive = item.findJsonBooleanProperty("notify_live") ?: false,
        )
    }
}

data class FolderDeleteResult(
    val removed: Set<String>,
    val warning: String?,
)

internal fun decodeFolderDeleteResponse(responseJson: String): FolderDeleteResult {
    val removed =
        responseJson
            .findJsonArrayProperty("removed")
            ?.jsonArrayElements()
            ?.mapNotNull { item ->
                item
                    .takeIf { it.startsWith('"') }
                    ?.readResponseString(0)
                    ?.value
                    ?.takeIf { it.isNotBlank() }
            }?.toSet()
            .orEmpty()
    return FolderDeleteResult(
        removed = removed,
        warning = responseJson.findJsonStringProperty("warning")?.takeIf { it.isNotBlank() },
    )
}

internal fun decodeFolderRenameResponse(responseJson: String): Map<String, String> =
    responseJson
        .findJsonArrayProperty("renamed")
        ?.jsonArrayElements()
        ?.mapNotNull { pair ->
            val names =
                pair.jsonArrayElements().mapNotNull { item ->
                    item.takeIf { it.startsWith('"') }?.readResponseString(0)?.value
                }
            val from = names.getOrNull(0)?.takeIf { it.isNotBlank() } ?: return@mapNotNull null
            val to = names.getOrNull(1)?.takeIf { it.isNotBlank() } ?: return@mapNotNull null
            from to to
        }?.toMap()
        .orEmpty()

internal fun parseMeronFolder(item: String): MeronFolderSummary? {
    val folder = item.fields()
    val id = folder.stringField("id")?.takeIf { it.isNotBlank() } ?: return null
    return MeronFolderSummary(
        id = id,
        name = folder.stringField("name").orEmpty(),
        unread = folder.int("unread") ?: 0,
        count = folder.int("count") ?: 0,
        createdAt = folder.number("created_at")?.toLong() ?: 0,
        notify = folder.bool("notify") ?: false,
        builtin = folder.stringField("builtin")?.takeIf { it.isNotBlank() },
    )
}

internal fun decodeMeronFolderList(responseJson: String): List<MeronFolderSummary>? {
    val fields = responseJson.responseFields()
    val raw = fields["folders"]?.takeIf { it.startsWith('[') } ?: return null
    return raw.jsonArrayElements().mapNotNull(::parseMeronFolder)
}

internal fun decodeMeronUnarchived(responseJson: String): List<String> =
    responseJson.responseFields().list("unarchived").mapNotNull { item ->
        item.fields().stringField("thread_id")?.takeIf { it.isNotBlank() }
    }

/** One conversation a move into a Meron folder changed; [previousFolderId]
 *  null means it was in no Meron folder before. */
data class MeronFolderMoveChange(
    val threadId: String,
    val accountId: String,
    val sourceFolder: String,
    val previousFolderId: String?,
)

data class MeronFolderMoveResult(
    val moved: Int,
    /** Thread ids the core refused, with its reason (`not_allowed`, `not_found`, `invalid`). */
    val skipped: List<Pair<String, String>>,
    val changes: List<MeronFolderMoveChange>,
    val folders: List<MeronFolderSummary>?,
)

internal fun decodeMeronFolderMoveResponse(responseJson: String): MeronFolderMoveResult {
    val fields = responseJson.responseFields()
    return MeronFolderMoveResult(
        moved = fields.int("moved") ?: 0,
        skipped =
            fields.list("skipped").map { item ->
                val skip = item.fields()
                skip.stringField("thread_id").orEmpty() to skip.stringField("reason").orEmpty()
            },
        changes =
            fields.list("changes").mapNotNull { item ->
                val change = item.fields()
                MeronFolderMoveChange(
                    threadId = change.stringField("thread_id")?.takeIf { it.isNotBlank() } ?: return@mapNotNull null,
                    accountId = change.stringField("account_id").orEmpty(),
                    sourceFolder = change.stringField("source_folder").orEmpty(),
                    previousFolderId = change.stringField("previous_folder_id")?.takeIf { it.isNotBlank() },
                )
            },
        folders = parseMeronFolderList(responseJson),
    )
}
