// Opens the conversation or mailbox referenced by a notification.
// Mobile notification payloads identify an account and a thread, sender or folder.
// Navigation resolves that identity before selecting the destination.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.notificationThreadId
import jp.nonbili.meron.shared.parseAccountListResponse
import jp.nonbili.meron.shared.threadIdIsRss
import kotlinx.coroutines.withContext

/**
 * Opens the conversation a tapped notification names, layered over whatever the user was looking at:
 * the mailbox behind it keeps its account, folder, search and filter, so backing out returns to the
 * Mail or Kanban view the tap interrupted rather than to the notification's own folder. The thread is
 * opened directly by composite id without searching the mailbox.
 */
internal fun MeronMobileState.openNotificationThread(target: NotificationThreadTarget) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    if (target.threadKey.isBlank()) {
        openNotificationMailbox(target)
        return
    }
    val threadId = notificationThreadId(target.accountId, target.folder, target.threadKey)
    val address = target.fromAddr.trim().lowercase()
    if (mailGroupBySender && address.isNotBlank() && !threadIdIsRss(threadId)) {
        openNotificationSenderChat(target, address)
        return
    }
    openLinkedThread(threadId, target.accountId)
}

/** With the list grouped by sender, a new-mail tap opens that person's chat
 *  (`sender:<address>` in the notification's folder) instead of the thread.
 *  `address` is the notified message's own sender, carried by the
 *  notification. */
private fun MeronMobileState.openNotificationSenderChat(
    target: NotificationThreadTarget,
    address: String,
) {
    val senderId = notificationThreadId(target.accountId, target.folder, "sender:$address")
    // The list's own row when it is loaded: it knows the person's name,
    // unread count and whether they are a no-reply address.
    val row =
        coreThreads.firstOrNull { it.id == senderId }
            ?: ThreadSummary(
                id = senderId,
                threadId = senderId,
                accountId = target.accountId,
                folder = if (target.folder.equals("inbox", ignoreCase = true)) "INBOX" else target.folder,
                subject = "",
                sender = address,
                senderAddress = address,
                group = "sender",
            )
    readCoreThread(row)
}

/** A group summary names an account and folder but no conversation, so this one
 *  does navigate the mailbox. Hiding an account from the side nav only drops it
 *  from the drawer list; its mail is still reachable, including from here. */
private fun MeronMobileState.openNotificationMailbox(target: NotificationThreadTarget) {
    mailSearch = ""
    mailFilter = FilterMode.All
    mailAttachmentsOnly = false
    selectedCoreAccountId = target.accountId
    selectedCoreFolder = target.folder
    syncing = true
    scope.launchThreadListRead(threadRemovalGuard) { read ->
        val folderReadVersion = folderReadGuard.version
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                val accounts =
                    coreAccounts.takeIf { accounts -> accounts.any { it.id == target.accountId } }
                        ?: parseAccountListResponse(client.listAccounts())
                val account =
                    accounts.firstOrNull { it.id == target.accountId }
                        ?: error("Account not found: ${target.accountId}")
                val result =
                    loadAccountInbox(
                        client = client,
                        account = account,
                        requestedFolder = target.folder,
                        query = "",
                        filter = FilterMode.All,
                        groupBySender = mailGroupBySender,
                        syncFirst = false,
                    )
                accounts to result
            }
        }.onSuccess { (accounts, result) ->
            if (coreAccounts.isEmpty()) {
                coreAccounts = accounts
            }
            coreFolders = reconcileFolderUnread(result.folders, folderReadVersion)
            if (result.folders.isNotEmpty()) {
                foldersByAccount = foldersByAccount + reconcileFolderUnread(result.folders, folderReadVersion).groupBy { it.accountId }
            }
            selectedCoreFolder = result.folder
            coreThreads = withLocalDraftFlags(read.filter(result.threads))
            visibleMailboxKey = mailboxCacheKey(target.accountId, result.folder, "", FilterMode.All, groupBySender = mailGroupBySender)
            mailboxCursor = result.nextCursor
            mailboxAccountCursors = result.accountCursors
            mailboxPageDepth = MAILBOX_PAGE_SIZE
            mailboxHeldThreads = emptyList()
            syncing = false
            initialThreadsLoaded = true
            selectedMailThreadIds = emptySet()
            // Loading the mailbox is not enough on its own: nothing else on this
            // path moves the app off the screen it was on, so the tap would land
            // behind Settings, Kanban, Compose, or a thread left open.
            selectedCoreThread = null
            previousTopScreen = Screen.Mail
            screen = Screen.Mail
        }.onFailure {
            syncing = false
            status = trsError("compose.toast.couldNotOpenNotificationThread", it)
        }
    }
}
