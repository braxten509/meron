// Recipient and attachment selection when replying from mobile.
// The shared core provides the reply-all decision and canonical addresses.
// Cases cover fallback addresses and the headers sent for a threaded reply.
package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals

class ReplyCommandsTest {
    @Test
    fun replyMailParamsUseReplyRecipientAndThreadingHeaders() {
        val request =
            sendMailRequest(
                id = 7,
                params =
                    MessageBody(
                        id = "m1",
                        from = "Ada",
                        to = "Me <me@example.com>",
                        cc = "Project <project@example.com>",
                        subject = "Design",
                        body = "Original",
                        fromAddr = "ada@example.com",
                        replyTo = "Team <team@example.com>",
                        messageId = "old@example.com",
                        references = "<root@example.com>",
                        reply = MessageReply(to = "Team <team@example.com>", cc = "Project <project@example.com>"),
                    ).toReplyMailParams(
                        accountId = "acc1",
                        body = "Quick reply",
                    ),
            )

        assertEquals(
            """{"id":7,"method":"mail.send","params":{"account_id":"acc1","from":"","to":"Team <team@example.com>","cc":"Project <project@example.com>","bcc":"","reply_to":"","subject":"Re: Design","body":"Quick reply","html":"","in_reply_to":"<old@example.com>","references":"<root@example.com> <old@example.com>","message_id":"","attachments":[]}}""",
            request.toJson(),
        )
    }

    @Test
    fun replyMailParamsIncludeAttachments() {
        val request =
            sendMailRequest(
                id = 43,
                params =
                    MessageBody(
                        id = "m1",
                        from = "Ada",
                        to = "Me <me@example.com>",
                        subject = "Design",
                        body = "Original",
                        fromAddr = "ada@example.com",
                        messageId = "old@example.com",
                        reply = MessageReply(to = "ada@example.com"),
                    ).toReplyMailParams(
                        accountId = "acc1",
                        body = "With file",
                        attachments =
                            listOf(
                                DraftAttachment(
                                    id = "a1",
                                    displayName = "reply.txt",
                                    mimeType = "text/plain",
                                    sizeBytes = 2,
                                    dataBase64 = "SGk=",
                                ),
                            ),
                    ),
            )

        assertEquals(
            """{"id":43,"method":"mail.send","params":{"account_id":"acc1","from":"","to":"ada@example.com","cc":"","bcc":"","reply_to":"","subject":"Re: Design","body":"With file","html":"","in_reply_to":"<old@example.com>","references":"<old@example.com>","message_id":"","attachments":[{"filename":"reply.txt","mime":"text/plain","data":"SGk=","inline_id":""}]}}""",
            request.toJson(),
        )
    }

    @Test
    fun replyMailParamsUseTheRecipientsTheCoreDecided() {
        val request =
            sendMailRequest(
                id = 8,
                params =
                    MessageBody(
                        id = "m2",
                        from = "Me",
                        to = "Ada <ada@example.com>, Alias <alias@example.com>",
                        cc = "Me <me@example.com>, Project <project@example.com>, Ada <ada@example.com>",
                        subject = "Re: Design",
                        body = "Original",
                        fromAddr = "me@example.com",
                        messageId = "<old@example.com>",
                        // Our own message: the core answers a follow-up to its
                        // recipients, with our addresses already filtered out.
                        reply = MessageReply(to = "Ada <ada@example.com>", cc = "Project <project@example.com>"),
                    ).toReplyMailParams(
                        accountId = "acc1",
                        body = "Follow-up",
                        from = "alias@example.com",
                    ),
            )

        assertEquals(
            """{"id":8,"method":"mail.send","params":{"account_id":"acc1","from":"alias@example.com","to":"Ada <ada@example.com>","cc":"Project <project@example.com>","bcc":"","reply_to":"","subject":"Re: Design","body":"Follow-up","html":"","in_reply_to":"<old@example.com>","references":"<old@example.com>","message_id":"","attachments":[]}}""",
            request.toJson(),
        )
    }

    // The rule itself lives in the core (meron-core/src/reply.rs) and is tested
    // there; mobile only has to use the form it was sent.

    @Test
    fun replyRecipientsComeFromTheCoreDecision() {
        val incoming =
            MessageBody(
                id = "m1",
                from = "Ada",
                fromAddr = "ada@example.com",
                to = "Me <me@example.com>, Alice <alice@example.com>",
                cc = "Project <project@example.com>",
                subject = "Design",
                body = "Original",
                reply =
                    MessageReply(
                        to = "Ada <ada@example.com>",
                        cc = "Project <project@example.com>",
                        allTo = "Ada <ada@example.com>",
                        allCc = "Alice <alice@example.com>, Project <project@example.com>",
                        allAddsRecipients = true,
                    ),
            )

        val plain = buildReplyRecipients(incoming)
        assertEquals("Ada <ada@example.com>", plain.to)
        assertEquals("Project <project@example.com>", plain.cc)

        val all = buildReplyRecipients(incoming, replyAll = true)
        assertEquals("Ada <ada@example.com>", all.to)
        assertEquals("Alice <alice@example.com>, Project <project@example.com>", all.cc)
    }

    @Test
    fun aMessageWithoutTheCoreFieldStillRepliesToItsSender() {
        val bubble =
            MessageBody(
                id = "local",
                from = "Ada",
                fromAddr = "ada@example.com",
                to = "Me <me@example.com>",
                subject = "Design",
                body = "Original",
            )

        assertEquals("ada@example.com", buildReplyRecipients(bubble, replyAll = true).to)
        assertEquals("", buildReplyRecipients(bubble).cc)
    }

    @Test
    fun replyAllIsOfferedOnlyWhenTheCoreSaysItReachesSomeoneNew() {
        fun message(reply: MessageReply?) =
            MessageBody(
                id = "m1",
                from = "Ada",
                fromAddr = "ada@example.com",
                to = "Me <me@example.com>",
                subject = "Design",
                body = "Original",
                reply = reply,
            )

        assertEquals(true, replyAllAddsRecipients(message(MessageReply(allAddsRecipients = true))))
        assertEquals(false, replyAllAddsRecipients(message(MessageReply(allAddsRecipients = false))))
        assertEquals(false, replyAllAddsRecipients(message(null)))
    }

    @Test
    fun addressListsSplitOnRealSeparatorsOnly() {
        val escaped = """"He said \"hi\", <ok>" <a@x.com>"""
        assertEquals(listOf(escaped, "b@y.com"), splitAddressList("$escaped, b@y.com"))
        val backslash = """"Name\\" <a@x.com>"""
        assertEquals(listOf(backslash, "b@y.com"), splitAddressList("$backslash, b@y.com"))

        // A comma inside a quoted name or angle brackets does not separate
        // entries; an apostrophe is an ordinary character in a name.
        assertEquals(
            listOf("\"Me, Myself\" <me@x.com>", "Alice <a@y.com>"),
            splitAddressList("\"Me, Myself\" <me@x.com>, Alice <a@y.com>"),
        )
        assertEquals(
            listOf("Me <me@x.com>", "O'Connor <other@x.com>", "Alice <a@y.com>"),
            splitAddressList("Me <me@x.com>, O'Connor <other@x.com>, Alice <a@y.com>"),
        )
    }
}
