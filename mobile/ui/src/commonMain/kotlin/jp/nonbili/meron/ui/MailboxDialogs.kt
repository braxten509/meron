// Presents the dialogs and confirmations shared by mobile mailbox screens.
// MeronApp supplies the current state and the navigation callback.
// Each dialog delegates changes to the existing state actions.
package jp.nonbili.meron.ui

import androidx.compose.runtime.Composable
import jp.nonbili.meron.shared.accountSummaryIsRss

@Composable
internal fun MailboxDialogs(
    state: MeronMobileState,
    appVersion: String,
    activeKanbanBoard: KanbanBoardSpec?,
    popAppBack: () -> Unit,
) {
    with(state) {
        if (showAddFeedDialog) {
            AddFeedDialog(
                url = addFeedUrl,
                onUrlChange = {
                    addFeedUrl = it
                    addFeedError = ""
                },
                error = addFeedError,
                submitting = addFeedSubmitting,
                onAdd = ::addFeedToSelectedRssAccount,
                onDismiss = {
                    if (!addFeedSubmitting) {
                        showAddFeedDialog = false
                        addFeedError = ""
                    }
                },
            )
        }

        selectedTagThread?.let { thread ->
            TagDialog(thread = thread, onDismiss = { selectedTagThread = null })
        }

        if (showTagsDialog) {
            TagsDialog(onDismiss = { showTagsDialog = false })
        }

        selectedMailMoveThread?.let { thread ->
            MoveThreadDialog(
                thread = thread,
                accounts = coreAccounts.filterNot { accountSummaryIsRss(it) },
                folders =
                    coreAccounts
                        .filterNot { accountSummaryIsRss(it) }
                        .flatMap { account -> foldersByAccount[account.id].orEmpty() },
                recentTransfers = recentTransferItems(thread),
                onMove = { folder ->
                    selectedMailMoveThread = null
                    moveThreadToFolder(thread, folder.name, folder.accountId.ifBlank { thread.accountId }) {
                        selectedMailThreadIds = emptySet()
                    }
                },
                onCreateAndMove = { name ->
                    selectedMailMoveThread = null
                    createFolderAndMoveThread(thread, name) {
                        selectedMailThreadIds = emptySet()
                    }
                },
                onDismiss = { selectedMailMoveThread = null },
            )
        }

        selectedMailCopyThread?.let { thread ->
            CopyThreadDialog(
                thread = thread,
                accounts = coreAccounts.filterNot { accountSummaryIsRss(it) },
                folders =
                    coreAccounts
                        .filterNot { accountSummaryIsRss(it) }
                        .flatMap { account -> foldersByAccount[account.id].orEmpty() },
                recentTransfers = recentTransferItems(thread),
                onCopy = { folder ->
                    selectedMailCopyThread = null
                    copyThreadToFolder(thread, folder)
                    selectedMailThreadIds = emptySet()
                },
                onDismiss = { selectedMailCopyThread = null },
            )
        }

        meronMoveThreads?.let { threads ->
            MeronFolderPickerDialog(
                threads = threads,
                // The built-in Archive is filled by archiving, never by hand.
                folders = meronFolders.filterNot(::isMeronArchiveFolder),
                onPick = { folder ->
                    meronMoveThreads = null
                    // Like Move to: a conversation moved from its own screen closes it.
                    val fromOpenThread = screen == Screen.Thread && threads.any { it.id == selectedCoreThread?.id }
                    moveToMeronFolder(threads, folder)
                    selectedMailThreadIds = emptySet()
                    if (fromOpenThread) popAppBack()
                },
                onNewFolder = {
                    meronMoveThreads = null
                    promptNewMeronFolder(threads)
                },
                onDismiss = { meronMoveThreads = null },
            )
        }

        when (val dialog = meronFolderDialog) {
            is MeronFolderDialog.Delete -> {
                DeleteMeronFolderDialog(
                    folderName = dialog.folder.name,
                    onConfirm = { deleteMeronFolder(dialog.folder) },
                    onDismiss = { meronFolderDialog = null },
                )
            }

            is MeronFolderDialog.New, is MeronFolderDialog.Rename -> {
                val rename = dialog is MeronFolderDialog.Rename
                MeronFolderNameDialog(
                    title = tr(if (rename) "meronFolders.renameTitle" else "meronFolders.new"),
                    confirmLabel = tr(if (rename) "buttons.save" else "meronFolders.create"),
                    name = meronFolderNameInput,
                    error = meronFolderNameError,
                    busy = meronFolderBusy,
                    onNameChange = {
                        meronFolderNameInput = it
                        meronFolderNameError = ""
                    },
                    onConfirm = ::submitMeronFolderName,
                    onDismiss = { if (!meronFolderBusy) meronFolderDialog = null },
                )
            }

            null -> {}
        }

        pendingEmptyFolder?.let { target ->
            EmptyFolderDialog(
                folderName = target.folderName,
                onConfirm = {
                    pendingEmptyFolder = null
                    emptyMailFolder(
                        accountId = target.accountId,
                        folderId = target.folderId,
                        column = target.column,
                    )
                },
                onDismiss = { pendingEmptyFolder = null },
            )
        }

        pendingDeleteFolder?.let { target ->
            DeleteFolderDialog(
                folderName = target.folderName,
                nested = target.nested,
                onConfirm = {
                    pendingDeleteFolder = null
                    deleteMailFolder(
                        accountId = target.accountId,
                        folderId = target.folderId,
                        column = target.column,
                    )
                },
                onDismiss = { pendingDeleteFolder = null },
            )
        }

        certPrompt?.let { prompt ->
            CertificateTrustDialog(
                server = "${prompt.host}:${prompt.port}",
                certificate = prompt.certificate,
                busy = certPromptBusy,
                onTrust = ::trustPromptedCertificate,
                onDismiss = ::dismissCertificatePrompt,
            )
        }

        if (showAboutDialog) {
            AboutDialog(
                appVersion = appVersion,
                onOpenUrl = services::openUrl,
                onShowChangelog = {
                    showAboutDialog = false
                    showChangelogDialog = true
                    loadChangelog()
                },
                onDismiss = { showAboutDialog = false },
            )
        }

        if (pendingCrashReport.isNotBlank()) {
            CrashReportDialog(
                summary = pendingCrashReport,
                onSend = {
                    pendingCrashReport = ""
                    mobileHost.clearPendingCrashReport()
                    mobileHost.shareDiagnosticLog()
                },
                onDismiss = {
                    pendingCrashReport = ""
                    mobileHost.clearPendingCrashReport()
                },
            )
        }

        if (showChangelogDialog) {
            ChangelogDialog(
                releases = changelog,
                loading = changelogLoading,
                error = changelogError,
                onDismiss = { showChangelogDialog = false },
            )
        }

        imagePreview?.let { preview ->
            ImagePreviewDialog(
                preview = preview,
                onShare = { shareImagePreview(preview) },
                onCopy = { copyImagePreview(preview) },
                onDismiss = { imagePreview = null },
            )
        }

        if (showKanbanColumnDialog && screen == Screen.Kanban) {
            KanbanColumnDialog(
                accounts = coreAccounts,
                board = activeKanbanBoard,
                foldersByAccount = foldersByAccount,
                onApply = {
                    applyKanbanColumns(it)
                    showKanbanColumnDialog = false
                },
                onCreateFolder = { promptNewFolder(it, NewFolderPlacement.AddColumn(activeKanbanBoardId)) },
                onDismiss = { showKanbanColumnDialog = false },
            )
        }

        pendingNewFolder?.let { target ->
            FolderNameDialog(
                title = tr("folders.newFolderOrLabel"),
                confirmLabel = tr("folders.create"),
                name = folderNameInput,
                hint =
                    tr(
                        "folders.subfolderHint",
                        mapOf("delimiter" to folderTreeDelimiter(foldersByAccount[target.account.id].orEmpty())),
                    ),
                onNameChange = { folderNameInput = it },
                onConfirm = { createMailFolder(target, folderNameInput) },
                onDismiss = { pendingNewFolder = null },
            )
        }

        pendingRenameFolder?.let { target ->
            FolderNameDialog(
                title = tr("folders.rename.title", mapOf("folder" to target.folderName)),
                confirmLabel = tr("folders.rename.confirmButton"),
                name = folderNameInput,
                onNameChange = { folderNameInput = it },
                onConfirm = { renameMailFolder(target, folderNameInput) },
                onDismiss = { pendingRenameFolder = null },
            )
        }

        kanbanActionThread?.let { thread ->
            KanbanThreadActionDialog(
                thread = thread,
                board = activeKanbanBoard,
                accounts = coreAccounts,
                onDismiss = { kanbanActionThread = null },
                onOpen = {
                    kanbanActionThread = null
                    readCoreThread(thread)
                },
                onToggleStar = {
                    kanbanActionThread = null
                    toggleStar(thread)
                },
                onToggleRead = {
                    kanbanActionThread = null
                    toggleRead(thread)
                },
                onArchive = {
                    kanbanActionThread = null
                    archiveOrRemove(thread)
                },
                onDelete = {
                    kanbanActionThread = null
                    deleteThread(thread)
                },
                onTags = {
                    kanbanActionThread = null
                    selectedTagThread = thread
                },
                onAddTag = { tag ->
                    kanbanActionThread = null
                    changeThreadTags(thread, add = listOf(tag)) { tags ->
                        status = trs(if (tags == null) "tags.invalid" else "tags.added", mapOf("tag" to tag))
                    }
                },
                onCopyFeedUrl = {
                    kanbanActionThread = null
                    services.copyText(trs("chat.feedUrl"), thread.feedUrl)
                    status = trs("common.copied")
                },
                onMove = { target ->
                    kanbanActionThread = null
                    moveThreadToColumn(thread, target)
                },
            )
        }
    }
}
