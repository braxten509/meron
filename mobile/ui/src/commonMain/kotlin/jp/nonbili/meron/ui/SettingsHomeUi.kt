// Lists settings pages, accounts and boards on mobile.
// Selecting an account or board identifies the target before opening its settings.
// The settings navigator supplies callbacks for adding accounts and opening pages.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.FilterList
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.ViewKanban
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.runtime.Composable
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.accountSummaryIsRss

@OptIn(ExperimentalMaterial3Api::class, ExperimentalComposeUiApi::class)
@Composable
internal fun SettingsHome(
    accounts: List<AccountSummary>,
    hiddenNavigationAccountIds: Set<String>,
    kanbanBoards: List<KanbanBoardSpec>,
    onAddMailAccount: () -> Unit,
    onAddFeedAccount: () -> Unit,
    onCreateKanbanBoard: () -> String,
    navigate: (String) -> Unit,
    onSelectAccount: (String) -> Unit,
    onSelectBoard: (String) -> Unit,
) {
    // Mirrors the desktop Settings sidebar: General, Messages and
    // Composer, then Kanban boards, Mail accounts, and Feed accounts.
    val mailAccounts = accounts.filter { !accountSummaryIsRss(it) }
    val feedAccounts = accounts.filter { accountSummaryIsRss(it) }
    val rootListState = rememberLazyListState()
    LazyColumn(Modifier.fillMaxSize().appScrollbar(rootListState), state = rootListState) {
        item {
            SettingsRow(
                icon = Icons.Filled.Settings,
                title = tr("settings.sections.general"),
                subtitle = null,
                onClick = { navigate(SettingsRoutes.General) },
            )
        }
        item {
            SettingsRow(
                icon = Icons.AutoMirrored.Filled.Chat,
                title = tr("settings.sections.messages"),
                subtitle = null,
                onClick = { navigate(SettingsRoutes.Messages) },
            )
        }
        item {
            SettingsRow(
                icon = Icons.Filled.Edit,
                title = tr("settings.sections.composer"),
                subtitle = null,
                onClick = { navigate(SettingsRoutes.Composer) },
            )
        }
        item {
            SettingsRow(
                icon = Icons.Filled.FilterList,
                title = tr("rules.title"),
                subtitle = null,
                onClick = { navigate(SettingsRoutes.Rules) },
            )
        }
        item {
            SettingsRow(
                icon = Icons.Filled.AutoAwesome,
                title = tr("aiSort.title"),
                subtitle = null,
                onClick = { navigate(SettingsRoutes.AiSort) },
            )
        }
        item { SettingsSectionLabel(tr("settings.sections.kanbanBoards")) }
        items(kanbanBoards, key = { it.id }) { board ->
            SettingsRow(
                icon = Icons.Filled.ViewKanban,
                leading = { KanbanBoardTile(board, 40.dp) },
                title = board.name,
                subtitle = tr("settings.kanban.boardColumns", mapOf("count" to board.columns.size)),
                onClick = {
                    onSelectBoard(board.id)
                    navigate(SettingsRoutes.KanbanBoard)
                },
            )
        }
        item {
            SettingsRow(
                icon = Icons.Filled.Add,
                title = tr("settings.kanban.newBoard"),
                subtitle = null,
                onClick = {
                    onSelectBoard(onCreateKanbanBoard())
                    navigate(SettingsRoutes.KanbanBoard)
                },
            )
        }
        item { SettingsSectionLabel(tr("settings.sections.mailAccounts")) }
        if (mailAccounts.isEmpty()) {
            item { SettingsEmptyLabel(tr("settings.sections.noMailAccounts")) }
        } else {
            items(mailAccounts, key = { it.id }) { account ->
                SettingsAccountRow(
                    account = account,
                    hidden = account.id in hiddenNavigationAccountIds,
                    onClick = {
                        onSelectAccount(account.id)
                        navigate(SettingsRoutes.Account)
                    },
                )
            }
        }
        item {
            SettingsRow(
                icon = Icons.Filled.Add,
                title = tr("settings.account.addMailAccount"),
                subtitle = null,
                onClick = onAddMailAccount,
            )
        }
        item { SettingsSectionLabel(tr("settings.sections.feedAccounts")) }
        if (feedAccounts.isEmpty()) {
            item { SettingsEmptyLabel(tr("settings.sections.noFeedAccounts")) }
        } else {
            items(feedAccounts, key = { it.id }) { account ->
                SettingsAccountRow(
                    account = account,
                    hidden = account.id in hiddenNavigationAccountIds,
                    fallbackTitle = tr("accounts.rssAtomFeeds"),
                    showAccountIdFallback = false,
                    showDefaultSubtitle = false,
                    onClick = {
                        onSelectAccount(account.id)
                        navigate(SettingsRoutes.Account)
                    },
                )
            }
        }
        item {
            SettingsRow(
                icon = Icons.Filled.Add,
                title = tr("mobile.accounts.addRssAccount"),
                subtitle = null,
                onClick = onAddFeedAccount,
            )
        }
        item { Spacer(Modifier.height(24.dp)) }
    }
}
