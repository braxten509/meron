// Shows the sorting history and the filing lessons behind it.
// The mobile settings page supplies account and folder labels for each decision.
// The history and lesson rows expose available actions for their entries.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Undo
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.AiSortFolder
import jp.nonbili.meron.shared.AiSortLesson
import jp.nonbili.meron.shared.AiSortLogEntry
import jp.nonbili.meron.shared.AiSortStatus
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

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
internal fun MeronMobileState.aiFolderLabel(
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
            color =
                if (failed) {
                    colors.errorContainer
                } else if (moved) {
                    colors.secondaryContainer
                } else {
                    colors.surfaceVariant
                },
            shape = RoundedCornerShape(50),
            modifier = Modifier.padding(start = 8.dp).widthIn(max = 140.dp),
        ) {
            Text(
                tag,
                Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
                fontSize = 11.sp,
                color =
                    if (failed) {
                        colors.onErrorContainer
                    } else if (moved) {
                        colors.onSecondaryContainer
                    } else {
                        colors.onSurfaceVariant
                    },
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
internal fun AiSortLessonRow(
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
