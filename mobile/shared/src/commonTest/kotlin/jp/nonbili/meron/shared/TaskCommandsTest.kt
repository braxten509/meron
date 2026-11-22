// Tasks associated with messages and threads on mobile.
// Command fixtures record which fields were sent for create and update operations.
// Response cases cover conversion to the summaries consumed by the interface.
package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals

class TaskCommandsTest {
    @Test
    fun taskClientMethodsUseSharedCoreCommandNames() {
        val core = RecordingCommandCore("""{"ok":true}""")
        val client = MobileMailCommandClient(core)

        runCommandFixture { client.taskLists() }
        assertEquals(MobileCommand.TaskLists, core.lastCommand)

        runCommandFixture { client.taskItems(TaskItemsParams(listId = "list-1", includeCompleted = true)) }
        assertEquals(MobileCommand.TaskItems, core.lastCommand)
        assertEquals("""{"list_id":"list-1","include_completed":true}""", core.lastPayloadJson)

        runCommandFixture {
            client.createTask(
                TaskCreateParams(
                    listId = "list-1",
                    title = "Book the flight",
                    notes = "window seat",
                    dueAt = 1_700_000_000,
                ),
            )
        }
        assertEquals(MobileCommand.TaskCreate, core.lastCommand)
        assertEquals(
            """{"list_id":"list-1","title":"Book the flight","notes":"window seat","due_at":1700000000,"account":"","thread_id":"","message_id":""}""",
            core.lastPayloadJson,
        )

        runCommandFixture { client.setTaskDone(TaskSetDoneParams(taskId = "task-1", done = true)) }
        assertEquals(MobileCommand.TaskSetDone, core.lastCommand)
        assertEquals("""{"task_id":"task-1","done":true}""", core.lastPayloadJson)

        runCommandFixture { client.reorderTasks(TaskReorderParams(listId = "list-1", taskIds = listOf("b", "a"))) }
        assertEquals(MobileCommand.TaskReorder, core.lastCommand)
        assertEquals("""{"list_id":"list-1","task_ids":["b","a"]}""", core.lastPayloadJson)

        runCommandFixture { client.clearCompletedTasks(TaskClearCompletedParams(listId = "list-1")) }
        assertEquals(MobileCommand.TaskClearCompleted, core.lastCommand)
        assertEquals("""{"list_id":"list-1"}""", core.lastPayloadJson)
    }

    @Test
    fun threadTasksAsksForTheThreadsTasks() {
        val core = RecordingCommandCore("""{"tasks":[]}""")
        val client = MobileMailCommandClient(core)
        runCommandFixture { client.threadTasks(TaskForThreadParams("acct#INBOX#t1")) }
        assertEquals(MobileCommand.TaskForThread, core.lastCommand)
        assertEquals("""{"thread_id":"acct#INBOX#t1"}""", core.lastPayloadJson)
    }

    /**
     * The restore blob travels from the delete response back to the core
     * untouched — parsing and rebuilding it would be a chance to lose a field.
     */

    @Test
    fun taskUpdateOmitsTheFieldsItIsNotChanging() {
        val core = RecordingCommandCore("""{"ok":true}""")
        val client = MobileMailCommandClient(core)

        runCommandFixture { client.updateTask(TaskUpdateParams(taskId = "task-1", title = "Pay the rent")) }
        assertEquals(MobileCommand.TaskUpdate, core.lastCommand)
        assertEquals("""{"task_id":"task-1","title":"Pay the rent"}""", core.lastPayloadJson)

        runCommandFixture { client.updateTask(TaskUpdateParams(taskId = "task-1", dueAt = 0)) }
        assertEquals("""{"task_id":"task-1","due_at":0}""", core.lastPayloadJson)
    }

    /** Adding a message to Tasks has no list to name; the core picks the default. */

    @Test
    fun taskCreatedFromAMessageCarriesItsThreadAndNoList() {
        val core = RecordingCommandCore("""{"ok":true}""")
        val client = MobileMailCommandClient(core)

        runCommandFixture {
            client.createTask(
                TaskCreateParams(
                    title = "Boiler repair",
                    account = "acct1",
                    threadId = "acct1#thread#4",
                    messageId = "<boiler@example.com>",
                ),
            )
        }
        assertEquals(
            """{"list_id":"","title":"Boiler repair","notes":"","due_at":0,"account":"acct1","thread_id":"acct1#thread#4","message_id":"<boiler@example.com>"}""",
            core.lastPayloadJson,
        )
    }

    @Test
    fun taskResponsesParseIntoTypedSummaries() {
        val listsJson =
            """{"lists":[{"id":"list-1","title":"My Tasks"},{"id":"list-2","title":"Groceries"}],"default_list_id":"list-1"}"""
        val lists = parseTaskListsResponse(listsJson)
        assertEquals(listOf("list-1", "list-2"), lists.lists.map { it.id })
        assertEquals("My Tasks", lists.lists.first().title)
        assertEquals("list-1", lists.defaultListId)

        val tasksJson =
            """{"list_id":"list-1","tasks":[{"id":"task-1","list_id":"list-1","title":"Reply","notes":"n","due_at":1700000000,"completed_at":0,"done":false,"account":"acct1","thread_id":"acct1#thread#4","message_id":"<m@example.com>"}]}"""
        val tasks = parseTasksResponse(tasksJson)
        assertEquals(1, tasks.size)
        assertEquals("Reply", tasks.first().title)
        assertEquals(1_700_000_000, tasks.first().dueAt)
        assertEquals(false, tasks.first().done)
        assertEquals("acct1#thread#4", tasks.first().threadId)

        assertEquals(2, parseClearedTaskCount("""{"ok":true,"removed":2}"""))
        assertEquals(0, parseClearedTaskCount("""{"ok":true}"""))
    }
}
