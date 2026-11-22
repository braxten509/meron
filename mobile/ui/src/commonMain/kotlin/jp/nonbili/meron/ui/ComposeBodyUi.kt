// Renders a message body and its folded reply quotation.
// The composer controls the editor value, focus and attachment callbacks.
// The quote chip lets the writer expand or remove the quoted message.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.ContactSuggestion

@Composable
internal fun ComposeField(
    value: String,
    onChange: (String) -> Unit,
    label: String,
    field: String = "",
    suggestions: List<ContactSuggestion> = emptyList(),
    onFocus: (field: String, value: String) -> Unit = { _, _ -> },
    onAcceptSuggestion: (field: String, contact: ContactSuggestion) -> Unit = { _, _ -> },
    modifier: Modifier = Modifier,
    placeholder: String = "",
    autoFocus: Boolean = false,
    trailingContent: @Composable (RowScope.() -> Unit)? = null,
    labelWidth: Dp = 0.dp,
    onLabelWidthChanged: (Dp) -> Unit = {},
) {
    var isFocused by remember { mutableStateOf(false) }
    val isRecipientField = field == "to" || field == "cc" || field == "bcc"
    val density = LocalDensity.current

    Column(modifier) {
        Row(
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                text = label,
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
                if (isRecipientField) {
                    RecipientChipsInput(
                        value = value,
                        onChange = onChange,
                        placeholder = placeholder,
                        keyboardOptions = nativeTextKeyboardOptions,
                        onFocusChanged = {
                            isFocused = it.isFocused
                            if (it.isFocused && field.isNotBlank()) onFocus(field, value)
                        },
                        isFocused = isFocused,
                    )
                } else {
                    BasicTextField(
                        value = value,
                        onValueChange = onChange,
                        keyboardOptions = nativeTextKeyboardOptions,
                        textStyle = MaterialTheme.typography.bodyMedium.copy(color = MaterialTheme.colorScheme.onSurface),
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .onFocusChanged {
                                    isFocused = it.isFocused
                                    if (it.isFocused && field.isNotBlank()) onFocus(field, value)
                                },
                        decorationBox = { innerTextField ->
                            Box(
                                modifier = Modifier.padding(vertical = 12.dp),
                                contentAlignment = Alignment.CenterStart,
                            ) {
                                if (value.isEmpty() && placeholder.isNotEmpty()) {
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

            if (value.isEmpty() && !isRecipientField) {
                val clipboardManager = LocalClipboardManager.current
                IconButton(
                    onClick = {
                        val text = clipboardManager.getText()?.text.orEmpty()
                        if (text.isNotEmpty()) {
                            onChange(text)
                        }
                    },
                ) {
                    Icon(
                        Icons.Filled.ContentPaste,
                        contentDescription = tr("common.paste"),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            if (trailingContent != null) {
                trailingContent()
            }
        }

        HorizontalDivider(
            color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.6f),
            thickness = 0.5.dp,
        )

        val activeInputText = if (isRecipientField) parseRecipients(value).second else value
        val showSuggestions = isFocused && suggestions.isNotEmpty() && activeInputText.trim().isNotEmpty()

        if (showSuggestions) {
            Surface(
                color = MaterialTheme.colorScheme.surfaceContainerLow,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column(
                    modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
                ) {
                    suggestions.forEach { contact ->
                        Row(
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .clickable { onAcceptSuggestion(field, contact) }
                                    .padding(horizontal = 24.dp, vertical = 12.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(16.dp),
                        ) {
                            Box(
                                modifier =
                                    Modifier
                                        .size(36.dp)
                                        .background(getAvatarColor(contact.addr), CircleShape),
                                contentAlignment = Alignment.Center,
                            ) {
                                val initials = if (contact.name.isNotEmpty()) contact.name.take(1).uppercase() else contact.addr.take(1).uppercase()
                                Text(
                                    text = initials,
                                    color = Color.White,
                                    fontWeight = FontWeight.Bold,
                                    fontSize = 14.sp,
                                )
                            }
                            Column {
                                if (contact.name.isNotEmpty()) {
                                    Text(
                                        text = contact.name,
                                        style = MaterialTheme.typography.bodyMedium,
                                        fontWeight = FontWeight.Bold,
                                        color = MaterialTheme.colorScheme.onSurface,
                                    )
                                }
                                Text(
                                    text = contact.addr,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                }
            }
            HorizontalDivider(
                color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.6f),
                thickness = 0.5.dp,
            )
        }
    }
}

// The quote of the message being answered, folded away under the body: a tap on
// the dots unfolds it into the body, a long press offers to drop it instead.
@Composable
internal fun FoldedQuoteChip(
    onExpand: () -> Unit,
    onRemove: () -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }
    Box(Modifier.padding(horizontal = 16.dp)) {
        QuoteToggle(
            open = false,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            onToggle = onExpand,
            onLongClick = { menuOpen = true },
        )
        DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
            DropdownMenuItem(
                text = { Text(tr("chat.showQuotedText")) },
                onClick = {
                    menuOpen = false
                    onExpand()
                },
            )
            DropdownMenuItem(
                text = { Text(tr("composer.removeQuotedText"), color = MaterialTheme.colorScheme.error) },
                leadingIcon = { Icon(Icons.Filled.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.error) },
                onClick = {
                    menuOpen = false
                    onRemove()
                },
            )
        }
    }
}
