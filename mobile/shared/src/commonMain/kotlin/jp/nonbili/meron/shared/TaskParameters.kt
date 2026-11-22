// Task Parameters for the mobile protocol.
// Used by the shared Kotlin client when it calls into the Rust core.
// Wire keys and request identifiers must stay compatible across platforms.
package jp.nonbili.meron.shared

/**
 * A new task. [listId] may be blank — the core then files it under the default
 * list, which is what "add this message to Tasks" wants, since that action has
 * no list picker to offer.
 */
data class TaskCreateParams(
    val listId: String = "",
    val title: String,
    val notes: String = "",
    val dueAt: Long = 0,
    val account: String = "",
    val threadId: String = "",
    val messageId: String = "",
    val dueHasTime: Boolean = false,
) {
    fun toJson(): String =
        jsonObject(
            "list_id" to listId.jsonString(),
            "title" to title.jsonString(),
            "notes" to notes.jsonString(),
            "due_at" to dueAt.toString(),
            "due_has_time" to dueHasTime.takeIf { it }?.toString(),
            "account" to account.jsonString(),
            "thread_id" to threadId.jsonString(),
            "message_id" to messageId.jsonString(),
        )
}

/** A patch: a null field is left alone rather than cleared. */
data class TaskUpdateParams(
    val taskId: String,
    val title: String? = null,
    val notes: String? = null,
    val dueAt: Long? = null,
    val listId: String? = null,
    val dueHasTime: Boolean? = null,
) {
    fun toJson(): String =
        jsonObject(
            "task_id" to taskId.jsonString(),
            "title" to title?.jsonString(),
            "notes" to notes?.jsonString(),
            "due_at" to dueAt?.toString(),
            "due_has_time" to dueHasTime?.toString(),
            "list_id" to listId?.jsonString(),
        )
}


data class TaskSetDoneParams(
    val taskId: String,
    val done: Boolean,
) {
    fun toJson(): String =
        jsonObject(
            "task_id" to taskId.jsonString(),
            "done" to done.toString(),
        )
}

data class TaskDeleteParams(
    val taskId: String,
) {
    fun toJson(): String = jsonObject("task_id" to taskId.jsonString())
}

data class TaskReorderParams(
    val listId: String,
    val taskIds: List<String>,
) {
    fun toJson(): String =
        jsonObject(
            "list_id" to listId.jsonString(),
            "task_ids" to taskIds.jsonStringArray(),
        )
}

/**
 * Undo a delete, by handing back the `restore` blob the delete returned.
 *
 * [restore] is raw JSON, not a string value: the client never looks inside it,
 * so it travels from the delete response to here untouched rather than being
 * parsed into fields and rebuilt.
 */
data class TaskRestoreParams(
    val restore: String,
) {
    fun toJson(): String = jsonObject("restore" to restore)
}

data class TaskClearCompletedParams(
    val listId: String,
) {
    fun toJson(): String = jsonObject("list_id" to listId.jsonString())
}
