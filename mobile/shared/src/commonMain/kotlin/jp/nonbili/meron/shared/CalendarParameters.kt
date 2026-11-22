// Calendar Parameters for the mobile protocol.
// Used by the shared Kotlin client when it calls into the Rust core.
// Wire keys and request identifiers must stay compatible across platforms.
package jp.nonbili.meron.shared

/** [message] is the thread message's `id` and [threadId] its conversation's,
 * which the calendar event links back to; [response] is `accepted`,
 * `tentative` or `declined`. */
data class InviteRespondParams(
    val message: String,
    val response: String,
    val threadId: String = "",
) {
    fun toJson(): String =
        jsonObject(
            "message" to message.jsonString(),
            "thread_id" to threadId.jsonString(),
            "response" to response.jsonString(),
        )
}

/** A calendar range in epoch seconds, `[from, to)`. */
data class CalendarRangeParams(
    val from: Long,
    val to: Long,
    val includeTasks: Boolean,
) {
    fun toJson(): String =
        jsonObject(
            "from" to from.toString(),
            "to" to to.toString(),
            "include_tasks" to includeTasks.toString(),
        )
}

/**
 * An event to create, or (with [eventId]) to change. Times are epoch seconds;
 * an all-day event's are UTC midnights with [end] exclusive. [repeat] is
 * `daily`, `weekly`, `monthly`, `yearly`, or empty for once.
 */
data class CalendarEventParams(
    val eventId: String = "",
    val title: String,
    val notes: String = "",
    val location: String = "",
    val start: Long,
    val end: Long,
    val allDay: Boolean = false,
    val repeat: String = "",
    /** Editing a repeating event: the start of the occurrence the editor was
     *  opened on, so the core moves the series by the change rather than
     *  restarting it there. 0 for none. */
    val occurrenceStart: Long = 0,
    /** Minutes before start, at most two; null follows the app-wide default
     *  and an empty list means none. */
    val alerts: List<Int>? = null,
) {
    fun toJson(): String =
        jsonObject(
            "event_id" to eventId.takeIf { it.isNotEmpty() }?.jsonString(),
            "alerts" to (alerts?.joinToString(separator = ",", prefix = "[", postfix = "]") ?: "null"),
            "occurrence_start" to occurrenceStart.takeIf { it > 0 }?.toString(),
            "title" to title.jsonString(),
            "notes" to notes.jsonString(),
            "location" to location.jsonString(),
            "start" to start.toString(),
            "end" to end.toString(),
            "all_day" to allDay.toString(),
            "repeat" to repeat.jsonString(),
        )
}
