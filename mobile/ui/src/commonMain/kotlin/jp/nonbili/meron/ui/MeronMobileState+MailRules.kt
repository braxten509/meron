// Mobile mailbox state changes for mail rules.
// Called by the Compose routes and their thread actions.
// Server responses are reconciled with the local mailbox state here.
package jp.nonbili.meron.ui

import androidx.compose.ui.input.key.key
import jp.nonbili.meron.shared.AiModelList
import jp.nonbili.meron.shared.AiSortConfig
import jp.nonbili.meron.shared.AiSortEstimate
import jp.nonbili.meron.shared.AiSortKeyCheck
import jp.nonbili.meron.shared.AiSortKeyState
import jp.nonbili.meron.shared.AiSortLesson
import jp.nonbili.meron.shared.AiSortSetup
import jp.nonbili.meron.shared.AiSortStatus
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.Rule
import jp.nonbili.meron.shared.RuleStatus
import jp.nonbili.meron.shared.parseAiModelsResponse
import jp.nonbili.meron.shared.parseAiSortConfigResponse
import jp.nonbili.meron.shared.parseAiSortEstimateResponse
import jp.nonbili.meron.shared.parseAiSortKeyCheckResponse
import jp.nonbili.meron.shared.parseAiSortKeyResponse
import jp.nonbili.meron.shared.parseAiSortLessonsResponse
import jp.nonbili.meron.shared.parseAiSortOkResponse
import jp.nonbili.meron.shared.parseAiSortRunResponse
import jp.nonbili.meron.shared.parseAiSortSetupResponse
import jp.nonbili.meron.shared.parseAiSortStatusResponse
import jp.nonbili.meron.shared.parseRuleCountResponse
import jp.nonbili.meron.shared.parseRuleStatusResponse
import jp.nonbili.meron.shared.parseRulesResponse
import jp.nonbili.meron.shared.parseTagListResponse
import jp.nonbili.meron.shared.requireCoreOk
import kotlinx.coroutines.withContext

/** The saved rules, or null if they could not be read. */
internal suspend fun MeronMobileState.loadRules(): List<Rule>? =
    runCatching {
        parseRulesResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).rules() }))
    }.getOrNull()

/** The rules as the core kept them, or null if it refused one. */
internal suspend fun MeronMobileState.saveRules(rules: List<Rule>): List<Rule>? =
    runCatching {
        parseRulesResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).setRules(rules) }))
    }.getOrNull()

internal suspend fun MeronMobileState.ruleMatches(id: String): Int? =
    runCatching {
        parseRuleCountResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).ruleMatches(id) }))
    }.getOrNull()

internal suspend fun MeronMobileState.runRule(id: String): Int? =
    runCatching {
        parseRuleCountResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).runRule(id) }))
    }.getOrNull()

internal suspend fun MeronMobileState.loadRuleStatus(): RuleStatus? =
    runCatching {
        parseRuleStatusResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).ruleStatus() }))
    }.getOrNull()

internal suspend fun MeronMobileState.stopRule(id: String): Boolean = runCatching { requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).stopRule(id) }) }.isSuccess

// AI sorting. None of these log anything: the key and the provider's answers
// must never reach a log.

internal suspend fun MeronMobileState.loadAiSortSetup(): AiSortSetup? =
    runCatching {
        parseAiSortSetupResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).aiSortConfig() }))
    }.getOrNull()

/** The setup as the core kept it, or why it refused it. */
internal suspend fun MeronMobileState.saveAiSortConfig(config: AiSortConfig): Result<AiSortConfig> =
    runCatching {
        parseAiSortConfigResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).setAiSortConfig(config) }))
    }

/** Saves [key] for [provider] (empty removes it), or says why the core refused. */
internal suspend fun MeronMobileState.saveAiSortKey(
    provider: String,
    key: String,
): Result<AiSortKeyState> =
    runCatching {
        parseAiSortKeyResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).setAiSortKey(provider, key) }))
    }

internal suspend fun MeronMobileState.checkAiSortKey(provider: String): AiSortKeyCheck =
    runCatching {
        parseAiSortKeyCheckResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).checkAiSortKey(provider) }))
    }.getOrElse { AiSortKeyCheck(ok = false, kind = "other", message = it.message.orEmpty()) }

internal suspend fun MeronMobileState.loadAiModels(provider: String): AiModelList? =
    runCatching {
        parseAiModelsResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).aiSortModels(provider) }))
    }.getOrNull()

internal suspend fun MeronMobileState.loadAiSortEstimate(): AiSortEstimate? =
    runCatching {
        parseAiSortEstimateResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).aiSortEstimate() }))
    }.getOrNull()

/** How many emails the "sort Inbox" job took on, or null if it didn't start. */
internal suspend fun MeronMobileState.runAiSort(): Int? =
    runCatching {
        parseAiSortRunResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).runAiSort() }))
    }.getOrNull()

internal suspend fun MeronMobileState.stopAiSort(): Boolean = runCatching { requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).stopAiSort() }) }.isSuccess

internal suspend fun MeronMobileState.loadAiSortStatus(): AiSortStatus? =
    runCatching {
        parseAiSortStatusResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).aiSortStatus() }))
    }.getOrNull()

internal suspend fun MeronMobileState.aiSortMoveBack(id: Long): Boolean =
    runCatching {
        parseAiSortOkResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).aiSortMoveBack(id) }))
    }.getOrDefault(false)

/** What AI sorting learned from mail moved by hand, newest first; null if it couldn't be read. */
internal suspend fun MeronMobileState.loadAiSortLessons(): List<AiSortLesson>? =
    runCatching {
        parseAiSortLessonsResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).aiSortLessons() }))
    }.getOrNull()

/** Forgets one lesson; the fresh list, or null if it failed. */
internal suspend fun MeronMobileState.aiSortForgetLesson(id: Long): List<AiSortLesson>? =
    runCatching {
        parseAiSortLessonsResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).aiSortForgetLesson(id) }))
    }.getOrNull()

internal suspend fun MeronMobileState.loadKnownTags(): List<String> =
    runCatching {
        parseTagListResponse(requireCoreOk(withContext(ioDispatcher) { MobileMailCommandClient(core).tagList() }))
    }.getOrDefault(emptyList())
