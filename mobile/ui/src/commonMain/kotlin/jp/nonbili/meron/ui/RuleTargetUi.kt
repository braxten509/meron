// Renders the target of a mobile mail rule and describes its conditions.
// Account and folder labels are resolved from the current mail state.
// The rule editor uses these rows when displaying or changing a destination.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.Rule
import jp.nonbili.meron.shared.SearchFields

internal fun accountLabel(account: AccountSummary): String = account.displayName.ifEmpty { account.email }

/** How a rule's target reads: its account too when there is more than one. */
internal fun MeronMobileState.targetLabel(
    accounts: List<AccountSummary>,
    accountId: String,
    folder: String,
): String {
    val name = foldersByAccount[accountId]?.firstOrNull { it.name == folder }?.displayName ?: folder
    val account = accounts.firstOrNull { it.id == accountId }
    return if (accounts.size > 1 && account != null) "${accountLabel(account)} / $name" else name
}

@Composable
internal fun MeronMobileState.moveLabel(
    accounts: List<AccountSummary>,
    rule: Rule,
): String =
    when (rule.moveTo) {
        "archive" -> tr("folders.roles.archive")
        "trash" -> tr("folders.roles.trash")
        "folder" -> if (rule.folderAccount.isEmpty()) rule.folder else targetLabel(accounts, rule.folderAccount, rule.folder)
        else -> tr("rules.moveNone")
    }

@Composable
internal fun MeronMobileState.ruleSummary(
    accounts: List<AccountSummary>,
    rule: Rule,
): String =
    listOfNotNull(
        tr("rules.summaryTags", mapOf("tags" to rule.tags.joinToString(", "))).takeIf { rule.tags.isNotEmpty() },
        tr("notification.actionMarkRead").takeIf { rule.markRead },
        tr("chat.star").takeIf { rule.star },
        tr("rules.summaryCopy", mapOf("folder" to targetLabel(accounts, rule.copyAccount, rule.copyFolder)))
            .takeIf { rule.copyFolder.isNotEmpty() },
        tr("rules.summaryMove", mapOf("folder" to moveLabel(accounts, rule))).takeIf { rule.moveTo.isNotEmpty() },
    ).joinToString(" · ")

/** A rule's target: `options` first, then every folder of every account that [include] keeps. */
@Composable
internal fun MeronMobileState.RuleTargetRow(
    title: String,
    value: String,
    accounts: List<AccountSummary>,
    options: List<Pair<String, () -> Unit>>,
    include: (FolderSummary) -> Boolean = { true },
    /** AI sorting only: Meron folders listed first under their own heading
     *  ([meronChosen] greyed out). Rules leave this null. */
    meronOptions: List<jp.nonbili.meron.shared.MeronFolderSummary>? = null,
    meronChosen: Set<String> = emptySet(),
    onPickMeron: (jp.nonbili.meron.shared.MeronFolderSummary) -> Unit = {},
    onPick: (accountId: String, folder: String) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    // With several accounts, the one whose folders are showing.
    var opened by remember { mutableStateOf<AccountSummary?>(null) }

    fun close() {
        open = false
        opened = null
    }
    val folderAccount = opened ?: accounts.singleOrNull()
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(title, Modifier.weight(1f))
        TextButton(onClick = { open = true }) { Text(value, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        DropdownMenu(expanded = open, onDismissRequest = ::close) {
            if (opened != null) {
                DropdownMenuItem(text = { Text(tr("buttons.back")) }, onClick = { opened = null })
            } else {
                options.forEach { (label, pick) ->
                    DropdownMenuItem(
                        text = { Text(label) },
                        onClick = {
                            close()
                            pick()
                        },
                    )
                }
                if (meronOptions != null) {
                    DropdownMenuItem(
                        text = {
                            Text(
                                tr("meronFolders.title").uppercase(),
                                style = MaterialTheme.typography.labelSmall,
                                fontWeight = FontWeight.Bold,
                            )
                        },
                        enabled = false,
                        onClick = {},
                    )
                    // The built-in Archive is never a target.
                    val meronTargets = meronOptions.filterNot(::isMeronArchiveFolder)
                    if (meronTargets.isEmpty()) {
                        DropdownMenuItem(text = { Text(tr("meronFolders.none")) }, enabled = false, onClick = {})
                    }
                    meronTargets.forEach { folder ->
                        DropdownMenuItem(
                            text = { Text(folder.name) },
                            leadingIcon = { Icon(Icons.Outlined.Folder, contentDescription = null, modifier = Modifier.size(20.dp)) },
                            enabled = folder.id !in meronChosen,
                            onClick = {
                                close()
                                onPickMeron(folder)
                            },
                        )
                    }
                    HorizontalDivider(Modifier.padding(vertical = 4.dp))
                }
            }
            if (folderAccount != null) {
                foldersByAccount[folderAccount.id].orEmpty().filter(include).forEach { folder ->
                    DropdownMenuItem(
                        text = { Text(folder.displayName) },
                        onClick = {
                            close()
                            onPick(folderAccount.id, folder.name)
                        },
                    )
                }
            } else {
                accounts.forEach { account ->
                    DropdownMenuItem(
                        text = { Text(accountLabel(account)) },
                        leadingIcon = {
                            AccountBadgeAvatar(
                                label = accountLabel(account),
                                avatarUrl = account.avatarUrl,
                                size = 20.dp,
                                fallbackIcon = accountAvatarFallbackIcon(account),
                            )
                        },
                        onClick = { opened = account },
                    )
                }
            }
        }
    }
}

/** A rule's conditions in the form's own words, not as search text. */
@Composable
internal fun ruleCondition(fields: SearchFields): String {
    @Composable
    fun valued(
        key: String,
        value: String,
    ) = "${tr(key)}: ${value.trim()}".takeIf { value.isNotBlank() }
    return listOfNotNull(
        tr("rules.allMail").takeIf { fields == SearchFields() },
        valued("composer.fields.from", fields.from),
        valued("composer.fields.to", fields.to),
        valued("composer.fields.subject", fields.subject),
        valued("tags.title", fields.tags),
        valued("search.hasWords", fields.words),
        valued("search.after", fields.after),
        valued("search.before", fields.before),
        tr("filters.unread").takeIf { fields.unread },
        tr("filters.starred").takeIf { fields.starred },
        tr("filters.hasAttachment").takeIf { fields.hasAttachment },
    ).joinToString(" · ")
}
