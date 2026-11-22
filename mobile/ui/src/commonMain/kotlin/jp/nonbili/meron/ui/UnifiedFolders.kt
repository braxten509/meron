// Resolves mobile unified folder roles to actual
// folders for each account. The mailbox can then
// load only the accounts that provide the requested
// role.

package jp.nonbili.meron.ui

import androidx.compose.runtime.Composable
import jp.nonbili.meron.shared.FolderSummary

/**
 * Present unified folders in the same order as a single account's folders.
 *
 * The folders the unified view offers, in the same order a single account lists its own — so switching
 * between an account and the unified view keeps the same muscle memory. These are roles, not folder names:
 * "Sent" here means each account's own Sent, which the core resolves per account, leaving out accounts
 * whose server has no such mailbox.
 *
 * Starred is absent here because it is not a mailbox any account owns — it is the cross-account starred
 * *items* listing, which resolves through the core's starred-items command rather than through a
 * per-account folder. It still sits in the switcher: see [UNIFIED_VIEW_ROLES].
 */
internal val UNIFIED_FOLDER_ROLES = listOf(INBOX_FOLDER, "sent", "drafts", "archive", "junk", "trash")

/**
 * The roles the unified view offers to switch between — its mailbox roles plus
 * starred, ordered as on desktop with starred next to the inbox. Shared by the
 * mail list and by a unified Kanban column, which show the same choices.
 */
internal val UNIFIED_VIEW_ROLES = listOf(INBOX_FOLDER, STARRED_FOLDER) + UNIFIED_FOLDER_ROLES.filterNot { it == INBOX_FOLDER }

/** Whether [folderId] selects the cross-account starred listing. */
internal fun isUnifiedStarredFolder(folderId: String): Boolean = folderId.equals(STARRED_FOLDER, ignoreCase = true)

/** Locale keys for the role names, shared with the desktop folder switcher. */
private val UNIFIED_ROLE_LABEL_KEYS =
    mapOf(
        INBOX_FOLDER to "folders.roles.inbox",
        "sent" to "folders.roles.sent",
        "drafts" to "folders.roles.drafts",
        "archive" to "folders.roles.archive",
        "junk" to "folders.roles.junk",
        "trash" to "folders.roles.trash",
    )

/** A selection that is not a unified role falls back to the inbox. */
internal fun unifiedFolderRole(folderId: String): String = UNIFIED_FOLDER_ROLES.firstOrNull { it.equals(folderId, ignoreCase = true) } ?: INBOX_FOLDER

/** Resolve a unified role to the real mailbox name used by one account. */
internal fun unifiedAccountFolder(
    folders: List<FolderSummary>,
    folderRole: String,
): String? {
    val role = unifiedFolderRole(folderRole)
    return folders.firstOrNull { it.role.equals(role, ignoreCase = true) }?.name
        ?: INBOX_FOLDER.takeIf { role == INBOX_FOLDER }
}

/**
 * The unified view's synthetic folder list, shown by the mail list's folder
 * switcher and by a unified column's. The folder *name* is the role, so a picked
 * entry round-trips straight back as the requested role.
 */
@Composable
internal fun unifiedFolders(): List<FolderSummary> =
    UNIFIED_VIEW_ROLES.map { role ->
        FolderSummary(
            accountId = UNIFIED_ACCOUNT_ID,
            name = role,
            role = role,
            // Starred has no folder-role name of its own; it reuses the filter
            // label, the same string the desktop switcher shows.
            displayName = if (role == STARRED_FOLDER) tr("filters.starred") else tr(UNIFIED_ROLE_LABEL_KEYS.getValue(role)),
        )
    }

/** The unified view's label for [folderId], e.g. "Sent". */
@Composable
internal fun unifiedFolderLabel(folderId: String): String = if (isUnifiedStarredFolder(folderId)) tr("filters.starred") else tr(UNIFIED_ROLE_LABEL_KEYS.getValue(unifiedFolderRole(folderId)))

/**
 * A unified Kanban column's own name. Unlike the mail list — where the unified mailbox is the whole
 * screen and the folder name alone is unambiguous — a column sits beside single-account columns whose
 * folders are called the same thing, so it has to carry "Unified" itself. Spelled out per role rather
 * than composed from "Unified" + a folder name, which no language agrees on.
 */
private val UNIFIED_COLUMN_LABEL_KEYS =
    mapOf(
        INBOX_FOLDER to "kanban.columns.unifiedInbox",
        STARRED_FOLDER to "kanban.columns.unifiedStarred",
        "sent" to "kanban.columns.unifiedSent",
        "drafts" to "kanban.columns.unifiedDrafts",
        "archive" to "kanban.columns.unifiedArchive",
        "junk" to "kanban.columns.unifiedJunk",
        "trash" to "kanban.columns.unifiedTrash",
    )

@Composable
internal fun unifiedColumnLabel(folderId: String): String = tr(UNIFIED_COLUMN_LABEL_KEYS.getValue(if (isUnifiedStarredFolder(folderId)) STARRED_FOLDER else unifiedFolderRole(folderId)))

/**
 * Whether a message stored in [messageFolder] is counted by the thread card shown for [cardFolder].
 *
 * A card's unread count is mailbox-scoped, so a thread that spans folders (an INBOX message and its
 * reply in Sent) must only take its own folder's reads off each card.
 *
 * Callers pass the mailbox read off the card's thread id; the role handling below is the fallback for
 * ids that carry none, since a card opened from a Kanban column has its `folder` replaced by the
 * column id — a role ("inbox", "sent") for a unified column, resolving to a different mailbox per
 * account.
 */
internal fun threadCardCoversFolder(
    cardFolder: String,
    accountFolders: List<FolderSummary>,
    messageFolder: String,
): Boolean =
    when {
        // The core leaves the folder blank for RSS items and for messages it
        // read out of the thread's own mailbox.
        messageFolder.isBlank() -> {
            true
        }

        kanbanFolderIdsEqual(cardFolder, messageFolder) -> {
            true
        }

        // Only reached when the card's own mailbox could not be read off its
        // thread id: a role resolves to this account's mailbox, and anything
        // else (a starred column id, say) covers nothing — leaving the count
        // untouched until the next sync beats clearing a card whose own
        // message is still unread.
        UNIFIED_FOLDER_ROLES.any { it.equals(cardFolder, ignoreCase = true) } -> {
            unifiedColumnMatchesFolder(cardFolder, accountFolders, messageFolder)
        }

        else -> {
            false
        }
    }

internal fun unifiedColumnMatchesFolder(
    columnFolderId: String,
    accountFolders: List<FolderSummary>,
    eventFolderId: String,
): Boolean {
    if (isUnifiedStarredFolder(columnFolderId)) return true
    val role = unifiedFolderRole(columnFolderId)
    if (eventFolderId.isBlank()) return role == INBOX_FOLDER
    val accountFolder = unifiedAccountFolder(accountFolders, role) ?: return false
    return kanbanFolderIdsEqual(accountFolder, eventFolderId)
}
