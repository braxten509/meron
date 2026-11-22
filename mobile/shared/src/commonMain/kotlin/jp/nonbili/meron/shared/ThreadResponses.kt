// Conversation pages and message details returned by the shared core.
// Mailbox rows, starred items and individual messages share identities across views.
// The message reader also decodes invitations and the page continuation supplied by the core.
package jp.nonbili.meron.shared

data class ThreadListPage(
    val threads: List<ThreadSummary>,
    val nextCursor: String,
    val folderUnread: Int? = null,
    val folderSynced: Boolean? = null,
    // The live server search failed, so these are only locally cached matches.
    val searchIncomplete: Boolean = false,
)

internal fun decodeThreadListPage(responseJson: String): ThreadListPage {
    val threadsJson =
        responseJson.findJsonArrayProperty("threads")
            ?: return ThreadListPage(
                threads = emptyList(),
                nextCursor = "",
                folderUnread = responseJson.findJsonLongProperty("folder_unread")?.toInt(),
                folderSynced = responseJson.findJsonBooleanProperty("folder_synced"),
                searchIncomplete = responseJson.findJsonBooleanProperty("search_incomplete") == true,
            )
    val threads =
        threadsJson.jsonArrayElements().mapNotNull { item ->
            val id = item.findJsonStringProperty("id").orEmpty()
            if (id.isBlank()) return@mapNotNull null
            ThreadSummary(
                id = id,
                accountId = item.findJsonStringProperty("account_id").orEmpty(),
                folder = item.findJsonStringProperty("folder_id") ?: item.findJsonStringProperty("folder").orEmpty(),
                folderRole = item.findJsonStringProperty("folder_role") ?: "folder",
                subject = item.findJsonStringProperty("subject").orEmpty(),
                // The envelope carries an empty from_name when the sender has
                // no display name, so fall through blanks to the address rather
                // than leaving the label empty for the row to fill in.
                sender =
                    item
                        .findJsonStringProperty("from_name")
                        .orEmpty()
                        .ifBlank { item.findJsonStringProperty("from_addr").orEmpty() }
                        .ifBlank { item.findJsonStringProperty("from").orEmpty() },
                preview = item.findJsonStringProperty("preview").orEmpty(),
                unread = item.findJsonBooleanProperty("unread") ?: false,
                unreadCount =
                    item.findJsonLongProperty("unread_count")?.toInt()
                        ?: if (item.findJsonBooleanProperty("unread") == true) 1 else 0,
                messageCount = item.findJsonLongProperty("message_count")?.toInt() ?: 0,
                // Top-level key: the subject is sender-written text and precedes it.
                senders =
                    item
                        .jsonObjectEntries()
                        .firstOrNull { it.first == "senders" }
                        ?.second
                        ?.takeIf { it.startsWith("[") }
                        ?.jsonArrayElements()
                        ?.map { sender ->
                            if (sender.findJsonBooleanProperty("me") == true) "" else sender.findJsonStringProperty("name").orEmpty()
                        }.orEmpty(),
                sendersTruncated =
                    item.jsonObjectEntries().firstOrNull { it.first == "senders_truncated" }?.second == "true",
                starred = item.findJsonBooleanProperty("starred") ?: false,
                // Top-level key, like senders: subjects are sender-written text.
                tags = item.topLevelStringArray("tags"),
                hasStarredItems = item.findJsonBooleanProperty("has_starred_items") ?: false,
                hasDraft = item.findJsonBooleanProperty("has_draft") ?: false,
                hasAttachments = item.jsonObjectEntries().firstOrNull { it.first == "has_attachments" }?.second == "true",
                // Top-level key, like senders: filenames are sender-written text.
                files =
                    item
                        .jsonObjectEntries()
                        .firstOrNull { it.first == "files" }
                        ?.second
                        ?.takeIf { it.startsWith("[") }
                        ?.jsonArrayElements()
                        ?.mapNotNull { file ->
                            val filename = file.findJsonStringProperty("filename").orEmpty()
                            if (filename.isBlank()) return@mapNotNull null
                            MessageAttachment(filename = filename, mimeType = file.findJsonStringProperty("mime").orEmpty())
                        }.orEmpty(),
                dateEpochSeconds = item.findJsonLongProperty("date") ?: item.findJsonLongProperty("date_epoch_seconds") ?: 0,
                feedUrl = item.findJsonStringProperty("feed_url").orEmpty(),
                senderAddress = item.findJsonStringProperty("from_addr").orEmpty(),
                // Top-level keys: a sender row (`group: "sender"`) and whether
                // that person is a no-reply style address.
                group =
                    item
                        .jsonObjectEntries()
                        .firstOrNull { it.first == "group" }
                        ?.second
                        ?.let { "{\"value\":$it}".findJsonStringProperty("value") }
                        .orEmpty(),
                automated = item.jsonObjectEntries().firstOrNull { it.first == "automated" }?.second == "true",
                // Top-level key: null when the conversation is in no Meron folder.
                meronFolderId =
                    item
                        .jsonObjectEntries()
                        .firstOrNull { it.first == "meron_folder_id" }
                        ?.second
                        ?.takeIf { it.startsWith("\"") }
                        ?.let { "{\"value\":$it}".findJsonStringProperty("value") }
                        ?.takeIf { it.isNotBlank() },
            )
        }
    return ThreadListPage(
        // Thread ids key the LazyColumn items; a duplicate id in one core
        // response (e.g. unified or search listings) would crash composition.
        threads = threads.distinctBy { it.id },
        nextCursor = responseJson.findJsonStringProperty("next_cursor").orEmpty(),
        folderUnread = responseJson.findJsonLongProperty("folder_unread")?.toInt(),
        folderSynced = responseJson.findJsonBooleanProperty("folder_synced"),
        searchIncomplete = responseJson.findJsonBooleanProperty("search_incomplete") == true,
    )
}

data class ThreadReadPage(
    val messages: List<MessageBody>,
    val nextCursor: String,
)

data class StarredItemsPage(
    val items: List<StarredItemSummary>,
    val nextCursor: String,
)

internal fun decodeStarredItemsPage(responseJson: String): StarredItemsPage =
    StarredItemsPage(
        items = parseStarredItemsResponse(responseJson),
        nextCursor = responseJson.findJsonStringProperty("next_cursor").orEmpty(),
    )

internal fun decodeStarredItemsResponse(responseJson: String): List<StarredItemSummary> {
    val itemsJson = responseJson.findJsonArrayProperty("items") ?: return emptyList()
    return itemsJson
        .jsonArrayElements()
        .mapNotNull { item ->
            val id = item.findJsonStringProperty("id").orEmpty()
            val threadId = item.findJsonStringProperty("thread_id").orEmpty()
            if (id.isBlank() || threadId.isBlank()) return@mapNotNull null
            StarredItemSummary(
                id = id,
                threadId = threadId,
                accountId = item.findJsonStringProperty("account_id").orEmpty(),
                folder = item.findJsonStringProperty("folder_id") ?: item.findJsonStringProperty("folder").orEmpty(),
                folderRole = item.findJsonStringProperty("folder_role") ?: "folder",
                subject = item.findJsonStringProperty("subject").orEmpty(),
                // The envelope carries an empty from_name when the sender has
                // no display name, so fall through blanks to the address rather
                // than leaving the label empty for the row to fill in.
                sender =
                    item
                        .findJsonStringProperty("from_name")
                        .orEmpty()
                        .ifBlank { item.findJsonStringProperty("from_addr").orEmpty() }
                        .ifBlank { item.findJsonStringProperty("from").orEmpty() },
                preview = item.findJsonStringProperty("preview").orEmpty(),
                unread = item.findJsonBooleanProperty("unread") ?: false,
                dateEpochSeconds = item.findJsonLongProperty("date") ?: item.findJsonLongProperty("date_epoch_seconds") ?: 0,
            )
        }.distinctBy { it.id }
}

internal fun decodeThreadReadPage(responseJson: String): ThreadReadPage {
    val messagesJson =
        responseJson.findJsonArrayProperty("messages")
            ?: return ThreadReadPage(messages = emptyList(), nextCursor = "")
    val messages =
        messagesJson.jsonArrayElements().mapNotNull { item ->
            val id = item.findJsonStringProperty("id").orEmpty()
            if (id.isBlank()) return@mapNotNull null
            val fromName = item.findJsonStringProperty("from_name").orEmpty()
            val fromAddr = item.findJsonStringProperty("from_addr").orEmpty()
            // Read the reply recipients out of the message's *top-level* keys.
            // The scanning helpers below match the first occurrence anywhere in
            // the object, and the body — attacker-written text — is serialized
            // ahead of this key, so a mail whose body contains its own
            // "reply": {...} would otherwise choose who the user replies to.
            val entries = item.jsonObjectEntries()
            val replyObject = entries.firstOrNull { it.first == "reply" }?.second
            // Top-level for the same reason: a forged offset in the body would
            // fold away text of the sender's choosing.
            val bodyQuoteStart = entries.firstOrNull { it.first == "body_quote_start" }?.second?.toIntOrNull()

            // The core's keys arrive sorted, so "reply" (whose own "to"/"cc"
            // address the sender) precedes the top-level recipients; a scan
            // would show the sender as the message's To.
            fun recipients(name: String): String =
                entries
                    .firstOrNull { it.first == name }
                    ?.second
                    ?.takeIf { it.startsWith('"') }
                    ?.readResponseString(0)
                    ?.value
                    .orEmpty()
            MessageBody(
                id = id,
                folderId = item.findJsonStringProperty("folder_id") ?: item.findJsonStringProperty("folder").orEmpty(),
                from = fromName.ifBlank { fromAddr },
                to = recipients("to"),
                cc = recipients("cc"),
                bcc = recipients("bcc"),
                subject = item.findJsonStringProperty("subject").orEmpty(),
                body = item.findJsonStringProperty("body").orEmpty(),
                bodyHtml = item.findJsonStringProperty("body_html").orEmpty(),
                mediaMissing = item.findJsonLongProperty("media_missing")?.toInt()?.coerceAtLeast(0) ?: 0,
                bodyQuoteStart = bodyQuoteStart,
                dateEpochSeconds = item.findJsonLongProperty("date") ?: item.findJsonLongProperty("date_epoch_seconds") ?: 0,
                fromAddr = fromAddr,
                replyTo = item.findJsonStringProperty("reply_to").orEmpty(),
                messageId = item.findJsonStringProperty("message_id").orEmpty(),
                inReplyTo = item.findJsonStringProperty("in_reply_to").orEmpty(),
                references = item.findJsonStringProperty("references").orEmpty(),
                unread = item.findJsonBooleanProperty("unread") ?: false,
                outgoing = item.findJsonBooleanProperty("outgoing") ?: false,
                starred = item.findJsonBooleanProperty("starred") ?: false,
                hasAttachments = item.findJsonBooleanProperty("has_attachments") ?: false,
                bodyMissing = item.findJsonBooleanProperty("body_missing") ?: false,
                bodyLoading = item.findJsonBooleanProperty("body_loading") ?: false,
                link = recipients("link"),
                reply =
                    replyObject?.takeIf { it.startsWith("{") }?.let { reply ->
                        MessageReply(
                            to = reply.findJsonStringProperty("to").orEmpty(),
                            cc = reply.findJsonStringProperty("cc").orEmpty(),
                            allTo = reply.findJsonStringProperty("all_to").orEmpty(),
                            allCc = reply.findJsonStringProperty("all_cc").orEmpty(),
                            allAddsRecipients = reply.findJsonBooleanProperty("all_adds_recipients") ?: false,
                            acceptsReplies = reply.findJsonBooleanProperty("accepts_replies") ?: true,
                        )
                    },
                invite = entries.firstOrNull { it.first == "invite" }?.second?.let(::parseMessageInvite),
                attachments =
                    item
                        .findJsonArrayProperty("attachments")
                        ?.jsonArrayElements()
                        ?.mapNotNull { attachmentJson ->
                            val filename = attachmentJson.findJsonStringProperty("filename").orEmpty()
                            if (filename.isBlank()) return@mapNotNull null
                            MessageAttachment(
                                filename = filename,
                                mimeType = attachmentJson.findJsonStringProperty("mime").orEmpty(),
                                sizeBytes = attachmentJson.findJsonLongProperty("size") ?: 0,
                                key = attachmentJson.findJsonStringProperty("key").orEmpty(),
                                url = attachmentJson.findJsonStringProperty("url").orEmpty(),
                            )
                        }.orEmpty(),
            )
        }
    return ThreadReadPage(
        // Message ids key the conversation LazyColumn items; duplicates crash.
        messages = messages.distinctBy { it.id },
        nextCursor = responseJson.findJsonStringProperty("next_cursor").orEmpty(),
    )
}

/**
 * A thread message's `invite` object. Read by its own top-level keys: the
 * summary, location and attendee names are the sender's text, so a scan could
 * be steered by them.
 */
private fun parseMessageInvite(json: String): MessageInvite? {
    if (!json.startsWith("{")) return null
    val entries = json.jsonObjectEntries().toMap()

    fun string(value: String?): String =
        value
            ?.takeIf { it.startsWith('"') }
            ?.readResponseString(0)
            ?.value
            .orEmpty()

    fun person(value: String?): Map<String, String> =
        value
            ?.takeIf { it.startsWith("{") }
            ?.jsonObjectEntries()
            ?.toMap()
            .orEmpty()
    val organizer = person(entries["organizer"])
    return MessageInvite(
        method = string(entries["method"]),
        uid = string(entries["uid"]),
        summary = string(entries["summary"]),
        location = string(entries["location"]),
        start = entries["start"]?.toLongOrNull() ?: 0,
        end = entries["end"]?.toLongOrNull() ?: 0,
        allDay = entries["all_day"] == "true",
        recurring = entries["recurring"] == "true",
        cancelled = entries["cancelled"] == "true",
        organizerName = string(organizer["name"]),
        organizerAddr = string(organizer["addr"]),
        attendees =
            entries["attendees"]
                ?.takeIf { it.startsWith("[") }
                ?.jsonArrayElements()
                ?.map { element ->
                    val attendee = person(element)
                    InviteAttendee(
                        name = string(attendee["name"]),
                        addr = string(attendee["addr"]),
                        partstat = string(attendee["partstat"]),
                    )
                }.orEmpty(),
        canRespond = entries["can_respond"] == "true",
        response = string(entries["response"]),
        eventId = string(entries["event_id"]),
        recurrenceId = entries["recurrence_id"]?.toLongOrNull() ?: 0,
    )
}
