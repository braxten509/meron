// Renders the mobile message editor and captures the
// inputs used for draft autosave. The open draft must
// retain its identity as recipients, content and
// attachments change.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.AttachFile
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.outlined.Drafts
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import jp.nonbili.meron.shared.ContactSuggestion
import jp.nonbili.meron.shared.DraftAttachment
import jp.nonbili.meron.shared.SendIdentity
import kotlinx.coroutines.delay
import kotlin.math.max

private const val COMPOSE_AUTOSAVE_DELAY_MS = 3_000L

internal data class ComposeAutosaveSnapshot(
    val selectedFromKey: String,
    val to: String,
    val cc: String,
    val bcc: String,
    val subject: String,
    val body: String,
    val attachments: List<DraftAttachment>,
)

internal fun composeAutosaveSnapshot(
    selectedFromKey: String,
    to: String,
    cc: String,
    bcc: String,
    subject: String,
    body: String,
    attachments: List<DraftAttachment>,
): ComposeAutosaveSnapshot? {
    val hasContent =
        to.isNotBlank() || cc.isNotBlank() || bcc.isNotBlank() ||
            subject.isNotBlank() || body.isNotBlank() || attachments.isNotEmpty()
    if (!hasContent) return null
    return ComposeAutosaveSnapshot(
        selectedFromKey = selectedFromKey,
        to = to,
        cc = cc,
        bcc = bcc,
        subject = subject,
        body = body,
        attachments = attachments,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ComposeScreen(
    sendIdentities: List<SendIdentity>,
    selectedFromKey: String,
    onFromChange: (String) -> Unit,
    to: String,
    onToChange: (String) -> Unit,
    cc: String,
    onCcChange: (String) -> Unit,
    bcc: String,
    onBccChange: (String) -> Unit,
    subject: String,
    onSubjectChange: (String) -> Unit,
    body: String,
    onBodyChange: (String) -> Unit,
    hasFoldedQuote: Boolean,
    onExpandQuote: () -> Unit,
    onRemoveQuote: () -> Unit,
    attachments: List<DraftAttachment>,
    recipientSuggestionField: String,
    recipientSuggestions: List<ContactSuggestion>,
    onRecipientFocus: (field: String, value: String) -> Unit,
    onAcceptRecipientSuggestion: (field: String, contact: ContactSuggestion) -> Unit,
    onAttach: () -> Unit,
    onClearAttachments: () -> Unit,
    onRemoveAttachment: (DraftAttachment) -> Unit,
    sendShortcutMode: SendShortcutMode,
    onSaveDraft: () -> Unit,
    onAutoSaveDraft: () -> Unit,
    onDiscardDraft: () -> Unit,
    onSend: () -> Unit,
    onBack: () -> Unit,
    sending: Boolean = false,
    snackbarHost: @Composable () -> Unit = {},
) {
    var confirmDiscard by remember { mutableStateOf(false) }
    var overflowOpen by remember { mutableStateOf(false) }
    var showCcBcc by remember { mutableStateOf(cc.isNotBlank() || bcc.isNotBlank()) }
    var maxLabelWidth by remember { mutableStateOf(0.dp) }
    val hasContent =
        to.isNotBlank() || cc.isNotBlank() || bcc.isNotBlank() ||
            subject.isNotBlank() || body.isNotBlank() || attachments.isNotEmpty()
    val autosaveSnapshot =
        remember(selectedFromKey, to, cc, bcc, subject, body, attachments) {
            composeAutosaveSnapshot(
                selectedFromKey = selectedFromKey,
                to = to,
                cc = cc,
                bcc = bcc,
                subject = subject,
                body = body,
                attachments = attachments,
            )
        }
    var autosaveInitialized by remember { mutableStateOf(false) }
    var lastAutosavedSnapshot by remember { mutableStateOf<ComposeAutosaveSnapshot?>(null) }

    LaunchedEffect(autosaveSnapshot) {
        if (!autosaveInitialized) {
            autosaveInitialized = true
            lastAutosavedSnapshot = autosaveSnapshot
            return@LaunchedEffect
        }
        val snapshot = autosaveSnapshot ?: return@LaunchedEffect
        if (snapshot == lastAutosavedSnapshot) return@LaunchedEffect
        delay(COMPOSE_AUTOSAVE_DELAY_MS)
        if (snapshot != lastAutosavedSnapshot) {
            onAutoSaveDraft()
            lastAutosavedSnapshot = snapshot
        }
    }

    if (confirmDiscard) {
        AlertDialog(
            onDismissRequest = { confirmDiscard = false },
            title = { Text(tr("mobile.compose.discardDraftTitle")) },
            text = { Text(tr("mobile.compose.discardDraftText")) },
            confirmButton = {
                TextButton(onClick = {
                    confirmDiscard = false
                    onDiscardDraft()
                }) {
                    Text(tr("buttons.discard"), color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmDiscard = false }) { Text(tr("mobile.compose.keepEditing")) }
            },
        )
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.surface,
        // Render status snackbars here too — without a host on this screen,
        // send/draft failures would queue up and only appear after navigating
        // back to the thread list.
        snackbarHost = snackbarHost,
        topBar = {
            TopAppBar(
                title = { Text(tr("composer.actions.newMessage")) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.Filled.Close, contentDescription = tr("buttons.discard"))
                    }
                },
                actions = {
                    IconButton(onClick = onAttach) {
                        Icon(Icons.Filled.AttachFile, contentDescription = tr("composer.actions.attachFiles"))
                    }
                    val sendEnabled = to.isNotBlank() && !sending
                    val sendingDescription = tr("composer.status.sending")
                    IconButton(onClick = onSend, enabled = sendEnabled) {
                        if (sending) {
                            CircularProgressIndicator(
                                modifier =
                                    Modifier
                                        .size(20.dp)
                                        .semantics { contentDescription = sendingDescription },
                                color = MaterialTheme.colorScheme.primary,
                                strokeWidth = 2.dp,
                            )
                        } else {
                            Icon(
                                Icons.AutoMirrored.Filled.Send,
                                contentDescription = tr("buttons.send"),
                                tint = if (sendEnabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface.copy(alpha = 0.38f),
                            )
                        }
                    }
                    Box {
                        IconButton(onClick = { overflowOpen = true }) {
                            Icon(Icons.Filled.MoreVert, contentDescription = tr("common.more"))
                        }
                        DropdownMenu(expanded = overflowOpen, onDismissRequest = { overflowOpen = false }) {
                            DropdownMenuItem(
                                text = { Text(tr("composer.actions.saveDraft")) },
                                leadingIcon = { Icon(Icons.Outlined.Drafts, contentDescription = null) },
                                onClick = {
                                    overflowOpen = false
                                    onSaveDraft()
                                },
                            )
                            DropdownMenuItem(
                                text = { Text(tr("chat.actions.discardDraft"), color = MaterialTheme.colorScheme.error) },
                                leadingIcon = { Icon(Icons.Filled.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
                                onClick = {
                                    overflowOpen = false
                                    if (hasContent) confirmDiscard = true else onDiscardDraft()
                                },
                            )
                        }
                    }
                },
            )
        },
    ) { innerPadding ->
        BoxWithConstraints(
            Modifier
                .fillMaxSize()
                .background(MaterialTheme.colorScheme.surface)
                .padding(innerPadding)
                .consumeWindowInsets(innerPadding)
                .imePadding(),
        ) {
            val optionalFieldsHeight = (if (sendIdentities.size > 1) 56 else 0) + (if (showCcBcc) 128 else 0)
            val attachmentHeight = attachments.size * 36 + if (attachments.isNotEmpty()) 44 else 0
            // With a folded quote the body only grows with what is typed, so the
            // quote's dots sit right under the text rather than below an empty box.
            val bodyMinHeight =
                if (hasFoldedQuote) {
                    96.dp
                } else {
                    max(
                        280f,
                        maxHeight.value - optionalFieldsHeight - attachmentHeight - 220f,
                    ).dp
                }
            val bodyScrollState = rememberScrollState()
            Column(
                Modifier
                    .fillMaxSize()
                    .appScrollbar(bodyScrollState)
                    .verticalScroll(bodyScrollState),
            ) {
                if (sendIdentities.size > 1) {
                    FromIdentitySelector(
                        identities = sendIdentities,
                        selectedKey = selectedFromKey,
                        onSelect = onFromChange,
                        labelWidth = maxLabelWidth,
                        onLabelWidthChanged = { maxLabelWidth = it },
                    )
                }
                ComposeField(
                    value = to,
                    onChange = onToChange,
                    label = tr("composer.fields.to"),
                    field = "to",
                    suggestions = if (recipientSuggestionField == "to") recipientSuggestions else emptyList(),
                    onFocus = onRecipientFocus,
                    onAcceptSuggestion = onAcceptRecipientSuggestion,
                    placeholder = tr("composer.placeholders.recipient"),
                    trailingContent =
                        if (!showCcBcc) {
                            {
                                Icon(
                                    imageVector = Icons.Filled.ArrowDropDown,
                                    contentDescription = tr("composer.actions.ccBcc"),
                                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                    modifier =
                                        Modifier
                                            .padding(end = 12.dp)
                                            .clickable { showCcBcc = true },
                                )
                            }
                        } else {
                            null
                        },
                    labelWidth = maxLabelWidth,
                    onLabelWidthChanged = { maxLabelWidth = it },
                )
                if (showCcBcc) {
                    ComposeField(
                        value = cc,
                        onChange = onCcChange,
                        label = tr("composer.fields.cc"),
                        field = "cc",
                        suggestions = if (recipientSuggestionField == "cc") recipientSuggestions else emptyList(),
                        onFocus = onRecipientFocus,
                        onAcceptSuggestion = onAcceptRecipientSuggestion,
                        placeholder = tr("composer.placeholders.commaSeparated"),
                        labelWidth = maxLabelWidth,
                        onLabelWidthChanged = { maxLabelWidth = it },
                    )
                    ComposeField(
                        value = bcc,
                        onChange = onBccChange,
                        label = tr("composer.fields.bcc"),
                        field = "bcc",
                        suggestions = if (recipientSuggestionField == "bcc") recipientSuggestions else emptyList(),
                        onFocus = onRecipientFocus,
                        onAcceptSuggestion = onAcceptRecipientSuggestion,
                        placeholder = tr("composer.placeholders.commaSeparated"),
                        labelWidth = maxLabelWidth,
                        onLabelWidthChanged = { maxLabelWidth = it },
                    )
                }
                ComposeField(
                    value = subject,
                    onChange = onSubjectChange,
                    label = tr("composer.fields.subject"),
                    placeholder = tr("composer.fields.subject"),
                    labelWidth = maxLabelWidth,
                    onLabelWidthChanged = { maxLabelWidth = it },
                )
                TextField(
                    value = body,
                    onValueChange = onBodyChange,
                    placeholder = { Text(tr("composer.placeholders.message"), style = MaterialTheme.typography.bodyMedium) },
                    keyboardOptions = nativeTextKeyboardOptions,
                    colors =
                        TextFieldDefaults.colors(
                            focusedContainerColor = Color.Transparent,
                            unfocusedContainerColor = Color.Transparent,
                            disabledContainerColor = Color.Transparent,
                            focusedIndicatorColor = Color.Transparent,
                            unfocusedIndicatorColor = Color.Transparent,
                            disabledIndicatorColor = Color.Transparent,
                        ),
                    textStyle = MaterialTheme.typography.bodyMedium,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .heightIn(min = bodyMinHeight)
                            .onPreviewKeyEvent { event ->
                                if (shouldSendFromEditor(event, sendShortcutMode)) {
                                    onSend()
                                    true
                                } else {
                                    false
                                }
                            },
                )
                if (hasFoldedQuote) {
                    FoldedQuoteChip(onExpand = onExpandQuote, onRemove = onRemoveQuote)
                }
                if (attachments.isNotEmpty()) {
                    Column(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .padding(horizontal = 16.dp, vertical = 8.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        attachments.forEach { attachment ->
                            Row(
                                modifier = Modifier.fillMaxWidth().heightIn(min = 36.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Icon(
                                    imageVector = Icons.Filled.AttachFile,
                                    contentDescription = null,
                                    modifier = Modifier.size(18.dp),
                                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                                Text(
                                    text = "${attachment.displayName} · ${formatBytes(attachment.sizeBytes)}",
                                    style = MaterialTheme.typography.bodyMedium,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.weight(1f),
                                )
                                IconButton(
                                    onClick = { onRemoveAttachment(attachment) },
                                    modifier = Modifier.size(24.dp),
                                ) {
                                    Icon(
                                        imageVector = Icons.Filled.Close,
                                        contentDescription = tr("composer.actions.removeAttachment"),
                                        modifier = Modifier.size(16.dp),
                                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                            }
                        }
                        TextButton(
                            onClick = onClearAttachments,
                            modifier = Modifier.align(Alignment.Start),
                        ) {
                            Text(tr("mobile.compose.clearAttachments"))
                        }
                    }
                }
            }
        }
    }
}
