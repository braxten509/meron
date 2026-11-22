// Applies mail actions to grouped selections across folders and accounts.
// The bulk operations report progress and reconcile completed groups.
// The selection remains identifiable during moves, copies, archival and deletion.

import { rememberThreadTransfer } from './recentThreadTransfers'
import { isLocalAccountId } from './accounts'
import { invoke } from '../lib/bridge'
import { t } from '../lib/i18n'
import { mailTransferError } from '../lib/mailTransferErrors'
import { clearBulkSelection, confirmAction, showToast, type BulkSelectionItem } from './ui'
import { BULK_GROUP_SIZE, bulkJobRunning, runBulkJob, type BulkJobResult } from './bulkProgress'
import { MERON_ARCHIVE_ID, scheduleMeronFoldersRefresh } from './meronFolders'
import { findLocalThread, uniqueThreadItems } from './mail'
import {
  applyMutationFolderUnreads,
  deletesForever,
  loadFolders,
  type MutationResult,
  refreshAccountFoldersCache,
} from './mailFolders'
import { assertMoveAffected, assertCopyAffected, archiveFailureText, assertDeleteAffected } from './mailMoveResults'
import { removeThreadLocally, refreshThreadLocation } from './mailMoveSelection'

/** Only one bulk Delete / Archive / Move runs at a time. */
function bulkJobBusy(): boolean {
  if (!bulkJobRunning()) return false
  showToast(t('threads.bulkProgress.busy'), 'error')
  return true
}

/**
 * One group of a bulk Delete or Move: ask the core for all of it in one call
 * (it does the server's work once for the whole group), then settle each
 * conversation by its own answer. `settle` throws for an answer that did
 * nothing. Once every answer is settled, throws the first failure, if any.
 */
async function invokeGroup<T extends BulkSelectionItem>(
  command: 'mail.deleteMany' | 'mail.moveMany',
  group: T[],
  payload: (item: T) => Record<string, unknown>,
  settle: (item: T, response: unknown) => void,
  done: (item: T) => void,
) {
  const response = await invoke<{ results?: unknown[] }>(command, { items: group.map(payload) })
  const results = Array.isArray(response?.results) ? response.results : []
  let failure: unknown
  group.forEach((item, index) => {
    try {
      const answer = results[index]
      if (answer === undefined) throw new Error('The mail engine gave no answer')
      const failed = answer && typeof answer === 'object' ? (answer as { error?: unknown; ok?: unknown }) : null
      if (failed && failed.error != null && failed.ok === undefined) throw new Error(String(failed.error))
      settle(item, answer)
      done(item)
    } catch (error) {
      failure ??= error
    }
  })
  if (failure !== undefined) throw failure
}

/** After a bulk job: the toast for what happened. Finished, stopped by the
 *  reader, or failed part-way (then only the unreached conversations remain). */
function reportBulkJob(
  result: BulkJobResult,
  words: { finished: string; stopped: string; failed: (error: unknown) => string },
) {
  if (result.error !== undefined) {
    const message = words.failed(result.error)
    showToast(
      result.completed
        ? `${message} ${t('mail.transferError.bulkProgress', { completed: result.completed, total: result.total })}`
        : message,
      'error',
    )
  } else if (result.stopped) {
    showToast(words.stopped)
  } else {
    showToast(words.finished)
  }
}

export async function bulkArchiveSelected(items: BulkSelectionItem[]) {
  // Local folders have no Archive folder; conversations in the built-in
  // Archive Meron folder are archived already.
  const targets = uniqueThreadItems(items).filter(
    (item) =>
      !item.draft &&
      !item.trash &&
      !isLocalAccountId(item.accountId) &&
      findLocalThread(item.threadId)?.meron_folder_id !== MERON_ARCHIVE_ID,
  )
  if (targets.length === 0 || bulkJobBusy()) return
  // Each row leaves once the server has archived it; the ones still waiting
  // stay in the list (greyed), so a stop or a failure has nothing to put back.
  const result = await runBulkJob('archive', targets, async ([item], done) => {
    const sourceThread = findLocalThread(item.threadId)
    const response = await invoke('mail.archive', { thread_id: item.threadId })
    assertMoveAffected(response, 'Archive')
    removeThreadLocally(item.threadId).complete()
    applyMutationFolderUnreads(response as MutationResult)
    if (sourceThread?.account_id) void refreshAccountFoldersCache(sourceThread.account_id, false)
    done(item)
  })
  if (result.completed) {
    await refreshThreadLocation(undefined, false)
    // Archived conversations are filed in the built-in Archive folder too.
    scheduleMeronFoldersRefresh()
  }
  reportBulkJob(result, {
    finished: t('mail.toast.archivedCount', { count: targets.length }),
    stopped: t('mail.toast.bulkStopped.archived', { done: result.completed, total: result.total }),
    failed: archiveFailureText,
  })
}

export async function bulkMoveSelectedToFolder(
  items: BulkSelectionItem[],
  targetFolderId: string,
  targetAccountId?: string,
) {
  const targets = uniqueThreadItems(items).filter(
    (item) => (targetAccountId && item.accountId !== targetAccountId) || item.folderId !== targetFolderId,
  )
  if (targets.length === 0 || bulkJobBusy()) return
  // A group at a time: each row leaves once the server has moved its group.
  const result = await runBulkJob(
    'move',
    targets,
    (group, done) =>
      invokeGroup(
        'mail.moveMany',
        group,
        (item) => ({
          thread_id: item.threadId,
          target_folder_id: targetFolderId,
          ...(targetAccountId ? { target_account_id: targetAccountId } : {}),
        }),
        (item, response) => {
          assertMoveAffected(response)
          removeThreadLocally(item.threadId).complete()
          rememberThreadTransfer({
            action: 'move',
            sourceAccountId: item.accountId,
            targetAccountId: targetAccountId || item.accountId,
            targetFolderId,
          })
          applyMutationFolderUnreads(response as MutationResult)
        },
        done,
      ),
    BULK_GROUP_SIZE,
  )
  if (result.completed) {
    await refreshThreadLocation(result.error === undefined ? targets[0]?.accountId : undefined, false)
    if (targetAccountId) void loadFolders(targetAccountId, false)
  }
  reportBulkJob(result, {
    finished: t('mail.toast.movedCount', { count: targets.length }),
    stopped: t('mail.toast.bulkStopped.moved', { done: result.completed, total: result.total }),
    failed: (error) => mailTransferError(error, t('mail.toast.moveFailed')),
  })
}

export async function bulkCopySelectedToFolder(
  items: BulkSelectionItem[],
  targetAccountId: string,
  targetFolderId: string,
) {
  const targets = uniqueThreadItems(items)
  if (targets.length === 0) return
  let completed = 0
  try {
    for (const item of targets) {
      const response = await invoke('mail.copy', {
        thread_id: item.threadId,
        target_account_id: targetAccountId,
        target_folder_id: targetFolderId,
      })
      assertCopyAffected(response)
      completed++
      rememberThreadTransfer({ action: 'copy', sourceAccountId: item.accountId, targetAccountId, targetFolderId })
    }
    await refreshThreadLocation(targets[0]?.accountId, true)
    if (targetAccountId !== targets[0]?.accountId) void loadFolders(targetAccountId, false)
    clearBulkSelection()
    showToast(t('mail.toast.copiedCount', { count: targets.length }))
  } catch (error) {
    const message = mailTransferError(error, t('mail.toast.copyFailed'))
    showToast(
      completed ? `${message} ${t('mail.transferError.bulkProgress', { completed, total: targets.length })}` : message,
      'error',
    )
    if (completed) void refreshThreadLocation(undefined, false)
  }
}

export async function bulkDeleteSelected(items: BulkSelectionItem[]) {
  const targets = uniqueThreadItems(items)
  if (targets.length === 0 || bulkJobBusy()) return
  const permanentTargets = targets.filter(
    (item) => item.draft || item.trash || deletesForever(item.accountId, item.folderId),
  )
  if (permanentTargets.length > 0) {
    const confirmed = await confirmAction({
      title: permanentTargets.length === targets.length ? 'Delete selected forever?' : 'Delete selected threads?',
      message:
        permanentTargets.length === targets.length
          ? `${permanentTargets.length} selected thread(s) will be permanently deleted. This can't be undone.`
          : `${permanentTargets.length} selected thread(s) will be permanently deleted; the rest will move to Trash.`,
      confirmLabel: 'Delete selected',
      tone: 'danger',
    })
    if (!confirmed) return
  }

  // A group at a time: each row leaves once the server has deleted its group.
  const result = await runBulkJob(
    'delete',
    targets,
    (group, done) =>
      invokeGroup(
        'mail.deleteMany',
        group,
        (item) => ({ thread_id: item.threadId, ...(item.folderId ? { folder: item.folderId } : {}) }),
        (item, response) => {
          assertDeleteAffected(response)
          removeThreadLocally(item.threadId).complete()
          applyMutationFolderUnreads(response as MutationResult)
        },
        done,
      ),
    BULK_GROUP_SIZE,
  )
  if (result.completed) await refreshThreadLocation(undefined, false)
  reportBulkJob(result, {
    finished: t('mail.toast.deletedCount', { count: targets.length }),
    stopped: t('mail.toast.bulkStopped.deleted', { done: result.completed, total: result.total }),
    failed: (error) => (error instanceof Error ? error.message : t('mail.toast.deleteFailed')),
  })
}
