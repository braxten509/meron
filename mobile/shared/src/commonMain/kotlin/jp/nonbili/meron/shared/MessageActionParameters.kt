// Message Action Parameters for the mobile protocol.
// Used by the shared Kotlin client when it calls into the Rust core.
// Wire keys and request identifiers must stay compatible across platforms.
package jp.nonbili.meron.shared

data class ThreadActionParams(
    val threadId: String,
    val folderId: String? = null,
    val messageIds: List<String> = emptyList(),
) {
    fun archiveJson(): String = jsonObject("thread_id" to threadId.jsonString())

    fun deleteJson(): String =
        jsonObject(
            "thread_id" to threadId.jsonString(),
            "folder" to folderId?.jsonString(),
            "message_ids" to messageIds.takeUnless { it.isEmpty() }?.jsonStringArray(),
        )
}

data class MoveThreadParams(
    val threadId: String,
    val targetFolderId: String,
    val targetAccountId: String? = null,
    /** Move only these messages (ids ending in `#uid`); empty moves the whole thread. */
    val messageIds: List<String> = emptyList(),
    /** An Undo: the core refuses it without [messageIds] rather than guess. */
    val undo: Boolean = false,
    /** Undo of mail that was shown in the Inbox from its own folder: show it there again. */
    val showInInbox: Boolean = false,
    /** Undo of an action that took the card out of this Meron folder: put it back. */
    val meronFolderId: String? = null,
    val meronSource: String? = null,
) {
    fun toJson(): String =
        jsonObject(
            "thread_id" to threadId.jsonString(),
            "target_folder_id" to targetFolderId.jsonString(),
            "target_account_id" to targetAccountId?.jsonString(),
            "message_ids" to messageIds.takeUnless { it.isEmpty() }?.jsonStringArray(),
            "undo" to if (undo) "true" else null,
            "show_in_inbox" to if (showInInbox) "true" else null,
            "meron_folder_id" to meronFolderId?.takeIf { it.isNotBlank() }?.jsonString(),
            "meron_source" to meronSource?.takeIf { it.isNotBlank() }?.jsonString(),
        )
}

data class CopyThreadParams(
    val threadId: String,
    val targetAccountId: String,
    val targetFolderId: String,
) {
    fun toJson(): String =
        jsonObject(
            "thread_id" to threadId.jsonString(),
            "target_account_id" to targetAccountId.jsonString(),
            "target_folder_id" to targetFolderId.jsonString(),
        )
}

data class MarkReadParams(
    val threadId: String,
    val seen: Boolean = true,
    val messageIds: List<String> = emptyList(),
    val folderId: String? = null,
) {
    fun toJson(): String =
        jsonObject(
            "thread_id" to threadId.jsonString(),
            "seen" to seen.toString(),
            "message_ids" to messageIds.takeUnless { it.isEmpty() }?.jsonStringArray(),
            "folder" to folderId?.jsonString(),
        )
}

data class MarkAllReadParams(
    val accountId: String,
    val folderId: String = "inbox",
) {
    fun toJson(): String =
        jsonObject(
            "account_id" to accountId.jsonString(),
            "folder_id" to folderId.jsonString(),
        )
}

/** Permanently deletes a Trash or Junk folder; the core rejects any other role. */
data class EmptyFolderParams(
    val accountId: String,
    val folderId: String,
) {
    fun toJson(): String =
        jsonObject(
            "account_id" to accountId.jsonString(),
            "folder_id" to folderId.jsonString(),
        )
}

data class MarkStarredParams(
    val threadId: String,
    val starred: Boolean,
    val messageIds: List<String> = emptyList(),
    val folderId: String? = null,
) {
    fun toJson(): String =
        jsonObject(
            "thread_id" to threadId.jsonString(),
            "starred" to starred.toString(),
            "message_ids" to messageIds.takeUnless { it.isEmpty() }?.jsonStringArray(),
            "folder" to folderId?.jsonString(),
        )
}

data class SetTagsParams(
    val threadId: String,
    val add: List<String> = emptyList(),
    val remove: List<String> = emptyList(),
    val messageIds: List<String> = emptyList(),
) {
    fun toJson(): String =
        jsonObject(
            "thread_id" to threadId.jsonString(),
            "add" to add.jsonStringArray(),
            "remove" to remove.jsonStringArray(),
            "message_ids" to messageIds.takeUnless { it.isEmpty() }?.jsonStringArray(),
        )
}
