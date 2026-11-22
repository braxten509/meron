// Lists the models available to the mobile sorting settings.
// The picker shows model details and reports the choice to the settings state.
// Section headings and model rows use the same selection and dismissal callbacks.

package jp.nonbili.meron.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import jp.nonbili.meron.shared.AiModel
import jp.nonbili.meron.shared.AiModelList

/**
 * Full-screen model list with a search field. Each row shows about what 1,000
 * new emails cost and flags thinking models, which may be too slow. If the list
 * can't load, a model id can still be typed in.
 */
@Composable
internal fun AiModelPicker(
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
