// Mail submission and draft storage through the shared command API.
// The composer serializes recipients, bodies and attachments into a core request.
// These cases also cover identifying the special folders used during draft handling.
package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class DraftCommandsTest {
    @Test
    fun sendMailMatchesDesktopComposerPayloadShape() {
        val request =
            sendMailRequest(
                id = 6,
                params =
                    SendMailParams(
                        accountId = "acc1",
                        from = "me@example.com",
                        to = "you@example.com",
                        cc = "cc@example.com",
                        subject = "Hi",
                        body = "Plain",
                        html = "<p>Plain</p>",
                        inReplyTo = "<old@example.com>",
                        references = "<root@example.com>",
                        attachments =
                            listOf(
                                MobileAttachmentInput(
                                    filename = "note.txt",
                                    mime = "text/plain",
                                    data = "SGk=",
                                ),
                            ),
                    ),
            )

        assertEquals(
            """{"id":6,"method":"mail.send","params":{"account_id":"acc1","from":"me@example.com","to":"you@example.com","cc":"cc@example.com","bcc":"","reply_to":"","subject":"Hi","body":"Plain","html":"<p>Plain</p>","in_reply_to":"<old@example.com>","references":"<root@example.com>","message_id":"","attachments":[{"filename":"note.txt","mime":"text/plain","data":"SGk=","inline_id":""}]}}""",
            request.toJson(),
        )
    }

    @Test
    fun draftCommandsMatchDesktopComposerPayloadShape() {
        val draft =
            ComposeDraft(
                to = "you@example.com",
                cc = "cc@example.com",
                bcc = "bcc@example.com",
                subject = "Draft",
                body = "Body",
                attachments =
                    listOf(
                        DraftAttachment(
                            id = "a1",
                            displayName = "note.txt",
                            mimeType = "text/plain",
                            sizeBytes = 2,
                            dataBase64 = "SGk=",
                        ),
                    ),
            )

        assertEquals(
            """{"id":41,"method":"mail.saveDraft","params":{"account_id":"acc1","draft_id":"draft@example.com","from":"me@example.com","to":"you@example.com","cc":"cc@example.com","bcc":"bcc@example.com","reply_to":"","subject":"Draft","body":"Body","html":"","in_reply_to":"","references":"","attachments":[{"filename":"note.txt","mime":"text/plain","data":"SGk=","inline_id":""}]}}""",
            saveDraftRequest(
                id = 41,
                params =
                    draft.toSaveDraftParams(
                        accountId = "acc1",
                        draftId = "draft@example.com",
                        from = "me@example.com",
                    ),
            ).toJson(),
        )
        assertEquals(
            """{"id":42,"method":"mail.discardDraft","params":{"account_id":"acc1","draft_id":"draft@example.com"}}""",
            discardDraftRequest(
                id = 42,
                params = DiscardDraftParams(accountId = "acc1", draftId = "draft@example.com"),
            ).toJson(),
        )
    }

    @Test
    fun draftsFolderDetectionCoversCommonServerNames() {
        assertTrue(folderIsDrafts("Drafts"))
        assertTrue(folderIsDrafts("[Gmail]/Drafts"))
        assertTrue(folderIsDrafts("INBOX.Draft"))
    }

    @Test
    fun trashFolderDetectionCoversCommonServerNames() {
        assertTrue(folderIsTrash("Trash"))
        assertTrue(folderIsTrash("[Gmail]/Trash"))
        assertTrue(folderIsTrash("INBOX.Deleted Items"))
    }
}
