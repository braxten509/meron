// Defines mailbox, conversation, message and draft data used by mobile screens.
// Core response parsers construct these models and the UI renders their contents.
// Attachment, invitation and send state remain attached to the message they describe.

package jp.nonbili.meron.shared

data class FolderSummary(
    val accountId: String,
    /** Wire name: what SELECT, sync and cached rows address the mailbox by. */
    val name: String,
    val unread: Int = 0,
    val role: String = "folder",
    /** `name` decoded from modified UTF-7; equal to `name` for ASCII folders. */
    val displayName: String = name,
    /** Hierarchy separator the server reported for this mailbox, if any. */
    val delimiter: String = "",
    /** The user opted this folder in to background sync and new-mail notifications. */
    val notify: Boolean = false,
    /** Opted in and holding one of the account's IMAP IDLE slots. Core polls
     *  the opted-in folders past those, so only these need a live watch. */
    val notifyLive: Boolean = false,
)

data class ThreadSummary(
    val id: String,
    val accountId: String,
    val folder: String,
    val folderRole: String = "folder",
    val subject: String,
    val sender: String,
    val preview: String = "",
    val unread: Boolean = false,
    val unreadCount: Int = 0,
    /** Total messages in the thread, read or not; 0 when the core did not group. */
    val messageCount: Int = 0,
    /** Distinct senders oldest first when the thread has more than one; a blank
     *  entry is the account itself, for the row to say "me". */
    val senders: List<String> = emptyList(),
    /** Senders between the first entry and the rest were left out of a long list. */
    val sendersTruncated: Boolean = false,
    val starred: Boolean = false,
    /** Tags on the thread's messages, sorted. */
    val tags: List<String> = emptyList(),
    val hasStarredItems: Boolean = false,
    val hasDraft: Boolean = false,
    val hasAttachments: Boolean = false,
    /** The thread's attachments in thread order, for the chips under the row;
     *  only messages whose body is cached are known. Name and type only. */
    val files: List<MessageAttachment> = emptyList(),
    val dateEpochSeconds: Long = 0,
    val feedUrl: String = "",
    val threadId: String = "",
    /** The other person's address (`from_addr`); blank when the core sent none. */
    val senderAddress: String = "",
    /** "sender" for a row standing for one correspondent (group-by-sender
     *  mode), blank for an ordinary thread row. */
    val group: String = "",
    /** The correspondent is a no-reply style address: nothing to reply to. */
    val automated: Boolean = false,
    /** The Meron folder the conversation was moved into on this device, if any. */
    val meronFolderId: String? = null,
)

/** A folder that exists only inside Meron, on this device. */
data class MeronFolderSummary(
    val id: String,
    val name: String,
    val unread: Int = 0,
    val count: Int = 0,
    val createdAt: Long = 0,
    /** New mail AI sorting files here notifies (this device only). */
    val notify: Boolean = false,
    /** `"archive"` for the built-in Archive folder (always there, can't be
     *  renamed or deleted); null for the user's own. */
    val builtin: String? = null,
)

data class StarredItemSummary(
    val id: String,
    val threadId: String,
    val accountId: String,
    val folder: String,
    val folderRole: String = "folder",
    val subject: String,
    val sender: String,
    val preview: String = "",
    val unread: Boolean = false,
    val dateEpochSeconds: Long = 0,
)

data class MessageAttachment(
    val filename: String,
    val mimeType: String = "application/octet-stream",
    val sizeBytes: Long = 0,
    val key: String = "",
    val url: String = "",
)

data class ThreadGalleryImage(
    val attachment: MessageAttachment,
    val ref: String,
    val filename: String,
    val messageId: String,
)

data class ThreadMediaItem(
    val attachment: MessageAttachment,
    val ref: String,
    val filename: String,
    val type: String,
    val galleryIndex: Int? = null,
)

data class StorageUsage(
    val cacheBytes: Long = 0,
    val dbBytes: Long = 0,
)

// One release in the in-app changelog (the GitHub releases atom feed, filtered
// by the core to the mobile `android/v*` tags).
data class ChangelogRelease(
    val version: String,
    val tag: String,
    val date: String,
    val notes: List<String>,
)

// Send lifecycle for an optimistically inserted message. None covers both a
// freshly synced message and a successfully sent one (which is replaced by its
// canonical copy on re-fetch). Mirrors desktop's 'sending' | 'sent' | 'failed'.
enum class SendStatus {
    None,
    Sending,
    Failed,
}

data class MessageBody(
    val id: String,
    val folderId: String = "",
    val from: String,
    val to: String,
    val cc: String = "",
    val bcc: String = "",
    val subject: String,
    val body: String,
    val bodyHtml: String = "",
    // UTF-16 offset where the plain body's quoted tail starts, so it can be
    // folded; null when there is none. HTML bodies mark theirs inline.
    val bodyQuoteStart: Int? = null,
    val dateEpochSeconds: Long = 0,
    val fromAddr: String = "",
    val replyTo: String = "",
    val messageId: String = "",
    val inReplyTo: String = "",
    val references: String = "",
    val unread: Boolean = false,
    // Sent by this account, classified by the core (own address or Sent-folder
    // provenance) — true even for aliases not configured in meron.
    val outgoing: Boolean = false,
    val starred: Boolean = false,
    val hasAttachments: Boolean = false,
    // True when the core has no cached body for this message (the on-demand
    // fetch failed), as opposed to a message whose body is genuinely empty.
    val bodyMissing: Boolean = false,
    // With bodyMissing: the core is downloading the body in the background, so
    // it is not a failure yet and a re-read will deliver it.
    val bodyLoading: Boolean = false,
    // An RSS item's original page; empty for mail and for items without one.
    val link: String = "",
    val attachments: List<MessageAttachment> = emptyList(),
    val sendStatus: SendStatus = SendStatus.None,
    // Who a reply to this message addresses and copies, decided by the core so
    // mobile, desktop and the MCP tools agree. Null for a message shaped
    // locally (an optimistic send bubble) or by a version before the core sent
    // it.
    val reply: MessageReply? = null,
    // Files the core is restoring in the background; a decrease retries failed images.
    val mediaMissing: Int = 0,
    // The calendar event when the message is an invitation (or an update,
    // cancellation or reply to one).
    val invite: MessageInvite? = null,
)

data class DraftAttachment(
    val id: String,
    val displayName: String,
    val mimeType: String = "application/octet-stream",
    val sizeBytes: Long = 0,
    val dataBase64: String = "",
    // Set for images the draft's HTML references as `cid:<inlineId>`; the core
    // builds them as inline MIME parts rather than plain attachments.
    val inlineId: String = "",
)

// `html` is the optional rich alternative sent alongside the plain `body`. The
// mobile composer edits plain text only, so it is populated just for forwards,
// where dropping the original's HTML part would strip the message down to its
// (often empty) text alternative.
data class ComposeDraft(
    val to: String = "",
    val cc: String = "",
    val bcc: String = "",
    val subject: String = "",
    val body: String = "",
    val attachments: List<DraftAttachment> = emptyList(),
    val html: String = "",
) {
    constructor(to: String, subject: String, body: String) : this(to, "", "", subject, body, emptyList())

    val canSend: Boolean
        get() = to.isNotBlank() && subject.isNotBlank() && (body.isNotBlank() || attachments.isNotEmpty())
}

data class ReplyRecipients(
    val to: String,
    val cc: String,
)

/**
 * Both reply forms of one message, as the core decided them (see meron-core/src/reply.rs): `to`/`cc`
 * for a plain reply, `allTo`/`allCc` for reply-all, whether reply-all reaches anyone the plain reply
 * does not, and whether a reply reaches anyone at all (false for no-reply and bounce addresses, which
 * quick replies pass over).
 */
data class MessageReply(
    val to: String = "",
    val cc: String = "",
    val allTo: String = "",
    val allCc: String = "",
    val allAddsRecipients: Boolean = false,
    val acceptsReplies: Boolean = true,
)

/** One attendee of a calendar invitation; [partstat] is the lowercase
 * PARTSTAT (`needs-action`, `accepted`, `tentative`, `declined`, …). */
data class InviteAttendee(
    val name: String = "",
    val addr: String = "",
    val partstat: String = "",
)

/** The calendar event a message carries, as meron-core/src/calendar.rs
 * describes it. [start]/[end] are epoch seconds; for an all-day event they are
 * UTC midnights with [end] exclusive, so they are formatted in UTC. */
data class MessageInvite(
    val method: String = "",
    val uid: String = "",
    val summary: String = "",
    val location: String = "",
    val start: Long = 0,
    val end: Long = 0,
    val allDay: Boolean = false,
    val recurring: Boolean = false,
    val cancelled: Boolean = false,
    val organizerName: String = "",
    val organizerAddr: String = "",
    val attendees: List<InviteAttendee> = emptyList(),
    /** Whether Accept / Maybe / Decline apply: an open request from someone else. */
    val canRespond: Boolean = false,
    /** The user's answer (`accepted`, `tentative`, `declined`), empty when none. */
    val response: String = "",
    /** The calendar event this invitation is filed as; empty when it is not
     *  in the calendar. */
    val eventId: String = "",
    /** Which occurrence of a repeating event this is about (epoch seconds);
     *  0 for the whole event. */
    val recurrenceId: Long = 0,
)
