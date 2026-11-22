// Folder Parameters for the mobile protocol.
// Used by the shared Kotlin client when it calls into the Rust core.
// Wire keys and request identifiers must stay compatible across platforms.
package jp.nonbili.meron.shared

data class FolderListParams(
    val accountId: String,
) {
    fun toJson(): String = jsonObject("account_id" to accountId.jsonString())
}

data class FolderCreateParams(
    val accountId: String,
    val name: String,
) {
    fun toJson(): String =
        jsonObject(
            "account_id" to accountId.jsonString(),
            "name" to name.jsonString(),
        )
}

/**
 * Deletes a folder on the server with everything in it, subfolders included; the
 * core rejects special-use folders anywhere in that subtree.
 */
data class FolderDeleteParams(
    val accountId: String,
    val folderId: String,
) {
    fun toJson(): String =
        jsonObject(
            "account_id" to accountId.jsonString(),
            "folder_id" to folderId.jsonString(),
        )
}

/**
 * Renames a folder on the server, subfolders included. [name] is the folder's
 * new full path; the core rejects special-use folders like a delete.
 */
data class FolderRenameParams(
    val accountId: String,
    val folderId: String,
    val name: String,
) {
    fun toJson(): String =
        jsonObject(
            "account_id" to accountId.jsonString(),
            "folder_id" to folderId.jsonString(),
            "name" to name.jsonString(),
        )
}

data class FolderSetNotifyParams(
    val accountId: String,
    val folderId: String,
    val enabled: Boolean,
) {
    fun toJson(): String =
        jsonObject(
            "account_id" to accountId.jsonString(),
            "folder_id" to folderId.jsonString(),
            "enabled" to enabled.toString(),
        )
}


data class MeronFolderThreadsParams(
    val folderId: String,
    val query: String = "",
    /** `all`, `unread` or `starred`. */
    val filter: String = "all",
    val attachments: Boolean = false,
    val limit: Int = 50,
    val beforeCursor: String? = null,
) {
    fun toJson(): String =
        jsonObject(
            "folder_id" to folderId.jsonString(),
            "query" to query.jsonString(),
            "filter" to filter.jsonString(),
            "attachments" to attachments.toString(),
            "limit" to limit.toString(),
            "before_cursor" to beforeCursor?.jsonString(),
        )
}
