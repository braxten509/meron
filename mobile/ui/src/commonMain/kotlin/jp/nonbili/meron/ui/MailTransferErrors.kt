// Builds mobile error messages for failed moves
// and copies. Confirmed progress matters because
// part of a batch can already be at its
// destination.

package jp.nonbili.meron.ui

/** Explain known server rejections while retaining progress and uncertain-write warnings. */
internal fun mailTransferError(
    error: Throwable?,
    fallback: String,
    lookup: (String, Map<String, Any?>) -> String,
): String {
    val detail = error?.message?.takeIf { it.isNotBlank() } ?: fallback
    val lower = detail.lowercase()

    fun matches(pattern: String) = Regex(pattern).containsMatchIn(lower)
    var uncertain = false
    val reasonKey =
        when {
            matches("""\btoobig\b|mail size is larger|message (?:size .*exceed|too (?:large|big))|maximum (?:message|mail) size""") -> {
                "sizeLimit"
            }

            matches("""\boverquota\b|quota exceeded|mailbox (?:is )?full|insufficient (?:disk )?space""") -> {
                "quota"
            }

            matches("""\btrycreate\b|\bnonexistent\b|mailbox does not exist|no such (?:mailbox|folder)""") -> {
                "missingFolder"
            }

            matches("""\bnoperm\b|permission denied|not permitted|access denied""") -> {
                "permission"
            }

            matches("""\bauthenticationfailed\b|authentication failed|login failed|account needs reconnect""") -> {
                "authentication"
            }

            matches("""timed? out|timeout""") -> {
                uncertain = true
                "timeout"
            }

            matches("""connection (?:lost|closed|reset|refused|aborted)|broken pipe|unexpected eof|peer closed|dns lookup|failed to lookup address|network (?:is )?unreachable""") -> {
                uncertain = true
                "connection"
            }

            else -> {
                null
            }
        }
    val progress = Regex("""Copy progress: (\d+)/(\d+) confirmed; message size: (\d+) bytes""").find(detail)
    val copiedNotRemoved = lower.contains("move copied messages but could not confirm removal from the source folder")
    val reason = reasonKey?.let { lookup("mail.transferError.$it", emptyMap()) }
    val parts =
        mutableListOf(
            when {
                reason != null -> withReason(fallback, reason, lookup)
                copiedNotRemoved -> fallback
                else -> detail
            },
        )
    if (copiedNotRemoved) parts += lookup("mail.transferError.copiedNotRemoved", emptyMap())
    if (progress != null) {
        if (reasonKey == "sizeLimit") {
            parts += lookup("mail.transferError.messageSize", mapOf("bytes" to progress.groupValues[3].toLong()))
        }
        parts +=
            lookup(
                "mail.transferError.progress",
                mapOf("copied" to progress.groupValues[1].toLong(), "total" to progress.groupValues[2].toLong()),
            )
    }
    if (uncertain || copiedNotRemoved || (progress != null && progress.groupValues[1].toLong() > 0)) {
        parts += lookup("mail.transferError.checkDestination", emptyMap())
    }
    return parts.joinToString(" ")
}
