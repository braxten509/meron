// Provides dialogs and secondary pages opened from
// mobile settings, including theme selection and
// import. The settings screen owns navigation to these
// surfaces.

package jp.nonbili.meron.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Download
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

/**
 * Theme picker page: the "match system" switch over light and dark sections of preview swatches,
 * mirroring the desktop ThemeDialog grid. While the theme follows the system each section holds its
 * own pick. Custom themes sit after the built-ins; there is no editor, so they arrive through Import
 * as the one-line string desktop shares.
 */
@Composable
internal fun ThemePickerPage(
    choice: ThemeChoice,
    systemDark: Boolean,
    onChange: (ThemeChoice) -> Unit,
    customThemes: List<CustomTheme>,
    onImportCustomTheme: (CustomTheme) -> Unit,
    onDeleteCustomTheme: (CustomTheme) -> Unit,
    onShareCustomTheme: (CustomTheme) -> Unit,
    modifier: Modifier = Modifier,
) {
    val builtins: List<AppTheme> = AppAppearanceMode.entries.filter { dynamicColorSupported || !it.isDynamic }
    val (darkModes, lightModes) = (builtins + customThemes).partition { it.isDark }
    val onSelect: (AppTheme) -> Unit = { mode -> onChange(choice.select(mode)) }
    var importing by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf<CustomTheme?>(null) }
    val swatchActions = SwatchActions(onShare = onShareCustomTheme, onDelete = { deleting = it })
    val lightLabel = tr("theme.light")
    val darkLabel = tr("theme.dark")
    val listState = rememberLazyListState()
    LazyColumn(
        modifier = modifier.appScrollbar(listState),
        state = listState,
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item {
            Row(
                Modifier
                    .fillMaxWidth()
                    .clickable { onChange(choice.withFollowSystem(!choice.followSystem, systemDark)) }
                    .padding(vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Column(Modifier.weight(1f)) {
                    Text(tr("theme.matchSystem"), style = MaterialTheme.typography.bodyLarge)
                    Text(
                        tr("theme.matchSystemHint"),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Switch(
                    checked = choice.followSystem,
                    onCheckedChange = { onChange(choice.withFollowSystem(it, systemDark)) },
                )
            }
        }
        themeSwatchSection(lightLabel, lightModes, choice.chosen, onSelect, swatchActions)
        themeSwatchSection(darkLabel, darkModes, choice.chosen, onSelect, swatchActions)
        item {
            OutlinedButton(onClick = { importing = true }, modifier = Modifier.padding(top = 4.dp)) {
                Icon(Icons.Filled.Download, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text(tr("theme.import"))
            }
        }
    }
    if (importing) {
        ImportThemeDialog(
            onImport = { theme ->
                importing = false
                onImportCustomTheme(theme)
            },
            onDismiss = { importing = false },
        )
    }
    deleting?.let { theme ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            title = { Text(tr("theme.delete")) },
            text = { Text(tr("theme.deleteMessage", mapOf("name" to theme.name))) },
            confirmButton = {
                TextButton(onClick = {
                    deleting = null
                    onDeleteCustomTheme(theme)
                }) { Text(tr("buttons.delete"), color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { deleting = null }) { Text(tr("buttons.cancel")) } },
        )
    }
}

private class SwatchActions(
    val onShare: (CustomTheme) -> Unit,
    val onDelete: (CustomTheme) -> Unit,
)

/**
 * Import a shared theme string: pasted, or taken from the clipboard when it
 * already holds one. The preview shows the theme as soon as the text parses.
 */
@Composable
private fun ImportThemeDialog(
    onImport: (CustomTheme) -> Unit,
    onDismiss: () -> Unit,
) {
    val clipboardManager = LocalClipboardManager.current
    var text by remember {
        mutableStateOf(
            clipboardManager
                .getText()
                ?.text
                ?.trim()
                ?.takeIf { parseThemeSource(it) != null }
                .orEmpty(),
        )
    }
    var name by remember { mutableStateOf("") }
    val source = parseThemeSource(text)
    val fallbackName = tr("theme.customTheme")
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(tr("theme.import")) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedTextField(
                    value = text,
                    onValueChange = { text = it },
                    placeholder = { Text(tr("theme.importPlaceholder")) },
                    isError = source == null && text.isNotBlank(),
                    supportingText =
                        if (source == null && text.isNotBlank()) {
                            { Text(tr("theme.importInvalid")) }
                        } else {
                            null
                        },
                    textStyle = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    placeholder = { Text(tr("theme.namePlaceholder")) },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                if (source != null) {
                    ThemePreviewMock(
                        themePreviewColors(CustomTheme(id = "", name = "", source = source)),
                        modifier = Modifier.clip(RoundedCornerShape(12.dp)),
                    )
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = source != null,
                onClick = {
                    if (source != null) onImport(CustomTheme(newCustomThemeId(), name.trim().ifEmpty { fallbackName }, source))
                },
            ) { Text(tr("theme.save")) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(tr("buttons.cancel")) } },
    )
}

/** A labelled section of theme swatches, laid out two per row. */
private fun LazyListScope.themeSwatchSection(
    label: String,
    modes: List<AppTheme>,
    chosen: Set<AppTheme>,
    onSelect: (AppTheme) -> Unit,
    actions: SwatchActions,
) {
    item {
        Text(
            label,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(bottom = 2.dp),
        )
    }
    items(modes.chunked(2)) { row ->
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            row.forEach { mode ->
                ThemeSwatch(
                    mode = mode,
                    selected = mode in chosen,
                    onSelect = { onSelect(mode) },
                    modifier = Modifier.weight(1f),
                    onShare = (mode as? CustomTheme)?.let { { actions.onShare(it) } },
                    onDelete = (mode as? CustomTheme)?.let { { actions.onDelete(it) } },
                )
            }
            if (row.size == 1) Spacer(Modifier.weight(1f))
        }
    }
}

/**
 * Message text size picker: a slider over the size steps above a sample drawn
 * at the size it would pick. The setting is applied as the slider moves rather
 * than on dismiss, so the mail behind the dialog resizes with the sample and
 * there is nothing to confirm.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun MessageTextSizeDialog(
    scale: Int,
    onScaleChange: (Int) -> Unit,
    onDismiss: () -> Unit,
) {
    val steps = MESSAGE_FONT_SCALE_STEPS
    val index = steps.indexOf(coerceMessageFontScale(scale)).coerceAtLeast(0)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(tr("settings.appearance.messageTextSize")) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                // Fixed height: the sample reflows as it grows, and a box that
                // resized with it would shift the slider under the finger.
                Surface(
                    color = MaterialTheme.colorScheme.surfaceVariant,
                    shape = RoundedCornerShape(10.dp),
                ) {
                    Box(
                        Modifier
                            .fillMaxWidth()
                            .height(132.dp)
                            .verticalScroll(rememberScrollState())
                            .padding(12.dp),
                    ) {
                        Text(
                            tr("settings.appearance.textSizeSample"),
                            style = messageBodyTextStyle(MaterialTheme.typography.bodyLarge, scale),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text("A", style = MaterialTheme.typography.bodySmall)
                    Slider(
                        value = index.toFloat(),
                        // Slider snaps to the stops itself, but a snapped float
                        // can land a hair under its stop, which truncation
                        // would read as the one below.
                        onValueChange = { onScaleChange(steps[it.roundToInt().coerceIn(0, steps.lastIndex)]) },
                        valueRange = 0f..steps.lastIndex.toFloat(),
                        // One stop per step, minus the two endpoints.
                        steps = steps.size - 2,
                        modifier = Modifier.weight(1f),
                    )
                    Text("A", style = MaterialTheme.typography.titleLarge)
                }
                Text(
                    trf("settings.appearance.textSizeValue", steps[index]),
                    modifier = Modifier.align(Alignment.CenterHorizontally),
                    color = MaterialTheme.colorScheme.primary,
                )
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text(tr("buttons.done")) } },
        dismissButton = {
            if (steps[index] != DEFAULT_MESSAGE_FONT_SCALE) {
                TextButton(onClick = { onScaleChange(DEFAULT_MESSAGE_FONT_SCALE) }) {
                    Text(tr("common.resetToDefault"))
                }
            }
        },
    )
}

@Composable
internal fun LanguagePickerDialog(
    currentTag: String,
    onSelect: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(tr("settings.language.label")) },
        text = {
            val listState = rememberLazyListState()
            LazyColumn(modifier = Modifier.heightIn(max = 480.dp).appScrollbar(listState), state = listState) {
                item {
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .clickable {
                                onSelect("")
                                onDismiss()
                            }.padding(vertical = 2.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = currentTag.isBlank(), onClick = {
                            onSelect("")
                            onDismiss()
                        })
                        Text(tr("settings.language.system"), modifier = Modifier.padding(start = 8.dp))
                    }
                }
                items(supportedAppLanguageTags) { tag ->
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .clickable {
                                onSelect(tag)
                                onDismiss()
                            }.padding(vertical = 2.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = tag == currentTag, onClick = {
                            onSelect(tag)
                            onDismiss()
                        })
                        Text(appLanguageDisplayName(tag), modifier = Modifier.padding(start = 8.dp))
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text(tr("buttons.done")) } },
    )
}

/**
 * Passphrase prompt shared by both halves of backup/restore.
 *
 * [BackupPassphraseMode.Export] collects a passphrase (and whether to include
 * account passwords); [BackupPassphraseMode.Restore] collects the passphrase
 * for a file that turned out to be encrypted. The caller owns the core call, so
 * this composable only gathers input.
 */
@Composable
internal fun BackupPassphraseDialog(
    mode: BackupPassphraseMode,
    busy: Boolean,
    error: String,
    onDismiss: () -> Unit,
    onConfirm: (passphrase: String, includeSecrets: Boolean) -> Unit,
) {
    var passphrase by remember { mutableStateOf("") }
    var confirmation by remember { mutableStateOf("") }
    var includeSecrets by remember { mutableStateOf(false) }

    val exporting = mode == BackupPassphraseMode.Export
    // Exporting: a passphrase is optional unless passwords are included, but
    // once typed it must be confirmed — a typo produces a file nobody can open.
    // Restoring: the passphrase is checked against the file immediately, so
    // there is nothing to confirm.
    val mismatched = exporting && passphrase != confirmation
    val missing = if (exporting) includeSecrets && passphrase.isEmpty() else passphrase.isEmpty()

    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = { Text(if (exporting) tr("settings.backup.exportTitle") else tr("settings.backup.restoreTitle")) },
        text = {
            val scrollState = rememberScrollState()
            Column(
                modifier = Modifier.appScrollbar(scrollState).verticalScroll(scrollState),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (exporting) {
                    Row(
                        Modifier.fillMaxWidth().clickable(enabled = !busy) { includeSecrets = !includeSecrets },
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Checkbox(checked = includeSecrets, onCheckedChange = { includeSecrets = it }, enabled = !busy)
                        Column(Modifier.weight(1f)) {
                            Text(tr("settings.backup.includeSecrets"), style = MaterialTheme.typography.bodyMedium)
                            Text(
                                tr("settings.backup.includeSecretsHint"),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
                OutlinedTextField(
                    value = passphrase,
                    onValueChange = { passphrase = it },
                    label = { Text(tr("settings.backup.passphrase")) },
                    singleLine = true,
                    enabled = !busy,
                    visualTransformation = PasswordVisualTransformation(),
                    modifier = Modifier.fillMaxWidth(),
                )
                if (exporting) {
                    OutlinedTextField(
                        value = confirmation,
                        onValueChange = { confirmation = it },
                        label = { Text(tr("settings.backup.passphraseConfirm")) },
                        singleLine = true,
                        enabled = !busy,
                        isError = mismatched && confirmation.isNotEmpty(),
                        visualTransformation = PasswordVisualTransformation(),
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
                Text(
                    if (exporting) tr("settings.backup.passphraseHint") else tr("settings.backup.restoreHint"),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (mismatched && confirmation.isNotEmpty()) {
                    Text(
                        tr("settings.backup.passphraseMismatch"),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
                if (error.isNotEmpty()) {
                    Text(error, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onConfirm(passphrase, exporting && includeSecrets) },
                enabled = !busy && !missing && !mismatched,
            ) {
                Text(if (exporting) tr("common.export") else tr("settings.backup.restoreAction"))
            }
        },
        dismissButton = { TextButton(onClick = onDismiss, enabled = !busy) { Text(tr("buttons.cancel")) } },
    )
}
