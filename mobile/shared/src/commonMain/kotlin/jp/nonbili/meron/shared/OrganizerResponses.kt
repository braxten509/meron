// Calendar events and tasks returned by the mobile command API.
// Calendar entries and task lists carry their own identity and optional fields.
// These parsers supply the shared organizer views on Android and iOS.
package jp.nonbili.meron.shared

/**
 * One occurrence in a calendar range, as meron-core/src/calendar/events.rs
 * shapes it. Times are epoch seconds; an all-day item's are UTC midnights
 * with [end] exclusive. [key] is unique within one answer.
 */
data class CalendarItem(
    val id: String,
    val key: String,
    /** `event`, `invite` or `task`. */
    val source: String,
    val title: String,
    val notes: String = "",
    val location: String = "",
    val start: Long,
    val end: Long,
    val allDay: Boolean = false,
    val recurring: Boolean = false,
    val repeat: String = "",
    val account: String = "",
    val threadId: String = "",
    /** An invitation's answer: `accepted` or `tentative`. */
    val response: String = "",
    /** A task's list. */
    val listId: String = "",
    /** The event's own alerts, minutes before start; null follows the
     *  default and an empty list means none. */
    val alerts: List<Int>? = null,
    /** Whether [alerts] count all-day days rather than minutes: the series'
     *  kind, which one occurrence of an invitation can differ from. */
    val alertsAllDay: Boolean? = null,
)

internal fun decodeCalendarEventsResponse(responseJson: String): List<CalendarItem> =
    responseJson
        .findJsonArrayProperty("events")
        ?.jsonArrayElements()
        ?.mapNotNull { element ->
            // Own keys only: titles and notes are the user's (or a sender's) text.
            val entries = element.jsonObjectEntries().toMap()

            fun string(name: String): String =
                entries[name]
                    ?.takeIf { it.startsWith('"') }
                    ?.readResponseString(0)
                    ?.value
                    .orEmpty()
            val id = string("id")
            if (id.isBlank()) return@mapNotNull null
            CalendarItem(
                id = id,
                key = string("key").ifBlank { id },
                source = string("source"),
                title = string("title"),
                notes = string("notes"),
                location = string("location"),
                start = entries["start"]?.toLongOrNull() ?: 0,
                end = entries["end"]?.toLongOrNull() ?: 0,
                allDay = entries["all_day"] == "true",
                recurring = entries["recurring"] == "true",
                repeat = string("repeat"),
                account = string("account"),
                threadId = string("thread_id"),
                response = string("response"),
                listId = string("list_id"),
                alerts =
                    entries["alerts"]
                        ?.takeIf { it.startsWith('[') }
                        ?.jsonArrayElements()
                        ?.mapNotNull { it.trim().toIntOrNull() },
                alertsAllDay = entries["alerts_all_day"]?.let { it == "true" },
            )
        }.orEmpty()

/** One task list, as `tasks.lists` reports it. */
data class TaskListSummary(
    val id: String,
    val title: String,
)

/**
 * One task. `dueAt` and `completedAt` are epoch seconds with 0 for "unset",
 * matching the core's columns, so no nullable arithmetic is needed to sort or
 * compare them.
 */
data class TaskSummary(
    val id: String,
    val listId: String,
    val title: String,
    val notes: String = "",
    val dueAt: Long = 0,
    val completedAt: Long = 0,
    val done: Boolean = false,
    val account: String = "",
    val threadId: String = "",
    val messageId: String = "",
    val dueHasTime: Boolean = false,
)

/** Lists plus the id to open when the caller has no preference of its own. */
data class TaskListsResponse(
    val lists: List<TaskListSummary>,
    val defaultListId: String,
)

internal fun decodeTaskListsResponse(responseJson: String): TaskListsResponse {
    val lists =
        responseJson
            .findJsonArrayProperty("lists")
            ?.jsonArrayElements()
            ?.mapNotNull { item ->
                val id = item.findJsonStringProperty("id").orEmpty()
                if (id.isBlank()) null else TaskListSummary(id, item.findJsonStringProperty("title").orEmpty())
            }.orEmpty()
    return TaskListsResponse(lists, responseJson.findJsonStringProperty("default_list_id").orEmpty())
}

internal fun decodeTasksResponse(responseJson: String): List<TaskSummary> =
    responseJson
        .findJsonArrayProperty("tasks")
        ?.jsonArrayElements()
        ?.mapNotNull(::parseTaskObject)
        .orEmpty()

private fun parseTaskObject(item: String): TaskSummary? {
    val id = item.findJsonStringProperty("id").orEmpty()
    if (id.isBlank()) return null
    return TaskSummary(
        id = id,
        listId = item.findJsonStringProperty("list_id").orEmpty(),
        title = item.findJsonStringProperty("title").orEmpty(),
        notes = item.findJsonStringProperty("notes").orEmpty(),
        dueAt = item.findJsonLongProperty("due_at") ?: 0,
        dueHasTime = item.findJsonBooleanProperty("due_has_time") ?: false,
        completedAt = item.findJsonLongProperty("completed_at") ?: 0,
        done = item.findJsonBooleanProperty("done") ?: false,
        account = item.findJsonStringProperty("account").orEmpty(),
        threadId = item.findJsonStringProperty("thread_id").orEmpty(),
        messageId = item.findJsonStringProperty("message_id").orEmpty(),
    )
}
