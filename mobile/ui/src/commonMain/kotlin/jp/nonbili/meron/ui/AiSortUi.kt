// Presents inbox sorting controls and status on
// mobile. The screen formats provider settings,
// estimated costs and account names returned by the
// core.

package jp.nonbili.meron.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
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
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.AiModelList
import jp.nonbili.meron.shared.AiSortConfig
import jp.nonbili.meron.shared.AiSortError
import jp.nonbili.meron.shared.AiSortEstimate
import jp.nonbili.meron.shared.AiSortFolder
import jp.nonbili.meron.shared.AiSortKeyState
import jp.nonbili.meron.shared.AiSortLesson
import jp.nonbili.meron.shared.AiSortStatus
import jp.nonbili.meron.shared.accountSummaryIsRss
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private const val PROVIDER_TINFOIL = "tinfoil"

private const val PROVIDER_OPENROUTER = "openrouter"

private const val READS_HEADERS = "headers"

internal const val READS_START = "start"

internal const val AI_POLL_MS = 1500L

/** History rows shown at first and added by each "View more". */
internal const val AI_LOG_PAGE = 50

/** The core refuses these as targets too; the picker just doesn't offer them. */
private val SYSTEM_FOLDER_ROLES = setOf("inbox", "sent", "drafts", "trash", "junk")

internal fun accountName(account: AccountSummary): String = account.displayName.ifEmpty { account.email }

private fun folderKey(folder: AiSortFolder): String = if (folder.isMeron) "meron\u0000${folder.meronFolderId}" else "${folder.account}\u0000${folder.folder}"

@Composable
private fun errorLabel(error: AiSortError): String =
    when (error.kind) {
        "key" -> tr("aiSort.errorKey")
        "credit" -> tr("aiSort.errorCredit")
        "verify" -> tr("aiSort.errorVerify")
        else -> tr("aiSort.errorOther")
    }

@Composable
private fun providerLabel(provider: String): String = if (provider == PROVIDER_TINFOIL) tr("aiSort.providerTinfoil") else tr("aiSort.providerOpenRouter")

@Composable
private fun SectionTitle(text: String) {
    Text(text, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(top = 8.dp))
}

@Composable
internal fun SmallNote(
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
            meronChosen =
                current.folders
                    .filter { it.isMeron }
                    .map { it.meronFolderId }
                    .toSet(),
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
