// Identifies pages in the mobile settings navigator.
// Routes carry the current account or board selection into a detail view.
// The settings host restores the corresponding page from the navigation stack.

package jp.nonbili.meron.ui

internal sealed class SettingsPage {
    data object Root : SettingsPage()

    data object General : SettingsPage()

    data object Messages : SettingsPage()

    data object Composer : SettingsPage()

    data object Rules : SettingsPage()

    data object AiSort : SettingsPage()

    data object AiSortLog : SettingsPage()

    data object SetupCopy : SettingsPage()

    data object RemoteSenders : SettingsPage()

    data object Theme : SettingsPage()

    data class AccountDetail(
        val accountId: String,
    ) : SettingsPage()

    data class AccountWallpaper(
        val accountId: String,
    ) : SettingsPage()

    data class KanbanBoardDetail(
        val boardId: String,
    ) : SettingsPage()

    data class KanbanBoardWallpaper(
        val boardId: String,
    ) : SettingsPage()

    data object SyncLog : SettingsPage()
}

internal object SettingsRoutes {
    const val Root = "settings/root"
    const val General = "settings/general"
    const val Messages = "settings/messages"
    const val Composer = "settings/composer"
    const val Rules = "settings/rules"
    const val AiSort = "settings/ai-sort"
    const val AiSortLog = "settings/ai-sort-log"
    const val SetupCopy = "settings/setup-copy"
    const val Account = "settings/account"
    const val AccountWallpaper = "settings/account-wallpaper"
    const val KanbanBoard = "settings/kanban-board"
    const val KanbanBoardWallpaper = "settings/kanban-board-wallpaper"
    const val SyncLog = "settings/sync-log"
    const val RemoteSenders = "settings/remote-senders"
    const val Theme = "settings/theme"
}
