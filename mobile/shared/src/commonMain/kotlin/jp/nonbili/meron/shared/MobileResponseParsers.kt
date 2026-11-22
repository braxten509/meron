// Public entry points for interpreting responses from the shared core.
// Desktop-compatible wire keys are decoded into Kotlin values for Android and iOS.
// The detailed account, folder, conversation and organizer readers live beside this file.
package jp.nonbili.meron.shared

// The core reports failures as a {"error":{"message":...}} JSON payload rather
// than throwing, so a raw invoke "succeeds" even when the action failed.
private val coreErrorMessageRegex =
    Regex("\"error\"\\s*:\\s*\\{[^{}]*\"message\"\\s*:\\s*\"((?:[^\"\\\\]|\\\\.)*)\"")

/** The error message from a core `{"error":{"message":...}}` payload, or null. */
fun coreErrorMessage(responseJson: String): String? =
    coreErrorMessageRegex.find(responseJson)?.groupValues?.getOrNull(1)?.let { raw ->
        raw.replace("\\\"", "\"").replace("\\n", "\n").replace("\\\\", "\\")
    }

/**
 * Throws when [responseJson] is a core error payload, so a caller's
 * runCatching/onFailure path runs instead of silently committing an optimistic
 * change the server rejected. Returns the response unchanged on success.
 */
fun requireCoreOk(responseJson: String): String {
    coreErrorMessage(responseJson)?.let { throw RuntimeException(it) }
    return responseJson
}

/**
 * [requireCoreOk] for a command that fans out across accounts, such as a unified
 * mark-all-read: it answers `"ok":false` with per-account `failures` instead of
 * an error payload when only some accounts failed, which must not read as done.
 */
fun requireCoreAllOk(responseJson: String): String {
    requireCoreOk(responseJson)
    if (responseJson.findJsonBooleanProperty("ok") == false) {
        val message =
            responseJson
                .findJsonArrayProperty("failures")
                ?.jsonArrayElements()
                ?.firstNotNullOfOrNull { it.findJsonStringProperty("message") }
        throw RuntimeException(message ?: "Some accounts could not be updated")
    }
    return responseJson
}

/**
 * Recognizes IMAP and SMTP authentication failures reported by the core.
 *
 * Mobile uses this signal to refresh a host-managed Google token and retry once.
 * The markers include XOAUTH2 invalid credentials and SMTP AUTH rejection.
 */
fun isOAuthLoginFailure(message: String?): Boolean {
    val normalized = message?.lowercase() ?: return false
    return "oauth login failed" in normalized ||
        "authenticationfailed" in normalized ||
        "invalid credentials" in normalized ||
        "smtp auth" in normalized
}

data class ThreadActionLocation(
    val threadId: String = "",
    val folder: String = "",
    val permanent: Boolean = false,
    /** The moved messages' UIDs in the destination folder, when the core reports them. */
    val targetUids: List<Long> = emptyList(),
    /** The Meron folder the action took the card out of, for Undo to put it back. */
    val meronFolderId: String = "",
    val meronSource: String = "",
    /** Archived Meron-only (the account has no archive mailbox): filed into
     *  the built-in Archive, the mail never moved. */
    val meronOnly: Boolean = false,
)

fun parseThreadActionLocationResponse(responseJson: String): ThreadActionLocation {
    // Core answers come wrapped as {"id":…,"result":{…}}; read the action's own fields.
    val body = responseJson.fields()["result"]?.takeIf { it.startsWith('{') } ?: responseJson
    return ThreadActionLocation(
        threadId = body.findJsonStringProperty("thread_id", topLevelOnly = true).orEmpty(),
        folder = body.findJsonStringProperty("folder", topLevelOnly = true) ?: body.findJsonStringProperty("trash", topLevelOnly = true).orEmpty(),
        permanent = body.findJsonBooleanProperty("permanent", topLevelOnly = true) ?: false,
        targetUids =
            body.fields()["target_uids"]
                ?.takeIf { it.startsWith('[') }
                ?.let { Regex("""\d+""").findAll(it).mapNotNull { match -> match.value.toLongOrNull() }.toList() }
                .orEmpty(),
        meronFolderId = body.findJsonStringProperty("meron_folder_id", topLevelOnly = true).orEmpty(),
        meronSource = body.findJsonStringProperty("meron_source", topLevelOnly = true).orEmpty(),
        meronOnly = body.findJsonBooleanProperty("meron_only", topLevelOnly = true) ?: false,
    )
}

/**
 * The answers of a bulk command such as `mail.deleteMany`: one per item, in
 * order. A success holds what the single command would have answered; a
 * failure holds the core's message for that item. Null when the command
 * answered no items at all.
 */
fun parseBulkActionResults(responseJson: String): List<Result<String>>? =
    responseJson.findJsonArrayProperty("results")?.jsonArrayElements()?.map { item ->
        val failure = item.findJsonStringProperty("error", topLevelOnly = true)
        if (failure != null) Result.failure(RuntimeException(failure)) else Result.success(item)
    }

fun parseAllocatedMessageId(responseJson: String): String = responseJson.findJsonStringProperty("message_id").orEmpty()

data class FolderUnreadChange(
    val accountId: String,
    val folderId: String,
    val unread: Int,
)

fun parseFolderUnreadChanges(responseJson: String): List<FolderUnreadChange> =
    responseJson
        .findJsonArrayProperty("folder_counts")
        ?.jsonArrayElements()
        ?.mapNotNull { item ->
            val accountId = item.findJsonStringProperty("account_id").orEmpty()
            val folderId = item.findJsonStringProperty("folder_id").orEmpty()
            val unread = item.findJsonLongProperty("unread")?.toInt() ?: return@mapNotNull null
            if (accountId.isBlank() || folderId.isBlank()) null else FolderUnreadChange(accountId, folderId, unread)
        }.orEmpty()

fun parseAccountListResponse(responseJson: String): List<AccountSummary> = decodeAccountListResponse(responseJson)

/** The app-wide proxy from an `app.proxyGet` response; missing means none. */
fun parseProxyResponse(responseJson: String): ProxySpec = parseProxySpec(responseJson.findJsonPropertyValue("proxy"), ProxySpec.off)

fun parseFolderListResponse(responseJson: String): List<FolderSummary> = decodeFolderListResponse(responseJson)

fun parseFolderDeleteResponse(responseJson: String): FolderDeleteResult = decodeFolderDeleteResponse(responseJson)

/** Every folder a rename moved, old name to new, the folder's subtree included. */
fun parseFolderRenameResponse(responseJson: String): Map<String, String> = decodeFolderRenameResponse(responseJson)

fun parseContactSuggestResponse(responseJson: String): List<ContactSuggestion> = decodeContactSuggestResponse(responseJson)

fun parseAutodiscoverResponse(responseJson: String): DiscoveredAccountSettings = decodeAutodiscoverResponse(responseJson)

fun parseProbeCertResponse(responseJson: String): ServerCertificate? = decodeProbeCertResponse(responseJson)

fun parseThreadListPage(responseJson: String): ThreadListPage = decodeThreadListPage(responseJson)

fun parseThreadListResponse(responseJson: String): List<ThreadSummary> = parseThreadListPage(responseJson).threads

/** Contact history uses the mailbox card shape and an opaque paging cursor. */
fun parseContactThreadsPage(responseJson: String): ThreadListPage = parseThreadListPage(responseJson)

fun parseStarredItemsPage(responseJson: String): StarredItemsPage = decodeStarredItemsPage(responseJson)

fun parseStarredItemsResponse(responseJson: String): List<StarredItemSummary> = decodeStarredItemsResponse(responseJson)

fun parseThreadReadPage(responseJson: String): ThreadReadPage = decodeThreadReadPage(responseJson)

fun parseThreadReadResponse(responseJson: String): List<MessageBody> = parseThreadReadPage(responseJson).messages

fun parseOpmlExportResponse(responseJson: String): String = responseJson.findJsonStringProperty("opml").orEmpty()

fun parseOpmlImportCountResponse(responseJson: String): Int = responseJson.findJsonLongProperty("imported")?.toInt() ?: 0

/** The serialized backup document from `backup.export`. */
fun parseBackupExportResponse(responseJson: String): String = responseJson.findJsonStringProperty("backup").orEmpty()

/**
 * What a restore did, or that the file is encrypted and needs a passphrase.
 * [needsPassphrase] is not an error: the host prompts and calls again.
 */
data class BackupImportResult(
    val needsPassphrase: Boolean = false,
    val accounts: Int = 0,
    /** Accounts already present locally, which a restore never overwrites. */
    val skipped: Int = 0,
    val feeds: Int = 0,
    val settings: Int = 0,
    val secrets: Int = 0,
)

fun parseBackupImportResponse(responseJson: String): BackupImportResult {
    if (responseJson.findJsonBooleanProperty("needs_passphrase") == true) {
        return BackupImportResult(needsPassphrase = true)
    }
    return BackupImportResult(
        accounts = responseJson.findJsonLongProperty("accounts")?.toInt() ?: 0,
        skipped = responseJson.findJsonLongProperty("skipped")?.toInt() ?: 0,
        feeds = responseJson.findJsonLongProperty("feeds")?.toInt() ?: 0,
        settings = responseJson.findJsonLongProperty("settings")?.toInt() ?: 0,
        secrets = responseJson.findJsonLongProperty("secrets")?.toInt() ?: 0,
    )
}

/**
 * Reads the stored mobile preferences from app.prefsGet.
 *
 * The cache accepts strings, booleans, long integers and string lists. Unsupported
 * values are skipped. An absent key lets the host keep its cached value.
 */
fun parseAppPrefsResponse(responseJson: String): Map<String, Any> {
    val objectJson = responseJson.findJsonObjectProperty("prefs") ?: return emptyMap()
    val out = LinkedHashMap<String, Any>()
    for ((key, raw) in objectJson.jsonObjectEntries()) {
        val value: Any =
            when {
                raw.startsWith('"') -> {
                    raw.readResponseString(0).value
                }

                raw == "true" -> {
                    true
                }

                raw == "false" -> {
                    false
                }

                raw.startsWith('[') -> {
                    raw
                        .jsonArrayElements()
                        .filter { it.startsWith('"') }
                        .map { it.readResponseString(0).value }
                }

                raw.toLongOrNull() != null -> {
                    raw.toLong()
                }

                // null, a nested object, or a float: nothing the pref store takes.
                else -> {
                    continue
                }
            }
        out[key] = value
    }
    return out
}

/** Encode one setting value for `AppPrefsSetParams`. */
fun encodeAppPrefValue(value: Any): String =
    when (value) {
        is String -> {
            value.jsonString()
        }

        is Boolean -> {
            value.toString()
        }

        is Int, is Long -> {
            value.toString()
        }

        is Collection<*> -> {
            value
                .filterIsInstance<String>()
                .joinToString(separator = ",", prefix = "[", postfix = "]") { it.jsonString() }
        }

        // Keep the payload well-formed rather than emitting a raw toString()
        // the reader would reject.
        else -> {
            value.toString().jsonString()
        }
    }

fun parseMediaFileUrlResponse(responseJson: String): String = responseJson.findJsonStringProperty("url").orEmpty()

/** The authorization URL from `oauth.authorizeUrl`; throws on a core error or none. */
fun parseOAuthAuthorizeUrlResponse(responseJson: String): String =
    requireCoreOk(responseJson).findJsonStringProperty("url")?.takeIf { it.isNotBlank() }
        ?: error("OAuth authorization URL missing")

fun parseAttachmentDataResponse(responseJson: String): String = responseJson.findJsonStringProperty("data").orEmpty()

fun parseSearchFieldsResponse(responseJson: String): SearchFields =
    SearchFields(
        from = responseJson.findJsonStringProperty("from").orEmpty(),
        to = responseJson.findJsonStringProperty("to").orEmpty(),
        subject = responseJson.findJsonStringProperty("subject").orEmpty(),
        tags = responseJson.findJsonStringProperty("tags").orEmpty(),
        words = responseJson.findJsonStringProperty("words").orEmpty(),
        after = responseJson.findJsonStringProperty("after").orEmpty(),
        before = responseJson.findJsonStringProperty("before").orEmpty(),
        unread = responseJson.findJsonBooleanProperty("unread") == true,
        starred = responseJson.findJsonBooleanProperty("starred") == true,
        hasAttachment = responseJson.findJsonBooleanProperty("has_attachment") == true,
    )

fun parseSearchBuildResponse(responseJson: String): String = responseJson.findJsonStringProperty("query").orEmpty()

fun parseChangelogResponse(responseJson: String): List<ChangelogRelease> {
    val releasesJson = responseJson.findJsonArrayProperty("releases") ?: return emptyList()
    return releasesJson.jsonArrayElements().mapNotNull { item ->
        val tag = item.findJsonStringProperty("tag").orEmpty()
        val version = item.findJsonStringProperty("version").orEmpty()
        if (tag.isBlank() && version.isBlank()) return@mapNotNull null
        val notes =
            item
                .findJsonArrayProperty("notes")
                ?.jsonArrayElements()
                ?.map { note -> if (note.startsWith('"')) note.readResponseString(0).value else note }
                ?.filter { it.isNotBlank() }
                .orEmpty()
        ChangelogRelease(
            version = version,
            tag = tag,
            date = item.findJsonStringProperty("date").orEmpty(),
            notes = notes,
        )
    }
}

fun parseStorageUsageResponse(responseJson: String): StorageUsage =
    StorageUsage(
        cacheBytes = responseJson.findJsonLongProperty("cacheBytes") ?: 0,
        dbBytes = responseJson.findJsonLongProperty("dbBytes") ?: 0,
    )

/** The tags a `mail.setTags` answer says the thread now carries. */
fun parseSetTagsResponse(responseJson: String): List<String> = responseJson.topLevelStringArray("tags")

/** Every tag in use, as `mail.tagList` answers, most used first. */
fun parseTagListResponse(responseJson: String): List<String> = parseTagCountsResponse(responseJson).map { it.tag }

data class TagCount(
    val tag: String,
    val count: Long,
)

/** Every tag in use and how many messages carry it, as `mail.tagList` answers, most used first. */
fun parseTagCountsResponse(responseJson: String): List<TagCount> =
    responseJson
        .jsonObjectEntries()
        .firstOrNull { it.first == "tags" }
        ?.second
        ?.takeIf { it.startsWith("[") }
        ?.jsonArrayElements()
        ?.mapNotNull { entry ->
            entry.findJsonStringProperty("tag")?.let { TagCount(it, entry.findJsonLongProperty("count") ?: 0) }
        }?.sortedByDescending { it.count }
        .orEmpty()

/** The name a `mail.replaceTag` answer kept: null after a delete. */
fun parseReplaceTagResponse(responseJson: String): String? = responseJson.findJsonStringProperty("tag")

/** The saved searches sent with a `mail.replaceTag`, in order, as they now read. */
fun parseReplacedQueries(responseJson: String): List<String> = responseJson.topLevelStringArray("queries")

/** The rules a `mail.rules` or `mail.setRules` answer lists. */
fun parseRulesResponse(responseJson: String): List<Rule> =
    responseJson
        .jsonObjectEntries()
        .firstOrNull { it.first == "rules" }
        ?.second
        ?.takeIf { it.startsWith("[") }
        ?.jsonArrayElements()
        ?.map { rule ->
            Rule(
                id = rule.findJsonStringProperty("id").orEmpty(),
                query = rule.findJsonStringProperty("query").orEmpty(),
                tags = rule.topLevelStringArray("tags"),
                markRead = rule.findJsonBooleanProperty("mark_read") ?: false,
                star = rule.findJsonBooleanProperty("star") ?: false,
                moveTo = rule.findJsonStringProperty("move_to").orEmpty(),
                folder = rule.findJsonStringProperty("folder").orEmpty(),
                folderAccount = rule.findJsonStringProperty("folder_account").orEmpty(),
                accounts = rule.topLevelStringArray("accounts"),
                copyFolder = rule.findJsonStringProperty("copy_folder").orEmpty(),
                copyAccount = rule.findJsonStringProperty("copy_account").orEmpty(),
                disabled = rule.findJsonBooleanProperty("disabled") ?: false,
            )
        }.orEmpty()

fun parseRuleStatusResponse(responseJson: String): RuleStatus {
    val entries = responseJson.jsonObjectEntries()

    fun property(name: String) = entries.firstOrNull { it.first == name }?.second
    return RuleStatus(
        log =
            property("log")
                ?.takeIf { it.startsWith("[") }
                ?.jsonArrayElements()
                ?.map { entry ->
                    RuleLogEntry(
                        at = entry.findJsonLongProperty("at") ?: 0,
                        ruleId = entry.findJsonStringProperty("rule_id").orEmpty(),
                        subject = entry.findJsonStringProperty("subject").orEmpty(),
                        count = entry.findJsonLongProperty("count")?.toInt() ?: 1,
                        failed = entry.findJsonBooleanProperty("failed") ?: false,
                        detail = entry.findJsonStringProperty("detail").orEmpty(),
                    )
                }.orEmpty(),
        running =
            property("running")
                ?.takeIf { it.startsWith("{") }
                ?.jsonObjectEntries()
                ?.associate { (id, left) -> id to (left.toIntOrNull() ?: 0) }
                .orEmpty(),
    )
}

/** The `count` answering `mail.ruleMatches` and `mail.runRule`. */
fun parseRuleCountResponse(responseJson: String): Int = responseJson.findJsonLongProperty("count")?.toInt() ?: 0

// ---- AI sorting ---------------------------------------------------------------
// Read by top-level fields only: a log row's subject or a folder note may itself
// contain text that looks like a field name.

/** `mail.aiSortConfig`: the setup, which providers have a key, and the last error. */
fun parseAiSortSetupResponse(responseJson: String): AiSortSetup = decodeAiSortSetupResponse(responseJson)

/** `mail.setAiSortConfig`: the setup as the core kept it. */
fun parseAiSortConfigResponse(responseJson: String): AiSortConfig = parseAiSortConfig(responseJson.responseFields().objectField("config"))

/** `mail.setAiSortKey`: whether a key is now saved, and its hint. */
fun parseAiSortKeyResponse(responseJson: String): AiSortKeyState = parseAiSortKeyState(responseJson.responseFields())

fun parseAiSortKeyCheckResponse(responseJson: String): AiSortKeyCheck = decodeAiSortKeyCheckResponse(responseJson)

fun parseAiModelsResponse(responseJson: String): AiModelList = decodeAiModelsResponse(responseJson)

fun parseAiSortEstimateResponse(responseJson: String): AiSortEstimate = decodeAiSortEstimateResponse(responseJson)

fun parseAiSortStatusResponse(responseJson: String): AiSortStatus = decodeAiSortStatusResponse(responseJson)

/** The `lessons` answering `mail.aiSortLessons` and `mail.aiSortForgetLesson`, newest first. */
fun parseAiSortLessonsResponse(responseJson: String): List<AiSortLesson> = decodeAiSortLessonsResponse(responseJson)

/** The `count` answering `mail.runAiSort`. */
fun parseAiSortRunResponse(responseJson: String): Int = responseJson.responseFields().int("count") ?: 0

/** The `ok` answering `mail.stopAiSort` and `mail.aiSortMoveBack`. */
fun parseAiSortOkResponse(responseJson: String): Boolean = responseJson.responseFields().bool("ok") ?: false

fun parseCalendarEventsResponse(responseJson: String): List<CalendarItem> = decodeCalendarEventsResponse(responseJson)

/** The calendar event id an `invite.addToCalendar` filed, or empty. */
fun parseCalendarEventIdResponse(responseJson: String): String = responseJson.findJsonStringProperty("event_id").orEmpty()

/** The raw `restore` object a `calendar.delete` returned, or null. Not parsed:
 *  it goes back to the core untouched. */
fun parseCalendarRestorePayload(responseJson: String): String? = responseJson.findJsonPropertyValue("restore")?.takeIf { it.startsWith('{') }

// ---- Tasks ------------------------------------------------------------------

fun parseTaskListsResponse(responseJson: String): TaskListsResponse = decodeTaskListsResponse(responseJson)

fun parseTasksResponse(responseJson: String): List<TaskSummary> = decodeTasksResponse(responseJson)

fun parseClearedTaskCount(responseJson: String): Int = responseJson.findJsonLongProperty("removed")?.toInt() ?: 0

/**
 * The raw `restore` object a delete returned, or null when there is nothing to
 * put back. Deliberately not parsed into fields: the caller's only job is to
 * hand it back, and re-encoding it would be a chance to lose something.
 */
fun parseTaskRestorePayload(responseJson: String): String? = responseJson.findJsonPropertyValue("restore")?.takeIf { it.startsWith('{') }

fun parseSenderImageResponse(responseJson: String): String = responseJson.findJsonStringProperty("src").orEmpty()

/** The `folders` list (A to Z) that every Meron folder command answers with;
 *  null when the answer has none. */
fun parseMeronFolderList(responseJson: String): List<MeronFolderSummary>? = decodeMeronFolderList(responseJson)

/** The Inbox thread ids of conversations a move out of Meron folders took
 *  out of the built-in Archive (un-archived on the server): an Undo archives
 *  those again. */
fun parseMeronUnarchived(responseJson: String): List<String> = decodeMeronUnarchived(responseJson)

/** The `folder` a create or rename answers with. */
fun parseMeronFolderResponse(responseJson: String): MeronFolderSummary? = responseJson.responseFields()["folder"]?.takeIf { it.startsWith('{') }?.let(::parseMeronFolder)

fun parseMeronFolderMoveResponse(responseJson: String): MeronFolderMoveResult = decodeMeronFolderMoveResponse(responseJson)
