// Task edits distinguish midnight from a date
// without a time. Cases cover setting, clearing and
// preserving an existing time when serializing an
// update.

package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class TaskDueTimeTest {
    @Test
    fun explicitMidnightRemainsDistinctFromDateOnly() {
        val timed = parseTasksResponse("""{"result":{"tasks":[{"id":"a","list_id":"l","title":"Midnight","due_at":1792886400,"due_has_time":true}]}}""").single()
        assertTrue(timed.dueHasTime)
        assertEquals(1792886400L, timed.dueAt)
        val dateOnly = parseTasksResponse("""{"result":{"tasks":[{"id":"a","list_id":"l","title":"Day","due_at":1792886400}]}}""").single()
        assertFalse(dateOnly.dueHasTime)
    }

    @Test
    fun timeCanBeSetClearedAndLeftUnchanged() {
        assertTrue(TaskUpdateParams("a", dueAt = 1792886400, dueHasTime = true).toJson().contains("\"due_has_time\":true"))
        assertTrue(TaskUpdateParams("a", dueHasTime = false).toJson().contains("\"due_has_time\":false"))
        assertFalse(TaskUpdateParams("a", title = "Rename").toJson().contains("due_has_time"))
    }
}
