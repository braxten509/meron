// Rule Parameters for the mobile protocol.
// Used by the shared Kotlin client when it calls into the Rust core.
// Wire keys and request identifiers must stay compatible across platforms.
package jp.nonbili.meron.shared

data class Rule(
    val id: String = "",
    val query: String,
    val tags: List<String> = emptyList(),
    val markRead: Boolean = false,
    val star: Boolean = false,
    val moveTo: String = "",
    val folder: String = "",
    /** The account of [folder]; empty for the mail's own account. */
    val folderAccount: String = "",
    /** The accounts the rule runs on; empty for all of them. */
    val accounts: List<String> = emptyList(),
    val copyFolder: String = "",
    val copyAccount: String = "",
    /** Kept, but not run. */
    val disabled: Boolean = false,
) {
    fun toJson(): String =
        jsonObject(
            "id" to id.jsonString(),
            "query" to query.jsonString(),
            "tags" to tags.jsonStringArray(),
            "mark_read" to markRead.toString(),
            "star" to star.toString(),
            "move_to" to moveTo.jsonString(),
            "folder" to folder.jsonString(),
            "folder_account" to folderAccount.jsonString(),
            "accounts" to accounts.jsonStringArray(),
            "copy_folder" to copyFolder.jsonString(),
            "copy_account" to copyAccount.jsonString(),
            "disabled" to disabled.toString(),
        )
}

/** One line of what rules did. [subject] is empty for a run by hand, which logs one line. */
data class RuleLogEntry(
    val at: Long = 0,
    val ruleId: String = "",
    val subject: String = "",
    val count: Int = 1,
    val failed: Boolean = false,
    val detail: String = "",
)

/** What rules did lately, and how many messages each rule run by hand has left. */
data class RuleStatus(
    val log: List<RuleLogEntry> = emptyList(),
    val running: Map<String, Int> = emptyMap(),
)

/** One folder the AI may sort into, with a short note of what belongs there. */
data class AiSortFolder(
    val account: String,
    val folder: String,
    val note: String = "",
    /** Set for a Meron folder (then [account] and [folder] are empty). */
    val meronFolderId: String = "",
) {
    val isMeron: Boolean get() = meronFolderId.isNotEmpty()

    fun toJson(): String =
        jsonObject(
            "account" to account.jsonString(),
            "folder" to folder.jsonString(),
            "note" to note.jsonString(),
            "meron_folder_id" to meronFolderId.takeIf { it.isNotEmpty() }?.jsonString(),
        )
}

/** The AI sorting setup. The API key is never part of it. */
data class AiSortConfig(
    val enabled: Boolean = false,
    /** `openrouter` or `tinfoil`. */
    val provider: String = "openrouter",
    /** Empty until the owner picks one: there is no default model. */
    val model: String = "",
    /** `headers` (sender and subject) or `start` (plus the start of the email). */
    val reads: String = "headers",
    val folders: List<AiSortFolder> = emptyList(),
    /** The core's last few models per service, newest first. Read only: the core keeps it. */
    val recentModels: Map<String, List<String>> = emptyMap(),
) {
    fun toJson(): String =
        jsonObject(
            "enabled" to enabled.toString(),
            "provider" to provider.jsonString(),
            "model" to model.jsonString(),
            "reads" to reads.jsonString(),
            "folders" to folders.joinToString(separator = ",", prefix = "[", postfix = "]") { it.toJson() },
        )
}

/** Whether a provider has a saved key; [hint] is its last few characters. */
data class AiSortKeyState(
    val hasKey: Boolean = false,
    val hint: String = "",
)

/** Why the last AI request failed. [kind] is `key`, `credit`, `verify` or `other`. */
data class AiSortError(
    val at: Long = 0,
    val message: String = "",
    val kind: String = "other",
)

/** The answer to `mail.aiSortConfig`. */
data class AiSortSetup(
    val config: AiSortConfig = AiSortConfig(),
    val keys: Map<String, AiSortKeyState> = emptyMap(),
    val lastError: AiSortError? = null,
)

/** The answer to `mail.checkAiSortKey`. */
data class AiSortKeyCheck(
    val ok: Boolean,
    val kind: String = "",
    val message: String = "",
)

/** One model a provider offers; prices are US dollars per million tokens, null when unknown. */
data class AiModel(
    val id: String,
    val name: String = id,
    val priceInPerM: Double? = null,
    val priceOutPerM: Double? = null,
    val structured: Boolean = false,
    val reasoning: Boolean = false,
    /** An OpenRouter decision model: one email per request, input billed only. */
    val decision: Boolean = false,
)

/** The answer to `mail.aiSortModels`; [default] is null (the owner always picks). */
data class AiModelList(
    val models: List<AiModel> = emptyList(),
    val default: String? = null,
)

/** What sorting the mail already in the Inbox would cost; [costUsd] is null when the price is unknown. */
data class AiSortEstimate(
    val total: Int = 0,
    val requests: Int = 0,
    val reads: String = "headers",
    val provider: String = "",
    val model: String = "",
    val costUsd: Double? = null,
    val accounts: Map<String, Int> = emptyMap(),
)

/** One line of what the AI did. [state] is `moving`, `moved`, `left`, `slow`, `failed` or `moved_back`. */
data class AiSortLogEntry(
    val id: Long,
    val at: Long = 0,
    val account: String = "",
    val subject: String = "",
    val sender: String = "",
    val toFolder: String = "",
    val reason: String = "",
    val state: String = "",
    val detail: String = "",
    /** Set when the AI filed the email into a Meron folder ([toFolder] is its name). */
    val meronFolderId: String = "",
)

/** The AI's log, the running "sort Inbox" job if any, and the last error. */
data class AiSortStatus(
    val log: List<AiSortLogEntry> = emptyList(),
    /** `total` to `done`, or null when no job runs. */
    val running: Pair<Int, Int>? = null,
    val lastError: AiSortError? = null,
)

/**
 * One thing AI sorting learned from mail the reader moved by hand. [place] is
 * `spam`, `inbox` or `meron` ([placeName] is then the Meron folder's name).
 * [used] is false when the AI can't follow it right now.
 */
data class AiSortLesson(
    val id: Long,
    val at: Long = 0,
    val account: String = "",
    val sender: String = "",
    val subject: String = "",
    val place: String = "",
    val placeName: String = "",
    val used: Boolean = true,
)
