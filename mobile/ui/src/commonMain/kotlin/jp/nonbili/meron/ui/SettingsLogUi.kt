// Renders the on-device diagnostic log in settings.
// The settings screen passes the lines read by the platform host.
// Long log lines are capped for display.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp

/** In-app viewer for the on-device diagnostic log, so a user can inspect what
 *  happened before deciding to share it (sharing lives in the top bar). */
@Composable
internal fun SettingsSyncLogPage(
    logLines: List<String>?,
    modifier: Modifier = Modifier,
) {
    val listState = rememberLazyListState()
    // The newest entries are at the end; start there.
    LaunchedEffect(logLines) {
        if (!logLines.isNullOrEmpty()) listState.scrollToItem(logLines.lastIndex)
    }
    when {
        logLines == null -> {
            Box(modifier)
        }

        logLines.isEmpty() -> {
            Text(tr("settings.syncLogEmpty"), color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = modifier.padding(16.dp))
        }

        else -> {
            SelectionContainer {
                LazyColumn(
                    modifier.appScrollbar(listState),
                    state = listState,
                    contentPadding = PaddingValues(16.dp),
                ) {
                    items(logLines) { line ->
                        Text(line, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace)
                    }
                }
            }
        }
    }
}

private const val MAX_LOG_LINE_CHARS = 2_000

/** One row per log line, each capped so a runaway line can't stall layout. */
internal fun diagnosticLogLines(text: String): List<String> =
    text
        .trimEnd()
        .lines()
        .takeIf { it.any { line -> line.isNotBlank() } }
        ?.map { if (it.length > MAX_LOG_LINE_CHARS) it.take(MAX_LOG_LINE_CHARS) + "…" else it }
        .orEmpty()
