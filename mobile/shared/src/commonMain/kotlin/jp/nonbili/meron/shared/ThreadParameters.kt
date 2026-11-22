// Thread Parameters for the mobile protocol.
// Used by the shared Kotlin client when it calls into the Rust core.
// Wire keys and request identifiers must stay compatible across platforms.
package jp.nonbili.meron.shared

data class ContactSuggestParams(
    val accountId: String = "",
    val query: String = "",
    val limit: Int = 8,
    // frequent (autocomplete default), name, or recent.
    val sort: String = "frequent",
    val directory: Boolean = false,
) {
    fun toJson(): String =
        jsonObject(
            "account" to accountId.jsonString(),
            "query" to query.jsonString(),
            "limit" to limit.toString(),
            "sort" to if (sort == "frequent") null else sort.jsonString(),
            "directory" to if (directory) "true" else null,
        )
}

/** Exact-address, cache-only history across all accounts and folders. */
data class ContactThreadsParams(
    val addr: String,
    val beforeCursor: String? = null,
    val limit: Int = 50,
) {
    fun toJson(): String =
        jsonObject(
            "addr" to addr.jsonString(),
            "before_cursor" to beforeCursor?.jsonString(),
            "limit" to limit.toString(),
        )
}

data class ThreadListParams(
    val accountId: String,
    val folderId: String = "inbox",
    val query: String = "",
    val filter: String = "all",
    // Keep only threads with attachments; combines with `filter`.
    val attachments: Boolean = false,
    val beforeCursor: String? = null,
    val refresh: Boolean = false,
    // Header rows to return, per account (the unified fan-out applies it to each
    // account's page). Omitted when null so the core keeps its own default; the
    // mailbox passes the depth it has already paged in so a background reload
    // does not shrink the list back to one page.
    val limit: Int? = null,
    // Unified only: which special-use folder each account answers from
    // ("inbox", "sent", "archive", …). Accounts whose server has no such
    // mailbox drop out of the merge. Omitted for a single account, whose
    // folderId already names a real mailbox.
    val folderRole: String? = null,
    // "sender" groups the plain list one row per correspondent; null (omitted)
    // keeps thread rows. The core ignores it for search, Starred and Drafts.
    val groupBy: String? = null,
) {
    fun toJson(): String =
        jsonObject(
            "account_id" to accountId.jsonString(),
            "folder_id" to folderId.jsonString(),
            "query" to query.jsonString(),
            "filter" to filter.jsonString(),
            "attachments" to attachments.toString(),
            "before_cursor" to beforeCursor?.jsonString(),
            "refresh" to refresh.toString(),
            "limit" to limit?.toString(),
            "folder_role" to folderRole?.jsonString(),
            "group_by" to groupBy?.jsonString(),
        )
}

data class StarredItemsParams(
    val query: String = "",
    val filter: String = "all",
    val attachments: Boolean = false,
    val limit: Int = 50,
    val beforeCursor: String? = null,
) {
    fun toJson(): String =
        jsonObject(
            "query" to query.jsonString(),
            "filter" to filter.jsonString(),
            "attachments" to attachments.toString(),
            "limit" to limit.toString(),
            "before_cursor" to beforeCursor?.jsonString(),
        )
}

data class AllocateIdentityParams(
    val accountId: String,
    val draft: Boolean,
) {
    fun toJson(): String =
        jsonObject(
            "account_id" to accountId.jsonString(),
            "draft" to draft.toString(),
        )
}

data class ThreadReadParams(
    val threadId: String,
    val beforeCursor: String? = null,
    val limit: Int? = CONVERSATION_PAGE_SIZE,
    val forPrint: Boolean = false,
) {
    fun toJson(): String =
        jsonObject(
            "thread_id" to threadId.jsonString(),
            "before_cursor" to beforeCursor?.jsonString(),
            "limit" to limit?.toString(),
            "for_print" to if (forPrint) "true" else null,
        )
}

data class AttachmentReadParams(
    val key: String,
) {
    fun toJson(): String = jsonObject("key" to key.jsonString())
}

data class SyncMailParams(
    val accountId: String,
    val folderId: String = "inbox",
    val limit: Int = 50,
    val folders: Boolean = true,
    // Foreground-only: lets the core run the non-gating sync tail (body
    // prefetch, Sent/Drafts headers) after returning, so the inbox list isn't
    // blocked on it. Background workers must leave this false — their
    // execution window ends when the call returns.
    val deferTail: Boolean = false,
) {
    fun toJson(): String =
        jsonObject(
            "account_id" to accountId.jsonString(),
            "folder_id" to folderId.jsonString(),
            "limit" to limit.toString(),
            "folders" to folders.toString(),
            "defer_tail" to deferTail.toString(),
        )
}

data class SyncRssParams(
    val accountId: String,
) {
    fun toJson(): String = jsonObject("account_id" to accountId.jsonString())
}

data class RssThreadParams(
    val threadId: String,
    val beforeCursor: String? = null,
    val limit: Int? = CONVERSATION_PAGE_SIZE,
) {
    fun toJson(): String =
        jsonObject(
            "thread_id" to threadId.jsonString(),
            "before_cursor" to beforeCursor?.jsonString(),
            "limit" to limit?.toString(),
        )
}
