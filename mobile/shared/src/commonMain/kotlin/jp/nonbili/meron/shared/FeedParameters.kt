// Feed Parameters for the mobile protocol.
// Used by the shared Kotlin client when it calls into the Rust core.
// Wire keys and request identifiers must stay compatible across platforms.
package jp.nonbili.meron.shared

data class AddRssFeedParams(
    val accountId: String,
    val feedUrl: String,
) {
    fun toJson(): String =
        jsonObject(
            "account" to accountId.jsonString(),
            "feed_url" to feedUrl.jsonString(),
        )
}

data class RemoveRssFeedParams(
    val threadId: String,
) {
    fun toJson(): String = jsonObject("thread_id" to threadId.jsonString())
}

data class MoveRssFeedParams(
    val threadId: String,
    val targetAccountId: String,
) {
    fun toJson(): String =
        jsonObject(
            "thread_id" to threadId.jsonString(),
            "target_account" to targetAccountId.jsonString(),
        )
}

data class ExportOpmlParams(
    val accountId: String,
) {
    fun toJson(): String = jsonObject("account" to accountId.jsonString())
}

data class ImportOpmlParams(
    val accountId: String,
    val opml: String,
) {
    fun toJson(): String =
        jsonObject(
            "account" to accountId.jsonString(),
            "opml" to opml.jsonString(),
        )
}


data class RssMarkReadParams(
    val threadId: String,
    val seen: Boolean = true,
    val itemKeys: List<String> = emptyList(),
) {
    fun toJson(): String =
        jsonObject(
            "thread_id" to threadId.jsonString(),
            "seen" to seen.toString(),
            "item_keys" to itemKeys.takeUnless { it.isEmpty() }?.jsonStringArray(),
        )
}

data class RssMarkStarredParams(
    val threadId: String,
    val starred: Boolean,
    val itemKeys: List<String> = emptyList(),
) {
    fun toJson(): String =
        jsonObject(
            "thread_id" to threadId.jsonString(),
            "starred" to starred.toString(),
            "item_keys" to itemKeys.takeUnless { it.isEmpty() }?.jsonStringArray(),
        )
}
