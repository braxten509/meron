// Search Parameters for the mobile protocol.
// Used by the shared Kotlin client when it calls into the Rust core.
// Wire keys and request identifiers must stay compatible across platforms.
package jp.nonbili.meron.shared

/**
 * The search options form, as the core's `search.parse` reads a query and
 * `search.build` writes it back. Dates are `yyyy-mm-dd`, or empty.
 */
data class SearchFields(
    val from: String = "",
    val to: String = "",
    val subject: String = "",
    val tags: String = "",
    val words: String = "",
    val after: String = "",
    val before: String = "",
    val unread: Boolean = false,
    val starred: Boolean = false,
    val hasAttachment: Boolean = false,
) {
    fun toJson(): String =
        jsonObject(
            "from" to from.jsonString(),
            "to" to to.jsonString(),
            "subject" to subject.jsonString(),
            "tags" to tags.jsonString(),
            "words" to words.jsonString(),
            "after" to after.jsonString(),
            "before" to before.jsonString(),
            "unread" to unread.toString(),
            "starred" to starred.toString(),
            "has_attachment" to hasAttachment.toString(),
        )
}
