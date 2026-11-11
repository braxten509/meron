package jp.nonbili.meron.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.filled.Undo
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.History
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.AiModel
import jp.nonbili.meron.shared.AiModelList
import jp.nonbili.meron.shared.AiSortConfig
import jp.nonbili.meron.shared.AiSortError
import jp.nonbili.meron.shared.AiSortEstimate
import jp.nonbili.meron.shared.AiSortFolder
import jp.nonbili.meron.shared.AiSortKeyState
import jp.nonbili.meron.shared.AiSortLesson
import jp.nonbili.meron.shared.AiSortLogEntry
import jp.nonbili.meron.shared.AiSortStatus
import jp.nonbili.meron.shared.accountSummaryIsRss
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlin.math.pow
import kotlin.math.roundToLong

private const val PROVIDER_TINFOIL = "tinfoil"
private const val PROVIDER_OPENROUTER = "openrouter"
private const val READS_HEADERS = "headers"
private const val READS_START = "start"
private const val AI_POLL_MS = 1500L
/** History rows shown at first and added by each "View more". */
private const val AI_LOG_PAGE = 50

/** The core refuses these as targets too; the picker just doesn't offer them. */
private val SYSTEM_FOLDER_ROLES = setOf("inbox", "sent", "drafts", "trash", "junk")

// Rough size of one new email's request, as in the plan's cost table: about
// 300 tokens in for sender and subject, 125 more with the start of the email,
// and 30 out.
private const val TOKENS_IN_HEADERS = 300.0
private const val TOKENS_IN_START = 425.0
private const val TOKENS_OUT = 30.0

// A decision model reads one email and the folder list per request, and bills
// no output.
private const val DECISION_TOKENS_IN_HEADERS = 150.0
private const val DECISION_TOKENS_IN_START = 275.0

/** About what 1,000 new emails cost with [model], or null when its price is unknown. */
private fun costPerThousand(
    model: AiModel,
    reads: String,
): Double? {
    val priceIn = model.priceInPerM ?: return null
    if (model.decision) {
        val tokensIn = if (reads == READS_START) DECISION_TOKENS_IN_START else DECISION_TOKENS_IN_HEADERS
        return 1000 * tokensIn * priceIn / 1_000_000
    }
    val priceOut = model.priceOutPerM ?: return null
    val tokensIn = if (reads == READS_START) TOKENS_IN_START else TOKENS_IN_HEADERS
    return 1000 * (tokensIn * priceIn + TOKENS_OUT * priceOut) / 1_000_000
}

/** US dollars with two significant digits below a dollar: $0.042, $0.50, $1.20. */
internal fun formatUsd(value: Double): String {
    if (value <= 0.0) return "\$0"
    var decimals = 2
    if (value < 1) {
        var scaled = value
        decimals = 0
        while (scaled < 10 && decimals < 8) {
            scaled *= 10
            decimals += 1
        }
        decimals = maxOf(decimals, 2)
    }
    val factor = 10.0.pow(decimals).toLong()
    val units = (value * factor).roundToLong()
    val fraction = (units % factor).toString().padStart(decimals, '0')
    val trimmed = if (decimals > 2) fraction.trimEnd('0').padEnd(2, '0') else fraction
    return "\$${units / factor}.$trimmed"
}

private fun accountName(account: AccountSummary): String = account.displayName.ifEmpty { account.email }

private fun folderKey(folder: AiSortFolder): String =
    if (folder.isMeron) "meron\u0000${folder.meronFolderId}" else "${folder.account}\u0000${folder.folder}"

@Composable
private fun errorLabel(error: AiSortError): String =
    when (error.kind) {
        "key" -> tr("aiSort.errorKey")
        "credit" -> tr("aiSort.errorCredit")
        "verify" -> tr("aiSort.errorVerify")
        else -> tr("aiSort.errorOther")
    }

@Composable
private fun providerLabel(provider: String): String =
    if (provider == PROVIDER_TINFOIL) tr("aiSort.providerTinfoil") else tr("aiSort.providerOpenRouter")

@Composable
private fun SectionTitle(text: String) {
    Text(text, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(top = 8.dp))
}

@Composable
private fun SmallNote(
    text: String,
    color: Color = MaterialTheme.colorScheme.onSurfaceVariant,
) {
    Text(text, fontSize = 12.sp, color = color)
}

@Composable
private fun ChoiceRow(
    label: String,
    selected: Boolean,
    enabled: Boolean,
    onSelect: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth().clickable(enabled = enabled, onClick = onSelect).padding(vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RadioButton(selected = selected, enabled = enabled, onClick = onSelect)
        Text(label, modifier = Modifier.padding(start = 8.dp))
    }
}

/**
 * AI sorting: an AI picks one of the owner's folders for each new email before
 * it shows in the Inbox. The key goes to the core once and is never read back;
 * the page only ever sees whether one is saved and its last few characters.
 */
@Composable
internal fun MeronMobileState.AiSortPage(
    modifier: Modifier = Modifier,
    onOpenLog: () -> Unit = {},
) {
    // Null until loaded: nothing may be saved over a setup not yet read.
    var config by remember { mutableStateOf<AiSortConfig?>(null) }
    var keys by remember { mutableStateOf(emptyMap<String, AiSortKeyState>()) }
    var setupError by remember { mutableStateOf<AiSortError?>(null) }
    var status by remember { mutableStateOf(AiSortStatus()) }
    // Null until loaded: what the AI learned from mail moved by hand.
    var lessons by remember { mutableStateOf<List<AiSortLesson>?>(null) }
    var forgetFailed by remember { mutableStateOf(false) }
    var lessonsShown by remember { mutableIntStateOf(AI_LOG_PAGE) }
    var models by remember { mutableStateOf(emptyMap<String, AiModelList>()) }
    var modelsFailed by remember { mutableStateOf(false) }
    var picking by remember { mutableStateOf(false) }
    var keyDraft by remember { mutableStateOf("") }
    // The service whose key box is open, and what its last check said.
    var editingKeyFor by remember { mutableStateOf<String?>(null) }
    var keyCheck by remember { mutableStateOf<Pair<Boolean, AiSortError?>?>(null) }
    var keyBusy by remember { mutableStateOf(false) }
    var noteDrafts by remember { mutableStateOf(emptyMap<String, String>()) }
    var notice by remember { mutableStateOf("") }
    var noticeIsError by remember { mutableStateOf(false) }
    var confirmSort by remember { mutableStateOf<AiSortEstimate?>(null) }
    // Every change shows at once and is saved in order, one save at a time; a
    // save's answer only lands if nothing changed while it was on its way.
    var saving by remember { mutableStateOf(false) }
    var generation by remember { mutableIntStateOf(0) }
    val coroutines = rememberCoroutineScope()

    LaunchedEffect(Unit) {
        loadAiSortSetup()?.let { setup ->
            config = setup.config
            keys = setup.keys
            setupError = setup.lastError
        }
        loadAiSortStatus()?.let { status = it }
        // A failed read shows as nothing learned rather than loading forever.
        lessons = loadAiSortLessons() ?: emptyList()
        refreshMeronFolders()
    }
    val polling = status.running != null || status.log.any { it.state == "moving" }
    LaunchedEffect(polling) {
        while (polling) {
            delay(AI_POLL_MS)
            loadAiSortStatus()?.let { status = it }
        }
    }
    val mailAccounts = coreAccounts.filterNot { accountSummaryIsRss(it) || it.needsReconnect }
    LaunchedEffect(mailAccounts.size) { mailAccounts.forEach { ensureAccountFolders(it.id) } }

    val current = config
    val provider = current?.provider ?: PROVIDER_OPENROUTER
    // The model list, for the chosen model's price; the picker loads it too.
    LaunchedEffect(provider, current?.model?.isNotEmpty()) {
        if (current?.model?.isNotEmpty() == true && provider !in models) {
            loadAiModels(provider)?.let { models = models + (provider to it) }
        }
    }

    fun show(
        text: String,
        error: Boolean = false,
    ) {
        notice = text
        noticeIsError = error
    }

    fun flush() {
        if (saving) return
        val sending = config ?: return
        val sent = generation
        saving = true
        coroutines.launch {
            val result = saveAiSortConfig(sending)
            saving = false
            result
                .onSuccess { if (sent == generation) config = it }
                .onFailure { error ->
                    show(error.message.orEmpty(), error = true)
                    // Show what the core really holds rather than the refused change.
                    if (sent == generation) loadAiSortSetup()?.let { config = it.config }
                }
            if (sent != generation) flush()
        }
    }

    fun update(next: AiSortConfig) {
        config = next
        generation += 1
        flush()
    }

    fun saveKey(
        forProvider: String,
        key: String,
    ) {
        if (keyBusy) return
        keyBusy = true
        coroutines.launch {
            val result = saveAiSortKey(forProvider, key)
            keyBusy = false
            result
                .onSuccess { state ->
                    keys = keys + (forProvider to state)
                    keyDraft = ""
                    editingKeyFor = null
                    keyCheck = null
                    show("")
                    // Removing the active service's key turns sorting off in the
                    // core: read the setup back so the switch matches it.
                    val before = generation
                    loadAiSortSetup()?.let { setup ->
                        keys = setup.keys
                        if (!saving && before == generation) config = setup.config
                    }
                }.onFailure { show(it.message.orEmpty(), error = true) }
        }
    }

    val pickedModel = models[provider]?.models?.firstOrNull { it.id == current?.model }
    if (picking && current != null) {
        AiModelPicker(
            list = models[provider],
            failed = modelsFailed,
            reads = current.reads,
            selected = current.model,
            recent = current.recentModels[provider].orEmpty(),
            onPick = { id ->
                picking = false
                update(current.copy(model = id))
            },
            onDismiss = { picking = false },
        )
    }

    editingKeyFor?.let { forProvider ->
        val state = keys[forProvider] ?: AiSortKeyState()
        val works = tr("aiSort.keyOk")
        fun close() {
            editingKeyFor = null
            keyDraft = ""
            keyCheck = null
        }
        AlertDialog(
            onDismissRequest = ::close,
            title = { Text(providerLabel(forProvider)) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (state.hasKey) Text(tr("aiSort.keySaved", mapOf("hint" to state.hint)), fontSize = 13.sp)
                    SetupField(
                        value = keyDraft,
                        onChange = { keyDraft = it },
                        label = tr(if (state.hasKey) "aiSort.keyReplace" else "aiSort.key"),
                        isPassword = true,
                        placeholder = tr("aiSort.keyPlaceholder"),
                    )
                    keyCheck?.let { (ok, error) ->
                        Text(
                            if (ok || error == null) works else errorLabel(error),
                            fontSize = 12.sp,
                            color = if (ok) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.error,
                        )
                    }
                    if (state.hasKey) {
                        Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                            TextButton(
                                enabled = !keyBusy,
                                onClick = {
                                    keyBusy = true
                                    coroutines.launch {
                                        val check = checkAiSortKey(forProvider)
                                        keyBusy = false
                                        keyCheck =
                                            if (check.ok) {
                                                if (forProvider == provider) setupError = null
                                                true to null
                                            } else {
                                                false to AiSortError(message = check.message, kind = check.kind.ifEmpty { "other" })
                                            }
                                    }
                                },
                            ) { Text(tr("aiSort.keyCheck")) }
                            TextButton(enabled = !keyBusy, onClick = { saveKey(forProvider, "") }) {
                                Text(tr("aiSort.keyRemove"), color = MaterialTheme.colorScheme.error)
                            }
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(
                    enabled = !keyBusy && keyDraft.isNotBlank(),
                    onClick = { saveKey(forProvider, keyDraft.trim()) },
                ) { Text(tr("buttons.save")) }
            },
            dismissButton = { TextButton(onClick = ::close) { Text(tr("buttons.cancel")) } },
        )
    }

    confirmSort?.let { estimate ->
        val args = mapOf("count" to estimate.total.toString(), "provider" to providerLabel(estimate.provider.ifEmpty { provider }))
        AlertDialog(
            onDismissRequest = { confirmSort = null },
            title = { Text(tr("aiSort.sortInbox")) },
            text = {
                Text(
                    estimate.costUsd?.let { tr("aiSort.sortInboxConfirm", args + ("cost" to formatUsd(it))) }
                        ?: tr("aiSort.sortInboxConfirmNoPrice", args),
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmSort = null
                        coroutines.launch {
                            val count = runAiSort()
                            if (count == null) {
                                show(trs("aiSort.errorOther"), error = true)
                            } else {
                                // The Sorting section shows how many are left.
                                show("")
                            }
                            loadAiSortStatus()?.let { status = it }
                        }
                    },
                ) { Text(tr("aiSort.sortInboxAction")) }
            },
            dismissButton = { TextButton(onClick = { confirmSort = null }) { Text(tr("buttons.cancel")) } },
        )
    }

    if (current == null) {
        Column(modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            SmallNote(tr("common.loading"))
        }
        return
    }
    Column(
        modifier.verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        val lastError = status.lastError ?: setupError
        if (lastError != null) {
            Surface(color = MaterialTheme.colorScheme.errorContainer, shape = MaterialTheme.shapes.small) {
                Column(Modifier.fillMaxWidth().padding(12.dp)) {
                    Text(errorLabel(lastError), color = MaterialTheme.colorScheme.onErrorContainer, fontSize = 13.sp)
                    if (lastError.kind == "other" && lastError.message.isNotBlank()) {
                        Text(
                            lastError.message,
                            color = MaterialTheme.colorScheme.onErrorContainer,
                            fontSize = 12.sp,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }
        }
        if (notice.isNotEmpty()) {
            SmallNote(notice, if (noticeIsError) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary)
        }

        // Main switch: it can always be turned off, but only on once the setup is complete.
        val keyState = keys[provider] ?: AiSortKeyState()
        val missing =
            listOfNotNull(
                tr("aiSort.needsKey").takeIf { !keyState.hasKey },
                tr("aiSort.needsModel").takeIf { current.model.isEmpty() },
                tr("aiSort.needsFolder").takeIf { current.folders.isEmpty() },
            )
        val ready = missing.isEmpty()
        SettingsToggleRow(
            icon = Icons.Filled.AutoAwesome,
            title = tr("aiSort.enabled"),
            checked = current.enabled && ready,
            enabled = ready || current.enabled,
            onToggle = { update(current.copy(enabled = !current.enabled)) },
        )

        SectionTitle(tr("aiSort.provider"))
        // One card per service with its own key, so switching is one tap; the
        // model last used there comes back with it.
        fun switchTo(option: String) {
            if (provider == option) return
            modelsFailed = false
            update(current.copy(provider = option, model = current.recentModels[option]?.firstOrNull().orEmpty()))
        }
        Surface(color = MaterialTheme.colorScheme.surface, shape = MaterialTheme.shapes.medium) {
            Column {
                listOf(PROVIDER_TINFOIL, PROVIDER_OPENROUTER).forEachIndexed { index, option ->
                    if (index > 0) HorizontalDivider()
                    val state = keys[option] ?: AiSortKeyState()
                    Row(
                        Modifier.fillMaxWidth().clickable(enabled = !keyBusy) { switchTo(option) }.padding(4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = provider == option, enabled = !keyBusy, onClick = { switchTo(option) })
                        Column(Modifier.weight(1f).padding(start = 4.dp)) {
                            Text(providerLabel(option), maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(
                                if (state.hasKey) tr("aiSort.keySaved", mapOf("hint" to state.hint)) else tr("aiSort.noKey"),
                                fontSize = 12.sp,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        TextButton(
                            enabled = !keyBusy,
                            onClick = {
                                keyDraft = ""
                                keyCheck = null
                                editingKeyFor = option
                            },
                        ) { Text(tr(if (state.hasKey) "aiSort.editKey" else "aiSort.addKey")) }
                    }
                }
            }
        }

        SectionTitle(tr("aiSort.model"))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text(
                    pickedModel?.name ?: current.model.ifEmpty { tr("common.notSelected") },
                    fontSize = 13.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    color = if (current.model.isEmpty()) MaterialTheme.colorScheme.error else Color.Unspecified,
                )
                pickedModel?.let { costPerThousand(it, current.reads) }?.let {
                    SmallNote(tr("aiSort.modelPrice", mapOf("cost" to formatUsd(it))))
                }
                if (pickedModel?.decision == true) SmallNote(tr("aiSort.modelDecision"))
                if (pickedModel?.reasoning == true) SmallNote(tr("aiSort.modelSlow"))
            }
            TextButton(
                onClick = {
                    picking = true
                    if (provider !in models) {
                        modelsFailed = false
                        coroutines.launch {
                            val list = loadAiModels(provider)
                            if (list == null) modelsFailed = true else models = models + (provider to list)
                        }
                    }
                },
            ) { Text(tr("common.change")) }
        }

        SectionTitle(tr("aiSort.reads"))
        listOf(READS_HEADERS to "aiSort.readsHeaders", READS_START to "aiSort.readsStart").forEach { (option, label) ->
            ChoiceRow(tr(label), selected = current.reads == option, enabled = true) {
                if (current.reads != option) update(current.copy(reads = option))
            }
        }

        SectionTitle(tr("aiSort.folders"))
        current.folders.forEach { folder ->
            val key = folderKey(folder)
            Column {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        aiFolderLabel(mailAccounts, folder),
                        Modifier.weight(1f, fill = false),
                        fontSize = 13.sp,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    if (folder.isMeron) {
                        Text(
                            tr("aiSort.meronFolderTag"),
                            Modifier.padding(start = 8.dp),
                            fontSize = 12.sp,
                            maxLines = 1,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Spacer(Modifier.weight(1f))
                    IconButton(onClick = {
                        noteDrafts = noteDrafts - key
                        update(current.copy(folders = current.folders.filterNot { folderKey(it) == key }))
                    }) {
                        Icon(Icons.Filled.Close, contentDescription = tr("aiSort.removeFolder"))
                    }
                }
                OutlinedTextField(
                    value = noteDrafts[key] ?: folder.note,
                    onValueChange = { noteDrafts = noteDrafts + (key to it) },
                    placeholder = { Text(tr("aiSort.notePlaceholder")) },
                    singleLine = true,
                    keyboardOptions = nativeTextKeyboardOptions,
                    modifier =
                        Modifier.fillMaxWidth().onFocusChanged { focus ->
                            val draft = noteDrafts[key]
                            if (!focus.isFocused && draft != null) {
                                noteDrafts = noteDrafts - key
                                val latest = config ?: return@onFocusChanged
                                if (latest.folders.any { folderKey(it) == key && it.note != draft }) {
                                    update(latest.copy(folders = latest.folders.map { if (folderKey(it) == key) it.copy(note = draft) else it }))
                                }
                            }
                        },
                )
            }
        }
        val chosen = current.folders.map(::folderKey).toSet()
        RuleTargetRow(
            title = "",
            value = tr("aiSort.addFolder"),
            accounts = mailAccounts,
            options = emptyList(),
            include = { it.role !in SYSTEM_FOLDER_ROLES && folderKey(AiSortFolder(it.accountId, it.name)) !in chosen },
            meronOptions = meronFolders.filterNot(::isMeronArchiveFolder),
            meronChosen = current.folders.filter { it.isMeron }.map { it.meronFolderId }.toSet(),
            onPickMeron = { meron ->
                update(current.copy(folders = current.folders + AiSortFolder(account = "", folder = "", meronFolderId = meron.id)))
            },
        ) { accountId, folder ->
            update(current.copy(folders = current.folders + AiSortFolder(account = accountId, folder = folder)))
        }

        SectionTitle(tr("aiSort.sortSection"))
        val running = status.running
        if (running != null) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    tr("aiSort.sortInboxLeft", mapOf("count" to (running.first - running.second).coerceAtLeast(0).toString())),
                    Modifier.weight(1f),
                    fontSize = 13.sp,
                )
                val failed = tr("aiSort.errorOther")
                TextButton(onClick = {
                    coroutines.launch {
                        if (!stopAiSort()) show(failed, error = true)
                        loadAiSortStatus()?.let { status = it }
                    }
                }) { Text(tr("aiSort.stop")) }
            }
        } else {
            val nothing = tr("aiSort.sortInboxNothing")
            val failed = tr("aiSort.errorOther")
            Button(
                enabled = ready,
                onClick = {
                    coroutines.launch {
                        when (val estimate = loadAiSortEstimate()) {
                            null -> show(failed, error = true)
                            else -> if (estimate.total == 0) show(nothing) else confirmSort = estimate
                        }
                    }
                },
            ) { Text(tr("aiSort.sortInbox")) }
        }

        // The history has a screen of its own; here, just the way in.
        SettingsRow(
            icon = Icons.Filled.History,
            title = tr("aiSort.log"),
            onClick = onOpenLog,
            trailing = {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (status.log.isNotEmpty()) {
                        Text(status.log.size.toString(), fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, contentDescription = null)
                }
            },
        )

        SectionTitle(tr("aiSort.lessons"))
        SmallNote(tr("aiSort.lessonsHint"))
        if (forgetFailed) SmallNote(tr("aiSort.lessonForgetFailed"), MaterialTheme.colorScheme.error)
        val learned = lessons
        if (learned == null) {
            SmallNote(tr("common.loading"))
        } else if (learned.isEmpty()) {
            SmallNote(tr("aiSort.lessonsEmpty"))
        } else {
            Column {
                learned.take(lessonsShown).forEach { lesson ->
                    AiSortLessonRow(
                        lesson = lesson,
                        onForget = {
                            coroutines.launch {
                                val fresh = aiSortForgetLesson(lesson.id)
                                forgetFailed = fresh == null
                                fresh?.let { lessons = it }
                            }
                        },
                    )
                    HorizontalDivider()
                }
                if (learned.size > lessonsShown) {
                    TextButton(onClick = { lessonsShown += AI_LOG_PAGE }, modifier = Modifier.fillMaxWidth()) {
                        Text(tr("aiSort.viewMore"))
                    }
                }
            }
        }
    }
}

/** Everything the AI moved, newest first, one line each, [AI_LOG_PAGE] at a time. */
@Composable
internal fun MeronMobileState.AiSortLogPage(modifier: Modifier = Modifier) {
    var history by remember { mutableStateOf<AiSortStatus?>(null) }
    var shown by remember { mutableIntStateOf(AI_LOG_PAGE) }
    val coroutines = rememberCoroutineScope()
    LaunchedEffect(Unit) { loadAiSortStatus()?.let { history = it } }
    val current = history
    val polling = current != null && (current.running != null || current.log.any { it.state == "moving" })
    LaunchedEffect(polling) {
        while (polling) {
            delay(AI_POLL_MS)
            loadAiSortStatus()?.let { history = it }
        }
    }
    if (current == null) {
        Column(modifier.padding(16.dp)) { SmallNote(tr("common.loading")) }
        return
    }
    val moveBackFailed = trs("aiSort.moveBackFailed")
    var failedNote by remember { mutableStateOf(false) }
    LazyColumn(modifier, contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)) {
        if (failedNote) {
            item(key = "move-back-failed") { SmallNote(moveBackFailed, MaterialTheme.colorScheme.error) }
        }
        if (current.log.isEmpty()) {
            item { SmallNote(tr("aiSort.logEmpty")) }
        }
        items(current.log.take(shown), key = { it.id }) { entry ->
            AiSortLogRow(
                entry = entry,
                onMoveBack = {
                    coroutines.launch {
                        val ok = aiSortMoveBack(entry.id)
                        failedNote = !ok
                        if (!ok) status = moveBackFailed
                        loadAiSortStatus()?.let { history = it }
                    }
                },
            )
            HorizontalDivider()
        }
        if (current.log.size > shown) {
            item {
                TextButton(onClick = { shown += AI_LOG_PAGE }, modifier = Modifier.fillMaxWidth()) {
                    Text(tr("aiSort.viewMore"))
                }
            }
        }
    }
}

/** A chosen folder's name, with its account when there is more than one. */
private fun MeronMobileState.aiFolderLabel(
    accounts: List<AccountSummary>,
    folder: AiSortFolder,
): String {
    if (folder.isMeron) return meronFolderName(folder.meronFolderId) ?: folder.meronFolderId
    val name = foldersByAccount[folder.account]?.firstOrNull { it.name == folder.folder }?.displayName ?: folder.folder
    val account = accounts.firstOrNull { it.id == folder.account }
    return if (accounts.size > 1 && account != null) "${accountName(account)} / $name" else name
}

/**
 * One line of the AI's history: the subject, a tag with the folder (or why it
 * stayed), and an undo arrow while it can still be moved back.
 */
@Composable
private fun AiSortLogRow(
    entry: AiSortLogEntry,
    onMoveBack: () -> Unit,
) {
    val moved = entry.state == "moved" || entry.state == "moving"
    val failed = entry.state == "failed"
    val tag =
        when (entry.state) {
            "moved", "moving" -> entry.toFolder
            "left" -> tr("aiSort.tagLeft")
            "slow" -> tr("aiSort.tagSlow")
            "failed" -> tr("aiSort.tagFailed")
            "moved_back" -> tr("aiSort.tagMovedBack")
            else -> entry.state
        }
    val colors = MaterialTheme.colorScheme
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(
            entry.subject.ifBlank { entry.sender },
            Modifier.weight(1f),
            fontSize = 14.sp,
            color = if (moved) Color.Unspecified else colors.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Surface(
            color = if (failed) colors.errorContainer else if (moved) colors.secondaryContainer else colors.surfaceVariant,
            shape = RoundedCornerShape(50),
            modifier = Modifier.padding(start = 8.dp).widthIn(max = 140.dp),
        ) {
            Text(
                tag,
                Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
                fontSize = 11.sp,
                color = if (failed) colors.onErrorContainer else if (moved) colors.onSecondaryContainer else colors.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        // The same room on every line, so the tags line up.
        Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) {
            if (entry.state == "moved") {
                IconButton(onClick = onMoveBack) {
                    Icon(Icons.AutoMirrored.Filled.Undo, contentDescription = tr("aiSort.moveBack"), tint = colors.primary)
                }
            }
        }
    }
}

/**
 * One thing the AI learned from mail moved by hand: the sender, the subject, a
 * tag with where it went, and a button to forget it. Dimmed, with a note,
 * while the AI can't follow it.
 */
@Composable
private fun AiSortLessonRow(
    lesson: AiSortLesson,
    onForget: () -> Unit,
) {
    val tag =
        when (lesson.place) {
            "spam" -> tr("aiSort.lessonSpam")
            "inbox" -> tr("aiSort.lessonInbox")
            else -> lesson.placeName
        }
    val colors = MaterialTheme.colorScheme
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f).padding(vertical = 4.dp)) {
            Text(
                lesson.sender,
                fontSize = 14.sp,
                color = if (lesson.used) Color.Unspecified else colors.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (lesson.subject.isNotBlank()) {
                Text(lesson.subject, fontSize = 12.sp, color = colors.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            if (!lesson.used) {
                when (lesson.place) {
                    "meron" -> SmallNote(tr("aiSort.lessonUnusedMeron"))
                    "spam" -> SmallNote(tr("aiSort.lessonUnusedSpam"))
                }
            }
        }
        Surface(
            color = if (lesson.used) colors.secondaryContainer else colors.surfaceVariant,
            shape = RoundedCornerShape(50),
            modifier = Modifier.padding(start = 8.dp).widthIn(max = 140.dp),
        ) {
            Text(
                tag,
                Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
                fontSize = 11.sp,
                color = if (lesson.used) colors.onSecondaryContainer else colors.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) {
            IconButton(onClick = onForget) {
                Icon(Icons.Filled.Close, contentDescription = tr("aiSort.lessonForget"), tint = colors.onSurfaceVariant)
            }
        }
    }
}

/**
 * Full-screen model list with a search field. Each row shows about what 1,000
 * new emails cost and flags thinking models, which may be too slow. If the list
 * can't load, a model id can still be typed in.
 */
@Composable
private fun AiModelPicker(
    list: AiModelList?,
    failed: Boolean,
    reads: String,
    selected: String,
    recent: List<String>,
    onPick: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    var query by remember { mutableStateOf("") }
    var typed by remember { mutableStateOf(selected) }
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.surface) {
            Column(Modifier.fillMaxSize()) {
                Row(Modifier.padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
                    IconButton(onClick = onDismiss) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = tr("buttons.back"))
                    }
                    Text(tr("aiSort.model"), style = MaterialTheme.typography.titleLarge)
                }
                when {
                    list != null -> {
                        OutlinedTextField(
                            value = query,
                            onValueChange = { query = it },
                            placeholder = { Text(tr("kanban.actions.search")) },
                            singleLine = true,
                            keyboardOptions = nativeTextKeyboardOptions,
                            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
                        )
                        val needle = query.trim()
                        val shown =
                            list.models.filter {
                                needle.isEmpty() || it.name.contains(needle, ignoreCase = true) || it.id.contains(needle, ignoreCase = true)
                            }
                        // The last few picked, on top while not searching.
                        val recentShown =
                            if (needle.isEmpty()) recent.map { id -> list.models.firstOrNull { it.id == id } ?: AiModel(id) } else emptyList()
                        LazyColumn(Modifier.fillMaxSize()) {
                            if (recentShown.isNotEmpty()) {
                                item(key = "recent-title") { ModelListTitle(tr("aiSort.recentModels")) }
                                items(recentShown, key = { "recent:${it.id}" }) { model ->
                                    AiModelRow(model, reads, model.id == selected) { onPick(model.id) }
                                }
                                item(key = "all-title") { ModelListTitle(tr("aiSort.allModels")) }
                            }
                            items(shown, key = { it.id }) { model ->
                                AiModelRow(model, reads, model.id == selected) { onPick(model.id) }
                            }
                        }
                    }

                    failed -> {
                        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            SmallNote(tr("aiSort.modelsFailed"), MaterialTheme.colorScheme.error)
                            OutlinedTextField(
                                value = typed,
                                onValueChange = { typed = it },
                                label = { Text(tr("aiSort.model")) },
                                singleLine = true,
                                keyboardOptions = nativeTextKeyboardOptions,
                                modifier = Modifier.fillMaxWidth(),
                            )
                            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                                TextButton(enabled = typed.isNotBlank(), onClick = { onPick(typed.trim()) }) { Text(tr("buttons.save")) }
                            }
                        }
                    }

                    else -> {
                        SmallNote(tr("common.loading"))
                    }
                }
            }
        }
    }
}

@Composable
private fun ModelListTitle(text: String) {
    Text(
        text,
        Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 4.dp),
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.primary,
    )
}

/** One model: its name, id and about what 1,000 emails cost. */
@Composable
private fun AiModelRow(
    model: AiModel,
    reads: String,
    selected: Boolean,
    onPick: () -> Unit,
) {
    Column(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onPick)
            .padding(horizontal = 16.dp, vertical = 10.dp),
    ) {
        Text(
            model.name,
            color = if (selected) MaterialTheme.colorScheme.primary else Color.Unspecified,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        val details =
            listOfNotNull(
                model.id.takeIf { it != model.name },
                costPerThousand(model, reads)?.let { tr("aiSort.modelPrice", mapOf("cost" to formatUsd(it))) },
                tr("aiSort.modelDecision").takeIf { model.decision },
            )
        if (details.isNotEmpty()) SmallNote(details.joinToString(" · "))
        if (model.reasoning) SmallNote(tr("aiSort.modelSlow"), MaterialTheme.colorScheme.error)
    }
    HorizontalDivider()
}
