// Supplies the rows and section labels used throughout mobile settings.
// Accounts, actions and toggle preferences share the same typography and spacing.
// Callers provide the displayed values and handle selection changes.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Inbox
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.RssFeed
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.accountSummaryIsRss

@Composable
internal fun SettingsSectionLabel(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 18.dp, bottom = 2.dp),
    )
}

@Composable
internal fun SettingsEmptyLabel(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
    )
}

@Composable
internal fun SettingsAccountRow(
    account: AccountSummary,
    hidden: Boolean,
    fallbackTitle: String? = null,
    showAccountIdFallback: Boolean = true,
    showDefaultSubtitle: Boolean = true,
    onClick: () -> Unit,
) {
    val label =
        if (showAccountIdFallback) {
            account.displayName.ifBlank { account.email.ifBlank { account.id } }
        } else {
            account.displayName.ifBlank {
                fallbackTitle ?: account.id.takeIf { showAccountIdFallback }.orEmpty()
            }
        }
    SettingsRow(
        icon = if (accountSummaryIsRss(account)) Icons.Filled.RssFeed else Icons.Filled.Inbox,
        leading = {
            AccountBadgeAvatar(
                label = label,
                avatarUrl = account.avatarUrl,
                size = 40.dp,
                fallbackIcon = accountAvatarFallbackIcon(account),
            )
        },
        title = label,
        subtitle =
            listOfNotNull(
                account.email.takeIf { showAccountIdFallback && it.isNotBlank() && it != label },
                if (hidden) tr("settings.account.hiddenFromNavigation") else null,
            ).joinToString(" · ").ifBlank {
                tr("settings.account.accountSettings").takeIf { showDefaultSubtitle }
            },
        onClick = onClick,
    )
}

@Composable
internal fun SettingsRow(
    icon: ImageVector,
    title: String,
    subtitle: String? = null,
    hint: String? = null,
    onClick: () -> Unit,
    leading: (@Composable () -> Unit)? = null,
    trailing: (@Composable () -> Unit)? = null,
    destructive: Boolean = false,
) {
    val accent = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface
    ListItem(
        headlineContent = { SettingsHeadline(title = title, hint = hint, color = accent) },
        supportingContent = subtitle?.let { { Text(it, maxLines = 2, overflow = TextOverflow.Ellipsis) } },
        leadingContent =
            leading ?: {
                Icon(
                    icon,
                    contentDescription = null,
                    tint = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                )
            },
        trailingContent = trailing,
        modifier = Modifier.clickable(onClick = onClick),
    )
}

// Full-width text input styled to align with the list rows in the detail forms.
@Composable
internal fun SettingsTextRow(
    value: String,
    label: String,
    onValueChange: (String) -> Unit,
    supporting: String? = null,
    singleLine: Boolean = true,
    minLines: Int = 1,
    keyboardDigits: Boolean = false,
) {
    // Sit the field on a full-width surface strip so the row reads as a white
    // section, matching the ListItem-based rows above and below it.
    Surface(color = MaterialTheme.colorScheme.surface, modifier = Modifier.fillMaxWidth()) {
        OutlinedTextField(
            value = value,
            onValueChange = onValueChange,
            label = { Text(label) },
            supportingText = supporting?.let { { Text(it) } },
            singleLine = singleLine,
            minLines = minLines,
            keyboardOptions =
                if (keyboardDigits) {
                    nativeTextKeyboardOptions.copy(keyboardType = KeyboardType.Number)
                } else {
                    nativeTextKeyboardOptions
                },
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        )
    }
}

@Composable
internal fun SettingsToggleRow(
    icon: ImageVector,
    title: String,
    subtitle: String? = null,
    hint: String? = null,
    checked: Boolean,
    /** Shown in place of [icon], such as an account's avatar. */
    leading: (@Composable () -> Unit)? = null,
    enabled: Boolean = true,
    onToggle: () -> Unit,
) {
    ListItem(
        headlineContent = { SettingsHeadline(title = title, hint = hint) },
        supportingContent = subtitle?.let { { Text(it, maxLines = 2, overflow = TextOverflow.Ellipsis) } },
        leadingContent = leading ?: { Icon(icon, contentDescription = null) },
        trailingContent = {
            Switch(
                checked = checked,
                onCheckedChange = { onToggle() },
                enabled = enabled,
                colors = settingsSwitchColors(),
            )
        },
        modifier = Modifier.clickable(enabled = enabled, onClick = onToggle),
    )
}

@Composable
private fun SettingsHeadline(
    title: String,
    hint: String?,
    color: Color = MaterialTheme.colorScheme.onSurface,
) {
    var showHint by remember { mutableStateOf(false) }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(
            title,
            color = color,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f, fill = false),
        )
        if (hint != null) {
            IconButton(onClick = { showHint = true }, modifier = Modifier.size(32.dp)) {
                Icon(
                    Icons.Filled.Info,
                    contentDescription = hint,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(18.dp),
                )
            }
        }
    }
    if (showHint && hint != null) {
        AlertDialog(
            onDismissRequest = { showHint = false },
            title = { Text(title) },
            text = { Text(hint) },
            confirmButton = {
                TextButton(onClick = { showHint = false }) { Text(tr("common.close")) }
            },
        )
    }
}

@Composable
private fun settingsSwitchColors() =
    with(MaterialTheme.colorScheme) {
        val isDark = background.luminance() < 0.5f
        val uncheckedTrack = if (isDark) Color(0xFF475569) else Color(0xFFCBD5E1)
        val uncheckedThumb = if (isDark) Color(0xFFE2E8F0) else Color.White
        // White is unreadable on a light primary, as in a dynamic dark scheme.
        val checkedThumb = if (primary.luminance() > 0.4f) onPrimary else Color.White
        SwitchDefaults.colors(
            checkedThumbColor = checkedThumb,
            checkedTrackColor = primary,
            uncheckedThumbColor = uncheckedThumb,
            uncheckedTrackColor = uncheckedTrack,
            uncheckedBorderColor = outline,
            disabledCheckedThumbColor = checkedThumb.copy(alpha = 0.38f),
            disabledCheckedTrackColor = primary.copy(alpha = 0.38f),
            disabledUncheckedThumbColor = uncheckedThumb.copy(alpha = 0.55f),
            disabledUncheckedTrackColor = uncheckedTrack.copy(alpha = 0.38f),
            disabledUncheckedBorderColor = outline.copy(alpha = 0.38f),
        )
    }
