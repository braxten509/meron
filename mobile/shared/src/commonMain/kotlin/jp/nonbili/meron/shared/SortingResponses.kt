// Settings, model information and progress for inbox sorting on mobile.
// The parsers read each response object at its own nesting level.
// Model and error state feed the setup and sorting history screens.
package jp.nonbili.meron.shared

internal fun parseAiSortConfig(fields: Map<String, String>?): AiSortConfig {
    if (fields == null) return AiSortConfig()
    return AiSortConfig(
        enabled = fields.bool("enabled") ?: false,
        provider = fields.stringField("provider") ?: "openrouter",
        model = fields.stringField("model").orEmpty(),
        reads = fields.stringField("reads") ?: "headers",
        folders =
            fields.list("folders").map { item ->
                val folder = item.fields()
                AiSortFolder(
                    account = folder.stringField("account").orEmpty(),
                    folder = folder.stringField("folder").orEmpty(),
                    note = folder.stringField("note").orEmpty(),
                    // Kept, or saving the settings here would drop Meron entries.
                    meronFolderId = folder.stringField("meron_folder_id").orEmpty(),
                )
            },
        recentModels =
            fields.objectField("recent_models")?.mapValues { (_, ids) ->
                ids.takeIf { it.startsWith('[') }?.jsonArrayElements().orEmpty().mapNotNull { id ->
                    id.takeIf { it.startsWith('"') }?.readResponseString(0)?.value
                }
            }.orEmpty(),
    )
}

private fun parseAiSortError(fields: Map<String, String>?): AiSortError? =
    fields?.let {
        AiSortError(
            at = it.number("at")?.toLong() ?: 0,
            message = it.stringField("message").orEmpty(),
            kind = it.stringField("kind") ?: "other",
        )
    }

internal fun parseAiSortKeyState(fields: Map<String, String>?): AiSortKeyState =
    AiSortKeyState(
        hasKey = fields?.bool("has_key") ?: false,
        hint = fields?.stringField("key_hint").orEmpty(),
    )

internal fun decodeAiSortSetupResponse(responseJson: String): AiSortSetup {
    val fields = responseJson.responseFields()
    return AiSortSetup(
        config = parseAiSortConfig(fields.objectField("config")),
        keys = fields.objectField("keys")?.mapValues { (_, value) -> parseAiSortKeyState(value.fields()) }.orEmpty(),
        lastError = parseAiSortError(fields.objectField("last_error")),
    )
}

internal fun decodeAiSortKeyCheckResponse(responseJson: String): AiSortKeyCheck {
    val fields = responseJson.responseFields()
    return AiSortKeyCheck(
        ok = fields.bool("ok") ?: false,
        kind = fields.stringField("kind").orEmpty(),
        message = fields.stringField("message").orEmpty(),
    )
}

internal fun decodeAiModelsResponse(responseJson: String): AiModelList {
    val fields = responseJson.responseFields()
    return AiModelList(
        models =
            fields.list("models").mapNotNull { item ->
                val model = item.fields()
                val id = model.stringField("id")?.takeIf { it.isNotEmpty() } ?: return@mapNotNull null
                AiModel(
                    id = id,
                    name = model.stringField("name")?.takeIf { it.isNotBlank() } ?: id,
                    priceInPerM = model.number("price_in_per_m"),
                    priceOutPerM = model.number("price_out_per_m"),
                    structured = model.bool("structured") ?: false,
                    reasoning = model.bool("reasoning") ?: false,
                    decision = model.bool("decision") ?: false,
                )
            },
        default = fields.stringField("default")?.takeIf { it.isNotEmpty() },
    )
}

internal fun decodeAiSortEstimateResponse(responseJson: String): AiSortEstimate {
    val fields = responseJson.responseFields()
    return AiSortEstimate(
        total = fields.int("total") ?: 0,
        requests = fields.int("requests") ?: 0,
        reads = fields.stringField("reads") ?: "headers",
        provider = fields.stringField("provider").orEmpty(),
        model = fields.stringField("model").orEmpty(),
        costUsd = fields.number("cost_usd"),
        accounts =
            fields.list("accounts").associate { item ->
                val account = item.fields()
                account.stringField("account").orEmpty() to (account.int("count") ?: 0)
            },
    )
}

internal fun decodeAiSortStatusResponse(responseJson: String): AiSortStatus {
    val fields = responseJson.responseFields()
    return AiSortStatus(
        log =
            fields.list("log").mapNotNull { item ->
                val entry = item.fields()
                AiSortLogEntry(
                    id = entry.number("id")?.toLong() ?: return@mapNotNull null,
                    at = entry.number("at")?.toLong() ?: 0,
                    account = entry.stringField("account").orEmpty(),
                    subject = entry.stringField("subject").orEmpty(),
                    sender = entry.stringField("sender").orEmpty(),
                    toFolder = entry.stringField("to_folder").orEmpty(),
                    reason = entry.stringField("reason").orEmpty(),
                    state = entry.stringField("state").orEmpty(),
                    detail = entry.stringField("detail").orEmpty(),
                    meronFolderId = entry.stringField("meron_folder_id").orEmpty(),
                )
            },
        running = fields.objectField("running")?.let { (it.int("total") ?: 0) to (it.int("done") ?: 0) },
        lastError = parseAiSortError(fields.objectField("last_error")),
    )
}

internal fun decodeAiSortLessonsResponse(responseJson: String): List<AiSortLesson> =
    responseJson.responseFields().list("lessons").mapNotNull { item ->
        val lesson = item.fields()
        AiSortLesson(
            id = lesson.number("id")?.toLong() ?: return@mapNotNull null,
            at = lesson.number("at")?.toLong() ?: 0,
            account = lesson.stringField("account").orEmpty(),
            sender = lesson.stringField("sender").orEmpty(),
            subject = lesson.stringField("subject").orEmpty(),
            place = lesson.stringField("place").orEmpty(),
            placeName = lesson.stringField("place_name").orEmpty(),
            used = lesson.bool("used") ?: true,
        )
    }
