// Builds editable recipient chips and the From selector for mobile composition.
// Address parsing keeps completed recipients separate from the partially typed address.
// The editor receives selected identities and recipient changes through callbacks.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.FocusState
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.SendIdentity
import jp.nonbili.meron.shared.formatSendIdentity
import jp.nonbili.meron.shared.splitRecipientEntries
import kotlin.math.abs

internal fun parseRecipients(value: String): Pair<List<String>, String> {
    if (value.isEmpty()) return Pair(emptyList(), "")
    // Quote/bracket-aware split so a quoted display name ("Doe, Jane" <j@x>)
    // stays one recipient instead of becoming two chips.
    val parts = splitRecipientEntries(value)
    if (parts.size <= 1) {
        return Pair(emptyList(), value)
    }
    val completed =
        parts
            .subList(0, parts.size - 1)
            .map { it.trim() }
            .filter { it.isNotEmpty() }
    val activeInput = parts.last().trimStart()
    return Pair(completed, activeInput)
}

internal fun getAvatarColor(identifier: String): Color {
    val colors =
        listOf(
            Color(0xFFC2185B),
            Color(0xFF7B1FA2),
            Color(0xFF512DA8),
            Color(0xFF303F9F),
            Color(0xFF1976D2),
            Color(0xFF00796B),
            Color(0xFF388E3C),
            Color(0xFFF57C00),
            Color(0xFF5D4037),
            Color(0xFF455A64),
        )
    val hash = identifier.hashCode()
    val index = abs(hash) % colors.size
    return colors[index]
}

internal fun parseEmailRecipient(recipient: String): Pair<String, String> {
    val trimmed = recipient.trim()
    if (trimmed.contains("<") && trimmed.endsWith(">")) {
        // Strip the quotes a quoted display name carries so the chip shows
        // `Doe, Jane`, not `"Doe, Jane"`.
        val name =
            trimmed
                .substringBefore("<")
                .trim()
                .removeSurrounding("\"")
                .replace("\\\"", "\"")
                .replace("\\\\", "\\")
                .trim()
        val email = trimmed.substringAfter("<").removeSuffix(">").trim()
        return Pair(name, email)
    }
    if (trimmed.contains("(") && trimmed.endsWith(")")) {
        val email = trimmed.substringBefore("(").trim()
        val name = trimmed.substringAfter("(").removeSuffix(")").trim()
        return Pair(name, email)
    }
    return Pair("", trimmed)
}

@Composable
internal fun RecipientChip(
    recipient: String,
    onDelete: () -> Unit,
) {
    val (name, email) = remember(recipient) { parseEmailRecipient(recipient) }
    val displayName = if (name.isNotEmpty()) name else email
    val initials = if (displayName.isNotEmpty()) displayName.take(1).uppercase() else "?"
    var menuExpanded by remember { mutableStateOf(false) }
    val clipboardManager = LocalClipboardManager.current

    Box {
        Surface(
            shape = RoundedCornerShape(16.dp),
            color = MaterialTheme.colorScheme.surface,
            border = androidx.compose.foundation.BorderStroke(1.dp, MaterialTheme.colorScheme.outline.copy(alpha = 0.5f)),
            modifier =
                Modifier
                    .height(32.dp)
                    .clickable { menuExpanded = true },
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier.padding(end = 12.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Box(
                    modifier =
                        Modifier
                            .size(32.dp)
                            .background(getAvatarColor(email), CircleShape),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = initials,
                        color = Color.White,
                        fontWeight = FontWeight.Bold,
                        fontSize = 13.sp,
                    )
                }
                Text(
                    text = displayName,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }

        DropdownMenu(
            expanded = menuExpanded,
            onDismissRequest = { menuExpanded = false },
            modifier =
                Modifier
                    .width(280.dp)
                    .background(MaterialTheme.colorScheme.surfaceContainerHigh),
        ) {
            Column(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .padding(vertical = 8.dp),
            ) {
                Row(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Box(
                        modifier =
                            Modifier
                                .size(40.dp)
                                .background(getAvatarColor(email), CircleShape),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            text = initials,
                            color = Color.White,
                            fontWeight = FontWeight.Bold,
                            fontSize = 16.sp,
                        )
                    }
                    Column {
                        if (name.isNotEmpty()) {
                            Text(
                                text = name,
                                style = MaterialTheme.typography.bodyLarge,
                                fontWeight = FontWeight.Bold,
                                color = MaterialTheme.colorScheme.onSurface,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        Text(
                            text = email,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }

                HorizontalDivider(
                    color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f),
                    thickness = 0.5.dp,
                    modifier = Modifier.padding(vertical = 4.dp),
                )

                DropdownMenuItem(
                    text = {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            Icon(
                                imageVector = Icons.Filled.ContentCopy,
                                contentDescription = null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.size(20.dp),
                            )
                            Text(
                                text = tr("common.copy"),
                                style = MaterialTheme.typography.bodyMedium,
                            )
                        }
                    },
                    onClick = {
                        clipboardManager.setText(AnnotatedString(email))
                        menuExpanded = false
                    },
                )

                HorizontalDivider(
                    color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f),
                    thickness = 0.5.dp,
                    modifier = Modifier.padding(vertical = 4.dp),
                )

                DropdownMenuItem(
                    text = {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            Icon(
                                imageVector = Icons.Filled.Delete,
                                contentDescription = null,
                                tint = MaterialTheme.colorScheme.error,
                                modifier = Modifier.size(20.dp),
                            )
                            Text(
                                text = tr("buttons.delete"),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.error,
                            )
                        }
                    },
                    onClick = {
                        onDelete()
                        menuExpanded = false
                    },
                )
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun RecipientChipsInput(
    value: String,
    onChange: (String) -> Unit,
    placeholder: String,
    keyboardOptions: KeyboardOptions,
    onFocusChanged: (FocusState) -> Unit,
    isFocused: Boolean,
) {
    val (completed, activeInput) = remember(value) { parseRecipients(value) }
    var initialized by remember { mutableStateOf(false) }
    val focusRequester = remember { FocusRequester() }

    // Maintain local TextFieldValue state for correct cursor positioning
    var activeInputState by remember { mutableStateOf(TextFieldValue("")) }

    // Keep activeInputState in sync with parent activeInput changes (e.g. initial reply, suggestion click, or backspace deletion)
    LaunchedEffect(activeInput) {
        if (activeInput != activeInputState.text) {
            activeInputState =
                TextFieldValue(
                    text = activeInput,
                    selection = TextRange(activeInput.length),
                )
        }
    }

    LaunchedEffect(value) {
        if (!initialized) {
            initialized = true
            if (value.isNotEmpty() && !value.trim().endsWith(",")) {
                onChange(value.trim() + ", ")
            }
        }
    }

    FlowRow(
        modifier =
            Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp)
                .clickable(
                    interactionSource = remember { MutableInteractionSource() },
                    indication = null,
                ) { focusRequester.requestFocus() }
                .padding(vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp, Alignment.CenterVertically),
    ) {
        completed.forEach { recipient ->
            RecipientChip(
                recipient = recipient,
                onDelete = {
                    val newCompleted = completed.filter { it != recipient }
                    val newValue =
                        if (newCompleted.isEmpty()) {
                            activeInput
                        } else {
                            newCompleted.joinToString(", ") + (if (activeInput.isNotEmpty()) ", $activeInput" else "")
                        }
                    onChange(newValue)
                },
            )
        }

        BasicTextField(
            value = activeInputState,
            onValueChange = { newActiveState ->
                // Update local state immediately to preserve cursor position
                activeInputState = newActiveState

                val newActiveText = newActiveState.text
                // Don't treat a comma inside an unfinished quoted name
                // ("Doe,) as a commit — wait for the closing quote.
                val quotesBalanced = newActiveText.count { it == '"' } % 2 == 0
                val committedValue =
                    if (newActiveText.endsWith(",") && quotesBalanced) {
                        val cleaned = newActiveText.removeSuffix(",").trim()
                        if (cleaned.isNotEmpty()) {
                            (completed + cleaned).joinToString(", ") + ", "
                        } else {
                            value
                        }
                    } else {
                        if (completed.isEmpty()) {
                            newActiveText
                        } else {
                            completed.joinToString(", ") + ", " + newActiveText
                        }
                    }
                onChange(committedValue)
            },
            keyboardOptions = keyboardOptions,
            textStyle = MaterialTheme.typography.bodyMedium.copy(color = MaterialTheme.colorScheme.onSurface),
            modifier =
                Modifier
                    .widthIn(min = 120.dp)
                    .align(Alignment.CenterVertically)
                    .focusRequester(focusRequester)
                    .onFocusChanged { focusState ->
                        if (!focusState.isFocused && activeInput.trim().isNotEmpty()) {
                            onChange((completed + activeInput.trim()).joinToString(", ") + ", ")
                        }
                        onFocusChanged(focusState)
                    },
            decorationBox = { innerTextField ->
                Box(
                    modifier = Modifier.padding(vertical = 6.dp),
                    contentAlignment = Alignment.CenterStart,
                ) {
                    if (activeInput.isEmpty() && completed.isEmpty() && placeholder.isNotEmpty()) {
                        Text(
                            text = placeholder,
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f),
                        )
                    }
                    innerTextField()
                }
            },
        )
    }
}

@Composable
internal fun FromIdentitySelector(
    identities: List<SendIdentity>,
    selectedKey: String,
    onSelect: (String) -> Unit,
    labelWidth: Dp = 0.dp,
    onLabelWidthChanged: (Dp) -> Unit = {},
) {
    var expanded by remember { mutableStateOf(false) }
    val selected = identities.firstOrNull { identityKey(it) == selectedKey } ?: identities.firstOrNull()
    val density = LocalDensity.current
    Box {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .heightIn(min = 48.dp)
                    .clickable { expanded = true }
                    .padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = tr("composer.fields.from"),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier =
                    Modifier
                        .onGloballyPositioned { layoutCoordinates ->
                            val widthDp = with(density) { layoutCoordinates.size.width.toDp() }
                            if (widthDp > labelWidth) {
                                onLabelWidthChanged(widthDp)
                            }
                        }.then(
                            if (labelWidth > 0.dp) Modifier.width(labelWidth) else Modifier.wrapContentWidth(),
                        ),
            )
            Box(modifier = Modifier.weight(1f)) {
                Text(
                    text = selected?.let { formatSendIdentity(it) } ?: tr("mobile.compose.selectSender"),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.fillMaxWidth(),
                )
                DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
                    identities.forEach { identity ->
                        DropdownMenuItem(
                            text = {
                                Text(
                                    formatSendIdentity(identity),
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            },
                            onClick = {
                                onSelect(identityKey(identity))
                                expanded = false
                            },
                        )
                    }
                }
            }
            Icon(
                imageVector = Icons.Filled.ArrowDropDown,
                contentDescription = tr("common.more"),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(end = 12.dp),
            )
        }
    }
    HorizontalDivider(
        color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.6f),
        thickness = 0.5.dp,
    )
}
