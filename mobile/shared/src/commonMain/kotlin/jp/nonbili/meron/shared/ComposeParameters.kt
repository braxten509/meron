// Compose Parameters for the mobile protocol.
// Used by the shared Kotlin client when it calls into the Rust core.
// Wire keys and request identifiers must stay compatible across platforms.
package jp.nonbili.meron.shared

data class MobileAttachmentInput(
    val filename: String,
    val mime: String,
    val data: String,
    val inlineId: String = "",
) {
    fun toJson(): String =
        jsonObject(
            "filename" to filename.jsonString(),
            "mime" to mime.jsonString(),
            "data" to data.jsonString(),
            "inline_id" to inlineId.jsonString(),
        )
}

data class SendMailParams(
    val accountId: String,
    val to: String,
    val subject: String,
    val body: String,
    val from: String = "",
    val cc: String = "",
    val bcc: String = "",
    val replyTo: String = "",
    val html: String = "",
    val inReplyTo: String = "",
    val references: String = "",
    val messageId: String = "",
    val attachments: List<MobileAttachmentInput> = emptyList(),
) {
    fun toJson(): String =
        jsonObject(
            "account_id" to accountId.jsonString(),
            "from" to from.jsonString(),
            "to" to to.jsonString(),
            "cc" to cc.jsonString(),
            "bcc" to bcc.jsonString(),
            "reply_to" to replyTo.jsonString(),
            "subject" to subject.jsonString(),
            "body" to body.jsonString(),
            "html" to html.jsonString(),
            "in_reply_to" to inReplyTo.jsonString(),
            "references" to references.jsonString(),
            "message_id" to messageId.jsonString(),
            "attachments" to attachments.joinToString(separator = ",", prefix = "[", postfix = "]") { it.toJson() },
        )
}

data class SaveDraftParams(
    val accountId: String,
    val draftId: String,
    val to: String,
    val subject: String,
    val body: String,
    val from: String = "",
    val cc: String = "",
    val bcc: String = "",
    val replyTo: String = "",
    val html: String = "",
    val inReplyTo: String = "",
    val references: String = "",
    val attachments: List<MobileAttachmentInput> = emptyList(),
) {
    fun toJson(): String =
        jsonObject(
            "account_id" to accountId.jsonString(),
            "draft_id" to draftId.jsonString(),
            "from" to from.jsonString(),
            "to" to to.jsonString(),
            "cc" to cc.jsonString(),
            "bcc" to bcc.jsonString(),
            "reply_to" to replyTo.jsonString(),
            "subject" to subject.jsonString(),
            "body" to body.jsonString(),
            "html" to html.jsonString(),
            "in_reply_to" to inReplyTo.jsonString(),
            "references" to references.jsonString(),
            "attachments" to attachments.joinToString(separator = ",", prefix = "[", postfix = "]") { it.toJson() },
        )
}

data class DiscardDraftParams(
    val accountId: String,
    val draftId: String,
) {
    fun toJson(): String =
        jsonObject(
            "account_id" to accountId.jsonString(),
            "draft_id" to draftId.jsonString(),
        )
}

fun ComposeDraft.toSendMailParams(
    accountId: String,
    from: String = "",
): SendMailParams =
    SendMailParams(
        accountId = accountId,
        from = from,
        to = to,
        cc = cc,
        bcc = bcc,
        subject = subject,
        body = body,
        html = html,
        attachments =
            attachments.map {
                MobileAttachmentInput(
                    filename = it.displayName,
                    mime = it.mimeType,
                    data = it.dataBase64,
                    inlineId = it.inlineId,
                )
            },
    )

fun ComposeDraft.toSaveDraftParams(
    accountId: String,
    draftId: String,
    from: String = "",
): SaveDraftParams =
    SaveDraftParams(
        accountId = accountId,
        draftId = draftId,
        from = from,
        to = to,
        cc = cc,
        bcc = bcc,
        subject = subject,
        body = body,
        html = html,
        attachments =
            attachments.map {
                MobileAttachmentInput(
                    filename = it.displayName,
                    mime = it.mimeType,
                    data = it.dataBase64,
                    inlineId = it.inlineId,
                )
            },
    )

fun MessageBody.toReplyMailParams(
    accountId: String,
    body: String,
    from: String = "",
    attachments: List<DraftAttachment> = emptyList(),
    replyAll: Boolean = false,
): SendMailParams {
    val recipients = buildReplyRecipients(this, replyAll)
    val replySubject = if (subject.startsWith("Re:", ignoreCase = true)) subject else "Re: $subject"
    val parentMessageId = messageId.trim().trim('<', '>')
    val parentReference = parentMessageId.takeIf { it.isNotBlank() }?.let { "<$it>" }.orEmpty()
    val nextReferences =
        listOf(references.trim(), parentReference)
            .filter { it.isNotBlank() }
            .joinToString(" ")
    return SendMailParams(
        accountId = accountId,
        from = from,
        to = recipients.to,
        cc = recipients.cc,
        subject = replySubject,
        body = body,
        inReplyTo = parentReference,
        references = nextReferences,
        attachments =
            attachments.map {
                MobileAttachmentInput(
                    filename = it.displayName,
                    mime = it.mimeType,
                    data = it.dataBase64,
                )
            },
    )
}

// A reply with the original quoted under it (see [replyQuotedPlainBody]): the
// quote joins the plain body, and an HTML original's quote makes the reply's
// HTML alternative, with the user's text converted above it and the inline
// images the quote references attached. A body that already quotes the message
// — a saved quoted reply, reopened — is not quoted twice: one that still ends in
// the quote untouched keeps its HTML, and one whose quote the user edited (its
// [marker], the attribution line, still there) goes out exactly as written.
fun SendMailParams.withReplyQuote(
    quoteText: String,
    quoteHtml: String,
    marker: String,
    inlineImages: List<DraftAttachment> = emptyList(),
): SendMailParams {
    val quote = quoteText.trim()
    if (quote.isBlank()) return this
    val sent = body.trim()
    val quoted = sent.endsWith(quote)
    if (!quoted && marker.isNotBlank() && sent.contains(marker)) return this
    val own = if (quoted) sent.removeSuffix(quote).trim() else sent
    val html = if (quoteHtml.isBlank()) "" else plainTextToHtml(own) + quoteHtml
    return copy(
        body =
            when {
                quoted -> sent
                own.isBlank() -> quote
                else -> own + quoteText
            },
        html = html,
        attachments =
            if (html.isBlank()) {
                attachments
            } else {
                attachments + inlineImages.map { MobileAttachmentInput(it.displayName, it.mimeType, it.dataBase64, it.inlineId) }
            },
    )
}
