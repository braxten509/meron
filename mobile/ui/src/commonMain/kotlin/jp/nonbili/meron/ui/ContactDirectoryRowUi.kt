// Renders a contact in the mobile directory.
// The row displays its identity and supports opening details or changing its pin.
// The directory screen handles those actions for the selected address.

package jp.nonbili.meron.ui

import androidx.compose.animation.core.animate
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PushPin
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import jp.nonbili.meron.shared.ContactSuggestion
import jp.nonbili.meron.shared.formatContactSuggestion
import kotlin.math.roundToInt

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun ContactDirectoryRow(
    state: ContactsScreenState,
    contact: ContactSuggestion,
    showSenderImages: Boolean,
    onSelect: () -> Unit,
    onPin: () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val pinned = state.isPinned(contact)
    val pinLabel = tr(if (pinned) "contacts.unpin" else "contacts.pin")
    val enabled = state.pinsLoaded && !state.pinsSaving
    var menuOpen by remember(contact.addr) { mutableStateOf(false) }
    var reveal by remember(contact.addr) { mutableFloatStateOf(0f) }
    val revealWidth = with(LocalDensity.current) { 88.dp.toPx() }
    val direction = if (LocalLayoutDirection.current == LayoutDirection.Rtl) -1f else 1f
    val displayName = contact.name.ifBlank { contact.addr }
    val hasDistinctName = contact.name.isNotBlank() && !contact.name.equals(contact.addr, ignoreCase = true)
    Box(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 1.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(if (reveal < -0.5f) colors.secondaryContainer else colors.surface),
    ) {
        if (reveal < -0.5f) {
            Box(
                modifier =
                    Modifier
                        .align(Alignment.CenterEnd)
                        .width(88.dp)
                        .heightIn(min = 58.dp)
                        .clickable(enabled = enabled) {
                            reveal = 0f
                            onPin()
                        }.padding(horizontal = 6.dp),
                contentAlignment = Alignment.Center,
            ) {
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    Icon(
                        if (pinned) Icons.Outlined.PushPin else Icons.Filled.PushPin,
                        contentDescription = null,
                        tint = colors.onSecondaryContainer,
                        modifier = Modifier.size(18.dp),
                    )
                    Text(
                        pinLabel,
                        style = MaterialTheme.typography.labelSmall,
                        color = colors.onSecondaryContainer,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
        Row(
            Modifier
                .offset { IntOffset((reveal * direction).roundToInt(), 0) }
                .fillMaxWidth()
                .heightIn(min = 58.dp)
                .background(colors.surface)
                .semantics {
                    if (enabled) {
                        customActions =
                            listOf(
                                CustomAccessibilityAction(pinLabel) {
                                    reveal = 0f
                                    onPin()
                                    true
                                },
                            )
                    }
                }
                // A bounded reveal, never a dismiss: releasing exposes a button, not an action.
                .draggable(
                    state = rememberDraggableState { delta -> reveal = (reveal + delta * direction).coerceIn(-revealWidth, 0f) },
                    orientation = Orientation.Horizontal,
                    enabled = enabled,
                    onDragStopped = { velocity ->
                        val target = if (velocity * direction < -600f || (velocity * direction <= 600f && reveal < -revealWidth / 2)) -revealWidth else 0f
                        animate(reveal, target) { value, _ -> reveal = value }
                    },
                ).combinedClickable(
                    onClick = {
                        if (reveal < -0.5f) {
                            reveal = 0f
                        } else {
                            onSelect()
                        }
                    },
                    onLongClickLabel = pinLabel,
                    onLongClick = { menuOpen = true },
                ).padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SenderAvatar(
                formatContactSuggestion(contact),
                enabled = showSenderImages,
                size = 40.dp,
                fallbackName = displayName,
            )
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    displayName,
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.Medium,
                    color = colors.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (hasDistinctName) {
                    Text(
                        contact.addr,
                        style = MaterialTheme.typography.bodySmall,
                        color = colors.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            if (pinned) {
                Icon(
                    Icons.Filled.PushPin,
                    contentDescription = null,
                    tint = colors.primary.copy(alpha = 0.7f),
                    modifier = Modifier.size(15.dp),
                )
            }
        }
        DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
            ContactPinMenuItem(state, contact) {
                menuOpen = false
                reveal = 0f
                onPin()
            }
        }
    }
}

@Composable
private fun ContactPinMenuItem(
    state: ContactsScreenState,
    contact: ContactSuggestion,
    onClick: () -> Unit,
) {
    val pinned = state.isPinned(contact)
    DropdownMenuItem(
        text = { Text(tr(if (pinned) "contacts.unpin" else "contacts.pin")) },
        leadingIcon = { Icon(if (pinned) Icons.Outlined.PushPin else Icons.Filled.PushPin, null) },
        onClick = onClick,
        enabled = state.pinsLoaded && !state.pinsSaving,
    )
}
