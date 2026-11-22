// A locally sent reply stays visible while its
// stored copy is on the way. The cases exercise the
// message identity used to replace the optimistic
// row.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.SendStatus
import kotlin.test.Test
import kotlin.test.assertEquals

// Reconciling the optimistic sent bubble with the conversation the core hands
// back. The Message-ID we generated is the primary match, but a server that
// rewrites it when storing its Sent copy (Proton Bridge stamps its own
// `@protonmail.internalid`) never produces one, so the envelope has to settle it.
class LocalSendMergeTest {
    @Test
    fun keepsTheOptimisticReplyUntilTheStoredCopyArrives() {
        val parent = message(id = "m1", messageId = "root@example.com", dateEpochSeconds = 1000)
        val optimistic = localSend(dateEpochSeconds = 2000)

        assertEquals(
            listOf("m1", "local-send-1"),
            mergeLocalSendMessages(listOf(parent, optimistic), listOf(parent)).map { it.id },
        )
    }

    @Test
    fun dropsTheOptimisticReplyOnAMessageIdMatch() {
        val optimistic = localSend(dateEpochSeconds = 2000)
        val stored = storedCopy(id = "sent-1", messageId = "<REPLY@example.com>", dateEpochSeconds = 2000)

        assertEquals(listOf("sent-1"), mergeLocalSendMessages(listOf(optimistic), listOf(stored)).map { it.id })
    }

    @Test
    fun dropsTheOptimisticReplyWhenTheServerRewroteTheMessageId() {
        val optimistic = localSend(dateEpochSeconds = 2000)
        val stored = storedCopy(id = "sent-1", messageId = "abc@protonmail.internalid", dateEpochSeconds = 2004)

        assertEquals(listOf("sent-1"), mergeLocalSendMessages(listOf(optimistic), listOf(stored)).map { it.id })
    }

    @Test
    fun collapsesTwoIdenticalRepliesOntoTwoStoredCopies() {
        val first = localSend(id = "local-send-1", messageId = "first@example.com", dateEpochSeconds = 2000)
        val second = localSend(id = "local-send-2", messageId = "second@example.com", dateEpochSeconds = 2010)
        val stored = storedCopy(id = "sent-1", messageId = "one@protonmail.internalid", dateEpochSeconds = 2001)

        // One copy back so far: the second reply keeps its bubble.
        assertEquals(
            listOf("sent-1", "local-send-2"),
            mergeLocalSendMessages(listOf(first, second), listOf(stored)).map { it.id },
        )
        // Both back: nothing left over.
        assertEquals(
            listOf("sent-1", "sent-2"),
            mergeLocalSendMessages(
                listOf(first, second),
                listOf(stored, storedCopy(id = "sent-2", messageId = "two@protonmail.internalid", dateEpochSeconds = 2011)),
            ).map { it.id },
        )
    }

    @Test
    fun pairsAStoredCopyWithTheReplyItBelongsToNotTheFirstBubble() {
        // Two replies in one conversation share sender, subject, recipients and
        // a timestamp seconds apart — the envelope alone cannot tell them
        // apart. The second one's copy comes back first.
        val first = localSend(id = "local-send-1", messageId = "first@example.com", dateEpochSeconds = 2000).copy(body = "First reply")
        val second =
            localSend(id = "local-send-2", messageId = "second@example.com", dateEpochSeconds = 2010).copy(body = "Second reply")
        val secondCopy =
            storedCopy(id = "sent-b", messageId = "b@protonmail.internalid", dateEpochSeconds = 2011).copy(body = "Second reply")

        assertEquals(
            listOf("local-send-1", "sent-b"),
            mergeLocalSendMessages(listOf(first, second), listOf(secondCopy)).map { it.id },
        )
    }

    @Test
    fun fallsBackToTheClosestSendTimeWhenTheServerReflowedTheBody() {
        val optimistic = localSend(dateEpochSeconds = 2000).copy(body = "A reply long enough to wrap")
        // Same words, rewrapped by the submission server.
        val stored =
            storedCopy(id = "sent-1", messageId = "abc@protonmail.internalid", dateEpochSeconds = 2003)
                .copy(body = "A reply long enough\nto wrap")

        assertEquals(listOf("sent-1"), mergeLocalSendMessages(listOf(optimistic), listOf(stored)).map { it.id })
    }

    @Test
    fun doesNotLetANewlySavedDraftClaimAnOptimisticReply() {
        // The autosaved draft of the *next* reply: outgoing, same envelope,
        // seconds apart — but a draft is not a sent copy.
        val optimistic = localSend(dateEpochSeconds = 2000)
        val draft =
            storedCopy(id = "draft-1", messageId = "next-draft@example.com", dateEpochSeconds = 2005)
                .copy(folderId = "Drafts")

        assertEquals(
            listOf("local-send-1", "draft-1"),
            mergeLocalSendMessages(listOf(optimistic), listOf(draft)).map { it.id },
        )
    }

    @Test
    fun doesNotMistakeTheMessageBeingRepliedToForTheStoredCopy() {
        // A conversation the user talks to themselves in: same sender, same
        // recipients, a minute apart, and the parent is outgoing too. It was
        // already on screen before the send, so it cannot be that send's copy.
        val parent =
            storedCopy(id = "m1", messageId = "root@example.com", dateEpochSeconds = 2000).copy(to = "me@example.com")
        val optimistic = localSend(dateEpochSeconds = 2060).copy(to = "me@example.com")

        assertEquals(
            listOf("m1", "local-send-1"),
            mergeLocalSendMessages(listOf(parent, optimistic), listOf(parent)).map { it.id },
        )
    }

    @Test
    fun doesNotMatchAnIncomingReplyThatMerelyLooksAlike() {
        val optimistic = localSend(dateEpochSeconds = 2000)
        // Same envelope shape, but the core did not classify it as ours.
        val incoming =
            storedCopy(id = "m2", messageId = "theirs@example.com", dateEpochSeconds = 2005).copy(outgoing = false)

        assertEquals(
            listOf("local-send-1", "m2"),
            mergeLocalSendMessages(listOf(optimistic), listOf(incoming)).map { it.id },
        )
    }

    private fun message(
        id: String,
        messageId: String,
        dateEpochSeconds: Long,
    ): MessageBody =
        MessageBody(
            id = id,
            folderId = "INBOX",
            from = "Them",
            fromAddr = "them@example.com",
            to = "me@example.com",
            subject = "Subject",
            body = "Body",
            messageId = messageId,
            dateEpochSeconds = dateEpochSeconds,
        )

    private fun localSend(
        id: String = "local-send-1",
        messageId: String = "reply@example.com",
        dateEpochSeconds: Long,
    ): MessageBody =
        MessageBody(
            id = id,
            folderId = "INBOX",
            from = "You",
            fromAddr = "me@example.com",
            to = "Them <them@example.com>",
            subject = "Re: Subject",
            body = "Body",
            messageId = messageId,
            dateEpochSeconds = dateEpochSeconds,
            sendStatus = SendStatus.Sending,
        )

    private fun storedCopy(
        id: String,
        messageId: String,
        dateEpochSeconds: Long,
    ): MessageBody =
        MessageBody(
            id = id,
            folderId = "Sent",
            from = "You",
            fromAddr = "me@example.com",
            to = "them@example.com",
            subject = "Re: Subject",
            body = "Body",
            messageId = messageId,
            dateEpochSeconds = dateEpochSeconds,
            outgoing = true,
        )
}
