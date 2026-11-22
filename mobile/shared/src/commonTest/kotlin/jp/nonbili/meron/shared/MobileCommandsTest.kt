// Mailbox commands shared by Android and iOS.
// These tests cover thread access, sender images, undo and contact requests.
// The recording core captures method names and JSON before responses reach the client.
package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals

class MobileCommandsTest {
    @Test
    fun senderImagesUseSharedCoreResolution() {
        val core = RecordingCommandCore("""{"src":"data:image/png;base64,aGVsbG8=","kind":"favicon"}""")
        val result = runCommandFixture { MobileMailCommandClient(core).resolveSenderImage("em@em1.cloudflare.com") }
        assertEquals("avatar.resolve", core.lastCommand)
        assertEquals("""{"email":"em@em1.cloudflare.com","size":96}""", core.lastPayloadJson)
        assertEquals("data:image/png;base64,aGVsbG8=", parseSenderImageResponse(result))
        assertEquals("", parseSenderImageResponse("""{"src":"","kind":""}"""))
    }

    @Test
    fun starredAndIdentityCommandsCarryCoreOwnedQueryAndLifecycle() {
        val core = RecordingCommandCore("{}")
        val client = MobileMailCommandClient(core)
        runCommandFixture {
            client.listStarredItems(StarredItemsParams(query = "design", filter = "unread", limit = 25, beforeCursor = "starred:opaque"))
        }
        assertEquals(MobileCommand.StarredItems, core.lastCommand)
        assertEquals(
            """{"query":"design","filter":"unread","attachments":false,"limit":25,"before_cursor":"starred:opaque"}""",
            core.lastPayloadJson,
        )

        runCommandFixture { client.allocateIdentity(AllocateIdentityParams("me@example.com", draft = true)) }
        assertEquals(MobileCommand.AllocateIdentity, core.lastCommand)
        assertEquals("""{"account_id":"me@example.com","draft":true}""", core.lastPayloadJson)
    }

    @Test
    fun contactsDirectoryAndHistoryUseSharedCommands() {
        assertEquals(
            """{"account":"","query":"","limit":100,"sort":"name","directory":true}""",
            ContactSuggestParams(limit = 100, sort = "name", directory = true).toJson(),
        )
        assertEquals(
            """{"account":"","query":"ali","limit":8,"sort":"recent","directory":true}""",
            ContactSuggestParams(query = "ali", sort = "recent", directory = true).toJson(),
        )
        assertEquals(
            """{"id":28,"method":"mail.contactThreads","params":{"addr":"alice@example.com","limit":50}}""",
            contactThreadsRequest(id = 28, params = ContactThreadsParams(addr = "alice@example.com")).toJson(),
        )
        val core = RecordingCommandCore("""{"threads":[]}""")
        val client = MobileMailCommandClient(core)
        runCommandFixture { client.contactThreads(ContactThreadsParams("alice@example.com", "[200,\"account\\nkey\"]", 20)) }
        assertEquals(MobileCommand.ContactThreads, core.lastCommand)
        assertEquals(
            """{"addr":"alice@example.com","before_cursor":"[200,\"account\\nkey\"]","limit":20}""",
            core.lastPayloadJson,
        )
    }

    @Test
    fun undoHandsTheRestoreBlobBackVerbatim() {
        val deleteResponse =
            """{"id":1,"result":{"ok":true,"restore":{"tasks":[{"id":"task-1","list_id":"list-1","title":"Reply"}]}}}"""
        val restore = parseTaskRestorePayload(deleteResponse)
        assertEquals("""{"tasks":[{"id":"task-1","list_id":"list-1","title":"Reply"}]}""", restore)

        val core = RecordingCommandCore("""{"ok":true}""")
        val client = MobileMailCommandClient(core)
        runCommandFixture { client.restoreTasks(TaskRestoreParams(restore!!)) }
        assertEquals(MobileCommand.TaskRestore, core.lastCommand)
        assertEquals("""{"restore":$restore}""", core.lastPayloadJson)
    }

    /** A delete with nothing to put back must not offer an Undo that no-ops. */

    @Test
    fun aDeleteWithNothingToRestoreReportsNoPayload() {
        assertEquals(null, parseTaskRestorePayload("""{"id":1,"result":{"ok":true}}"""))
        assertEquals(null, parseTaskRestorePayload("""{"id":1,"result":{"ok":true,"restore":null}}"""))
    }

    /** A null field is "leave it alone", so it must not reach the core at all. */

    @Test
    fun clientMethodsUseSharedCoreCommandNames() {
        val core = RecordingCommandCore("""{"ok":true}""")
        val client = MobileMailCommandClient(core)

        runCommandFixture { client.listFolders(FolderListParams(accountId = "acc1")) }
        assertEquals(MobileCommand.FolderList, core.lastCommand)
        assertEquals("""{"account_id":"acc1"}""", core.lastPayloadJson)

        runCommandFixture { client.createFolder(FolderCreateParams(accountId = "acc1", name = "Work")) }
        assertEquals(MobileCommand.FolderCreate, core.lastCommand)
        assertEquals("""{"account_id":"acc1","name":"Work"}""", core.lastPayloadJson)

        runCommandFixture { client.deleteFolder(FolderDeleteParams(accountId = "acc1", folderId = "Work")) }
        assertEquals(MobileCommand.FolderDelete, core.lastCommand)
        assertEquals("""{"account_id":"acc1","folder_id":"Work"}""", core.lastPayloadJson)

        runCommandFixture { client.renameFolder(FolderRenameParams(accountId = "acc1", folderId = "Work", name = "Jobs")) }
        assertEquals(MobileCommand.FolderRename, core.lastCommand)
        assertEquals("""{"account_id":"acc1","folder_id":"Work","name":"Jobs"}""", core.lastPayloadJson)

        runCommandFixture { client.setFolderNotify(FolderSetNotifyParams(accountId = "acc1", folderId = "Work", enabled = true)) }
        assertEquals(MobileCommand.FolderSetNotify, core.lastCommand)
        assertEquals("""{"account_id":"acc1","folder_id":"Work","enabled":true}""", core.lastPayloadJson)

        runCommandFixture { client.listThreads(ThreadListParams(accountId = "acc1")) }
        assertEquals(MobileCommand.ThreadList, core.lastCommand)
        assertEquals("""{"account_id":"acc1","folder_id":"inbox","query":"","filter":"all","attachments":false,"refresh":false}""", core.lastPayloadJson)

        runCommandFixture { client.listStarredItems() }
        assertEquals(MobileCommand.StarredItems, core.lastCommand)
        assertEquals("""{"query":"","filter":"all","attachments":false,"limit":50}""", core.lastPayloadJson)

        runCommandFixture { client.readThread(ThreadReadParams(threadId = "thread1")) }
        assertEquals(MobileCommand.ThreadRead, core.lastCommand)
        assertEquals("""{"thread_id":"thread1","limit":10}""", core.lastPayloadJson)

        runCommandFixture { client.sync(SyncMailParams(accountId = "acc1")) }
        assertEquals(MobileCommand.Sync, core.lastCommand)
        assertEquals("""{"account_id":"acc1","folder_id":"inbox","limit":50,"folders":true,"defer_tail":false}""", core.lastPayloadJson)

        runCommandFixture { client.sync(SyncMailParams(accountId = "acc1", deferTail = true)) }
        assertEquals(MobileCommand.Sync, core.lastCommand)
        assertEquals("""{"account_id":"acc1","folder_id":"inbox","limit":50,"folders":true,"defer_tail":true}""", core.lastPayloadJson)

        runCommandFixture {
            client.send(
                SendMailParams(
                    accountId = "acc1",
                    to = "you@example.com",
                    subject = "Hi",
                    body = "Hello",
                ),
            )
        }
        assertEquals(MobileCommand.Send, core.lastCommand)

        runCommandFixture { client.archive(ThreadActionParams(threadId = "thread1")) }
        assertEquals(MobileCommand.Archive, core.lastCommand)
        assertEquals("""{"thread_id":"thread1"}""", core.lastPayloadJson)

        runCommandFixture { client.delete(ThreadActionParams(threadId = "thread1")) }
        assertEquals(MobileCommand.Delete, core.lastCommand)
        assertEquals("""{"thread_id":"thread1"}""", core.lastPayloadJson)

        runCommandFixture { client.move(MoveThreadParams(threadId = "thread1", targetFolderId = "Work")) }
        assertEquals(MobileCommand.Move, core.lastCommand)
        assertEquals("""{"thread_id":"thread1","target_folder_id":"Work"}""", core.lastPayloadJson)

        runCommandFixture { client.move(MoveThreadParams(threadId = "thread1", targetFolderId = "INBOX", targetAccountId = "acc2")) }
        assertEquals(MobileCommand.Move, core.lastCommand)
        assertEquals("""{"thread_id":"thread1","target_folder_id":"INBOX","target_account_id":"acc2"}""", core.lastPayloadJson)

        runCommandFixture { client.copy(CopyThreadParams(threadId = "thread1", targetAccountId = "acc2", targetFolderId = "Archive")) }
        assertEquals(MobileCommand.Copy, core.lastCommand)
        assertEquals("""{"thread_id":"thread1","target_account_id":"acc2","target_folder_id":"Archive"}""", core.lastPayloadJson)

        runCommandFixture { client.markRead(MarkReadParams(threadId = "thread1", seen = false)) }
        assertEquals(MobileCommand.MarkRead, core.lastCommand)
        assertEquals("""{"thread_id":"thread1","seen":false}""", core.lastPayloadJson)

        runCommandFixture { client.markAllRead(MarkAllReadParams(accountId = "acc1", folderId = "INBOX")) }
        assertEquals(MobileCommand.MarkAllRead, core.lastCommand)
        assertEquals("""{"account_id":"acc1","folder_id":"INBOX"}""", core.lastPayloadJson)

        runCommandFixture { client.emptyFolder(EmptyFolderParams(accountId = "acc1", folderId = "Trash")) }
        assertEquals(MobileCommand.EmptyFolder, core.lastCommand)
        assertEquals("""{"account_id":"acc1","folder_id":"Trash"}""", core.lastPayloadJson)

        runCommandFixture { client.readAttachment(AttachmentReadParams(key = "acc/INBOX/1/note.txt")) }
        assertEquals(MobileCommand.AttachmentRead, core.lastCommand)
        assertEquals("""{"key":"acc/INBOX/1/note.txt"}""", core.lastPayloadJson)

        runCommandFixture { client.markStarred(MarkStarredParams(threadId = "thread1", starred = true)) }
        assertEquals(MobileCommand.MarkStarred, core.lastCommand)
        assertEquals("""{"thread_id":"thread1","starred":true}""", core.lastPayloadJson)
    }

    @Test
    fun threadListPreservesFrontendWireFieldNames() {
        val request =
            threadListRequest(
                id = 4,
                params =
                    ThreadListParams(
                        accountId = "acc1",
                        folderId = "inbox",
                        query = "design",
                        filter = "unread",
                        attachments = true,
                        beforeCursor = "1700000000:44",
                        refresh = true,
                    ),
            )

        assertEquals(
            """{"id":4,"method":"mail.threadList","params":{"account_id":"acc1","folder_id":"inbox","query":"design","filter":"unread","attachments":true,"before_cursor":"1700000000:44","refresh":true}}""",
            request.toJson(),
        )
    }

    // Omitted unless the mailbox asks for a specific depth, so the core keeps
    // applying its own page default for every other caller.

    @Test
    fun threadListSendsLimitOnlyWhenSet() {
        assertEquals(
            """{"account_id":"acc1","folder_id":"inbox","query":"","filter":"all","attachments":false,"refresh":false,"limit":150}""",
            ThreadListParams(accountId = "acc1", limit = 150).toJson(),
        )
        assertEquals(
            """{"account_id":"acc1","folder_id":"inbox","query":"","filter":"all","attachments":false,"refresh":false}""",
            ThreadListParams(accountId = "acc1").toJson(),
        )
    }

    @Test
    fun folderCreatePreservesFrontendWireFieldNames() {
        assertEquals(
            """{"id":6,"method":"mail.folderCreate","params":{"account_id":"acc1","name":"Work"}}""",
            folderCreateRequest(id = 6, params = FolderCreateParams(accountId = "acc1", name = "Work")).toJson(),
        )
    }

    @Test
    fun folderDeletePreservesFrontendWireFieldNames() {
        assertEquals(
            """{"id":7,"method":"mail.folderDelete","params":{"account_id":"acc1","folder_id":"Work"}}""",
            folderDeleteRequest(id = 7, params = FolderDeleteParams(accountId = "acc1", folderId = "Work")).toJson(),
        )
    }

    @Test
    fun moveThreadPreservesFrontendWireFieldNames() {
        assertEquals(
            """{"id":7,"method":"mail.move","params":{"thread_id":"acc#imap#inbox#thread","target_folder_id":"Work"}}""",
            moveThreadRequest(
                id = 7,
                params = MoveThreadParams(threadId = "acc#imap#inbox#thread", targetFolderId = "Work"),
            ).toJson(),
        )
    }

    @Test
    fun copyThreadPreservesFrontendWireFieldNames() {
        assertEquals(
            """{"id":8,"method":"mail.copy","params":{"thread_id":"acc#imap#inbox#thread","target_account_id":"acc2","target_folder_id":"Archive"}}""",
            copyThreadRequest(
                id = 8,
                params =
                    CopyThreadParams(
                        threadId = "acc#imap#inbox#thread",
                        targetAccountId = "acc2",
                        targetFolderId = "Archive",
                    ),
            ).toJson(),
        )
    }

    @Test
    fun threadReadUsesSharedConversationPageSizeByDefault() {
        assertEquals(
            """{"id":5,"method":"mail.threadRead","params":{"thread_id":"acc#imap#inbox#thread","limit":10}}""",
            threadReadRequest(id = 5, params = ThreadReadParams(threadId = "acc#imap#inbox#thread")).toJson(),
        )
    }

    @Test
    fun syncMailMatchesMobileCorePayloadShape() {
        val request =
            syncMailRequest(
                id = 8,
                params =
                    SyncMailParams(
                        accountId = "acc1",
                        folderId = "INBOX",
                        limit = 25,
                        folders = false,
                    ),
            )

        assertEquals(
            """{"id":8,"method":"mail.sync","params":{"account_id":"acc1","folder_id":"INBOX","limit":25,"folders":false,"defer_tail":false}}""",
            request.toJson(),
        )
    }

    @Test
    fun threadActionsUseCurrentBridgePayloads() {
        assertEquals(
            """{"thread_id":"t1"}""",
            ThreadActionParams(threadId = "t1").archiveJson(),
        )
        assertEquals(
            """{"thread_id":"t1","folder":"Trash","message_ids":["m1","m2"]}""",
            ThreadActionParams(threadId = "t1", folderId = "Trash", messageIds = listOf("m1", "m2")).deleteJson(),
        )
        assertEquals(
            """{"thread_id":"t1","seen":false,"message_ids":["m1"]}""",
            MarkReadParams(threadId = "t1", seen = false, messageIds = listOf("m1")).toJson(),
        )
        assertEquals(
            """{"thread_id":"t1","seen":true,"message_ids":["acc#Sent#7"],"folder":"Sent"}""",
            MarkReadParams(threadId = "t1", messageIds = listOf("acc#Sent#7"), folderId = "Sent").toJson(),
        )
        assertEquals(
            """{"account_id":"acc1","folder_id":"INBOX"}""",
            MarkAllReadParams(accountId = "acc1", folderId = "INBOX").toJson(),
        )
        assertEquals(
            """{"thread_id":"t1","starred":true,"message_ids":["m1"]}""",
            MarkStarredParams(threadId = "t1", starred = true, messageIds = listOf("m1")).toJson(),
        )
        assertEquals(
            """{"thread_id":"t1","starred":true,"message_ids":["acc#Sent#7"],"folder":"Sent"}""",
            MarkStarredParams(
                threadId = "t1",
                starred = true,
                messageIds = listOf("acc#Sent#7"),
                folderId = "Sent",
            ).toJson(),
        )
        assertEquals(
            """{"id":20,"method":"mail.archive","params":{"thread_id":"t1"}}""",
            archiveThreadRequest(id = 20, ThreadActionParams(threadId = "t1")).toJson(),
        )
        assertEquals(
            """{"id":21,"method":"mail.delete","params":{"thread_id":"t1","folder":"Trash"}}""",
            deleteThreadRequest(id = 21, ThreadActionParams(threadId = "t1", folderId = "Trash")).toJson(),
        )
        assertEquals(
            """{"id":22,"method":"mail.markRead","params":{"thread_id":"t1","seen":true}}""",
            markReadRequest(id = 22, MarkReadParams(threadId = "t1", seen = true)).toJson(),
        )
        assertEquals(
            """{"id":23,"method":"mail.markAllRead","params":{"account_id":"acc1","folder_id":"INBOX"}}""",
            markAllReadRequest(id = 23, MarkAllReadParams(accountId = "acc1", folderId = "INBOX")).toJson(),
        )
        assertEquals(
            """{"id":24,"method":"mail.markStarred","params":{"thread_id":"t1","starred":false}}""",
            markStarredRequest(id = 24, MarkStarredParams(threadId = "t1", starred = false)).toJson(),
        )
        assertEquals(
            """{"id":25,"method":"mail.emptyFolder","params":{"account_id":"acc1","folder_id":"Trash"}}""",
            emptyFolderRequest(id = 25, EmptyFolderParams(accountId = "acc1", folderId = "Trash")).toJson(),
        )
    }

    @Test
    fun jsonBuilderEscapesPayloadStrings() {
        assertEquals(
            """{"account_id":"a\"b","folder_id":"line\nbreak","query":"","filter":"all","attachments":false,"refresh":false}""",
            ThreadListParams(accountId = "a\"b", folderId = "line\nbreak").toJson(),
        )
    }
}
