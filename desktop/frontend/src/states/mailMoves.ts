// Coordinates mail transfers, deletion, archive and undo from desktop views.
// Message and folder identities from the core determine how local state is
// reconciled.
// Callers include meronFolders.ts and quickReply.ts.

import type { Message } from '../types'
import { rememberThreadTransfer } from './recentThreadTransfers'
import { isLocalAccountId } from './accounts'
import { invoke } from '../lib/bridge'
import { contextualErrorMessage } from '../lib/errors'
import { t } from '../lib/i18n'
import { mailTransferError } from '../lib/mailTransferErrors'
import { confirmAction, ui$, showToast, showUndoToast } from './ui'
import { MERON_ARCHIVE_ID, scheduleMeronFoldersRefresh } from './meronFolders'
import { isLocalSendId, discardPendingSend, cancelUnsentRescue } from './pendingSends'
import {
  findLocalThread,
  kanbanKeysWithThread,
  listFilterKey,
  loadThread,
  mail$,
  normalizeMessageId,
  reloadThreadCards,
  removeKanbanThread,
  releaseRemovedThread,
  requestThreadReselect,
  threadListViewKey,
  updateKanbanThread,
} from './mail'
import {
  applyMutationFolderUnreads,
  isDraftFolder,
  deletesForever,
  isTrashFolderId,
  loadFolders,
  type MutationResult,
} from './mailFolders'
import {
  type UndoRestore,
  threadAccountId,
  threadIdInFolder,
  type MovedCopiesResult,
  assertMoveAffected,
  movedCopyIds,
  undoRestore,
  assertCopyAffected,
  archiveFailureText,
  assertDeleteAffected,
  type MeronOnlyArchive,
  undoMeronOnlyArchive,
} from './mailMoveResults'
import { removeThreadLocally, refreshThreadLocation, restoreRemovedThread } from './mailMoveSelection'
export {
  bulkArchiveSelected,
  bulkMoveSelectedToFolder,
  bulkCopySelectedToFolder,
  bulkDeleteSelected,
} from './mailBulkMoves'

export { removeThreadLocally } from './mailMoveSelection'

// Moving, copying, archiving, and deleting threads and messages.

function reconcileThreadDraftFromLoadedMessages(threadId: string, messages: Message[]) {
  if (!threadId) return
  const threadMessages = messages.filter((message) => message.thread_id === threadId)
  if (threadMessages.some((message) => isDraftFolder(message.folder_id, message.account_id))) return

  mail$.threads.set(
    mail$.threads.get().map((thread) => (thread.thread_id === threadId ? { ...thread, has_draft: false } : thread)),
  )
  updateKanbanThread(threadId, (thread) => ({ ...thread, has_draft: false }))
}

// Thread ids encode the source folder, so a surviving row with the same id
// means the message copies are still in that folder — the server didn't
// actually apply the change, even though the call reported success. Checked
// after the post-action refresh so the toast reflects what the list shows.
function threadStillListed(threadId: string): boolean {
  return mail$.threads.get().some((item) => item.thread_id === threadId)
}

export async function moveThreadToFolder(
  threadId: string,
  targetFolderId: string,
  options: { undo?: boolean; messageIds?: string[]; targetAccountId?: string; restore?: UndoRestore } = {},
) {
  if (!threadId || !targetFolderId) return
  const sourceThread = findLocalThread(threadId)
  const sourceFolder = sourceThread?.folder_id ?? ''
  const sourceAccountId = sourceThread?.account_id || threadAccountId(threadId)
  const targetAccountId = options.targetAccountId || sourceAccountId
  const crossAccount = targetAccountId !== sourceAccountId
  if (!crossAccount && sourceFolder === targetFolderId) return
  const targetThreadId = threadIdInFolder(threadId, sourceThread?.account_id, targetFolderId)

  const { rollback, complete } = removeThreadLocally(threadId)
  try {
    const response = await invoke<MovedCopiesResult & MutationResult>('mail.move', {
      thread_id: threadId,
      target_folder_id: targetFolderId,
      ...(options.targetAccountId ? { target_account_id: options.targetAccountId } : {}),
      ...(options.messageIds ? { message_ids: options.messageIds } : {}),
      // An Undo: the core moves back exactly messageIds, or refuses.
      ...(options.restore
        ? {
            undo: true,
            ...(options.restore.showInInbox ? { show_in_inbox: true } : {}),
            ...(options.restore.meronFolderId
              ? { meron_folder_id: options.restore.meronFolderId, meron_source: options.restore.meronSource }
              : {}),
          }
        : {}),
    })
    assertMoveAffected(response)
    complete()
    applyMutationFolderUnreads(response)
    const copyIds = movedCopyIds(response)
    // The move already updated the cache. Reloading the first page in place
    // keeps the threads the reader had scrolled to; replacing the whole list
    // would drop them and can close the conversation that just became selected.
    await refreshThreadLocation(sourceThread?.account_id, false)
    if (crossAccount) void loadFolders(targetAccountId, false)
    if (threadStillListed(threadId)) {
      showToast(t('mail.toast.moveFailedInSameFolder'), 'error')
      return false
    } else {
      if (options.undo !== false && sourceAccountId) {
        rememberThreadTransfer({
          action: 'move',
          sourceAccountId,
          targetAccountId,
          targetFolderId,
        })
      }
      if (!crossAccount && options.undo !== false && sourceFolder && copyIds) {
        const restore = undoRestore(sourceThread, response)
        showUndoToast(t('mail.toast.threadMoved'), () => {
          releaseRemovedThread(threadId)
          void moveThreadToFolder(targetThreadId, sourceFolder, { undo: false, messageIds: copyIds, restore })
        })
      } else {
        showToast(t('mail.toast.threadMoved'))
      }
    }
    return true
  } catch (error) {
    rollback()
    showToast(mailTransferError(error, t('mail.toast.moveFailed')), 'error')
    if (crossAccount) void loadFolders(targetAccountId, false)
    return false
  }
}

export async function copyThreadToFolder(threadId: string, targetAccountId: string, targetFolderId: string) {
  if (!threadId || !targetAccountId || !targetFolderId) return
  const sourceThread = findLocalThread(threadId)
  const sourceAccountId = sourceThread?.account_id || threadAccountId(threadId)
  try {
    const response = await invoke('mail.copy', {
      thread_id: threadId,
      target_account_id: targetAccountId,
      target_folder_id: targetFolderId,
    })
    assertCopyAffected(response)
    if (sourceAccountId) {
      rememberThreadTransfer({
        action: 'copy',
        sourceAccountId,
        targetAccountId,
        targetFolderId,
      })
    }
    await refreshThreadLocation(sourceThread?.account_id, true)
    if (targetAccountId !== sourceThread?.account_id) {
      void loadFolders(targetAccountId, false)
    }
    showToast(t('mail.toast.threadCopied'))
  } catch (error) {
    showToast(mailTransferError(error, t('mail.toast.copyFailed')), 'error')
  }
}

export async function archiveThread(threadId: string) {
  if (!threadId) return
  const sourceThread = findLocalThread(threadId)
  if (isLocalAccountId(sourceThread?.account_id || threadAccountId(threadId))) return
  // Already in the built-in Archive Meron folder: archived already.
  if (sourceThread?.meron_folder_id === MERON_ARCHIVE_ID) return
  const sourceFolder = sourceThread?.folder_id ?? ''
  const { rollback, complete } = removeThreadLocally(threadId)
  try {
    const response = await invoke<
      { folder?: string; thread_id?: string } & MovedCopiesResult & MutationResult & MeronOnlyArchive
    >('mail.archive', {
      thread_id: threadId,
    })
    assertMoveAffected(response, 'Archive')
    complete()
    applyMutationFolderUnreads(response)
    const archivedThreadId = response.thread_id ?? threadIdInFolder(threadId, sourceThread?.account_id, response.folder)
    const copyIds = movedCopyIds(response)
    await refreshThreadLocation(sourceThread?.account_id, false)
    // Filed in the built-in Archive folder too: its row follows.
    scheduleMeronFoldersRefresh()
    if (response.meron_only) {
      // Filed into the built-in Archive only; the mail stayed on the server.
      showUndoToast(t('mail.toast.archivedCount', { count: 1 }), () => {
        void undoMeronOnlyArchive(threadId, sourceThread?.account_id, response)
      })
    } else if (threadStillListed(threadId)) {
      showToast(t('mail.toast.archiveFailedInSameFolder'), 'error')
    } else if (sourceFolder && copyIds) {
      // Offer to move it back where it came from. Falls back to a plain toast
      // when the origin folder or the archived copies are unknown (nothing
      // reliable to restore to, or from).
      const restore = undoRestore(sourceThread, response)
      showUndoToast(t('mail.toast.archivedCount', { count: 1 }), () => {
        releaseRemovedThread(threadId)
        void moveThreadToFolder(archivedThreadId, sourceFolder, { undo: false, messageIds: copyIds, restore })
      })
    } else {
      showToast(t('mail.toast.archivedCount', { count: 1 }))
    }
  } catch (error) {
    rollback()
    showToast(archiveFailureText(error), 'error')
  }
}

export async function deleteThread(threadId: string, options: { permanent?: boolean } = {}) {
  if (!threadId) return
  const sourceThread = findLocalThread(threadId)
  const sourceFolder = sourceThread?.folder_id ?? ''
  // Drafts are expunged in place by the engine (never moved to Trash), so the
  // delete is permanent there too — but worded as a discard.
  const isDraft =
    sourceThread?.folder_role === 'drafts' ||
    (!sourceThread?.folder_role && isDraftFolder(sourceFolder, sourceThread?.account_id))
  const isTrash =
    sourceThread?.folder_role === 'trash' ||
    (!sourceThread?.folder_role && isTrashFolderId(sourceThread?.account_id ?? '', sourceFolder))
  const permanent = options.permanent ?? (isDraft || isTrash || isLocalAccountId(sourceThread?.account_id))
  if (isDraft || permanent) {
    if (
      !(await confirmAction({
        title: isDraft ? 'Discard draft?' : 'Delete thread forever?',
        message: isDraft
          ? "This draft will be permanently deleted. This can't be undone."
          : "This thread will be permanently deleted. This can't be undone.",
        confirmLabel: isDraft ? 'Discard' : 'Delete forever',
        tone: 'danger',
      }))
    ) {
      return
    }
  }

  const { rollback, complete } = removeThreadLocally(threadId)

  try {
    const response = await invoke<
      { deleted?: number; permanent?: boolean; trash?: string; thread_id?: string } & MovedCopiesResult & MutationResult
    >('mail.delete', {
      thread_id: threadId,
      ...(sourceFolder ? { folder: sourceFolder } : {}),
    })
    assertDeleteAffected(response)
    complete()
    applyMutationFolderUnreads(response)
    const trashedThreadId = response.thread_id ?? threadIdInFolder(threadId, sourceThread?.account_id, response.trash)
    const copyIds = movedCopyIds(response)
    const canUndoTrashMove = !!(response.thread_id || response.trash) && !!copyIds
    await refreshThreadLocation(undefined, false)
    if (threadStillListed(threadId)) {
      showToast(t('mail.toast.deleteFailedInSameFolder'), 'error')
    } else if (!isDraft && !permanent && !response.permanent && sourceFolder && canUndoTrashMove) {
      const restore = undoRestore(sourceThread, response)
      showUndoToast(t('mail.toast.threadMovedToTrash'), () => {
        releaseRemovedThread(threadId)
        void moveThreadToFolder(trashedThreadId, sourceFolder, { undo: false, messageIds: copyIds, restore })
      })
    } else {
      showToast(
        isDraft
          ? t('mail.toast.draftDiscarded')
          : permanent
            ? t('mail.toast.deletedCount', { count: 1 })
            : t('mail.toast.threadMovedToTrash'),
      )
    }
  } catch (error) {
    rollback()
    showToast(error instanceof Error ? error.message : t('mail.toast.deleteFailed'), 'error')
  }
}

export async function discardSavedDraftCopy(
  draft: {
    threadId: string
    messageId: string
    folderId: string
    accountId?: string
    draftMessageId?: string
    /** The conversation a full-editor reply was written into. Unlike
     *  `threadId` — the discarded draft's own thread, which the discard can
     *  empty — this is a thread the reply is only one message of: its card
     *  carries the Draft badge, but it never goes away with the draft. */
    replyThreadId?: string
  },
  options: { throwOnError?: boolean; failureMessage?: string } = {},
): Promise<boolean> {
  if (!draft.accountId && !draft.threadId) return true

  const previousThreads = mail$.threads.get()
  const previousMessages = mail$.messages.get()
  const viewKey = () =>
    threadListViewKey(ui$.selectedAccount.peek(), ui$.selectedFolder.peek(), ui$.query.peek(), listFilterKey())
  const previousView = viewKey()
  const discardedDraftMessageId = normalizeMessageId(draft.draftMessageId)
  const withoutDiscardedDraft = (messages: Message[]) =>
    messages.filter((message) => {
      if (draft.messageId && message.id === draft.messageId) return false
      if (
        discardedDraftMessageId &&
        normalizeMessageId(message.message_id) === discardedDraftMessageId &&
        isDraftFolder(message.folder_id, message.account_id)
      ) {
        return false
      }
      return true
    })
  const nextMessages = withoutDiscardedDraft(previousMessages)
  const selectedThread = ui$.selectedThread.get()
  const removeThread = !!draft.threadId && !nextMessages.some((message) => message.thread_id === draft.threadId)
  const kanbanKeys = kanbanKeysWithThread(draft.threadId || (draft.replyThreadId ?? ''))

  mail$.messages.set(nextMessages)
  if (removeThread) {
    mail$.threads.set(previousThreads.filter((thread) => thread.thread_id !== draft.threadId))
    removeKanbanThread(draft.threadId)
    if (selectedThread === draft.threadId) {
      ui$.selectedThread.set('')
      requestThreadReselect()
    }
  } else {
    reconcileThreadDraftFromLoadedMessages(draft.threadId, nextMessages)
  }
  // The absence of a draft row proves nothing unless every message of the
  // conversation is here to say so: a thread that isn't open has none of them
  // loaded, and one opened past its first page is still missing its older ones,
  // where a second draft the badge stands for may well sit. Deriving the badge
  // from a partial view would clear it — and clear it again after the card
  // re-read below has correctly put it back. Short of the whole conversation,
  // that re-read is what settles it.
  const reconcileReplyThreadDraft = (loaded: Message[]) => {
    const replyThreadId = draft.replyThreadId ?? ''
    if (!replyThreadId || mail$.messagesCursor.get()) return
    if (!loaded.some((message) => message.thread_id === replyThreadId)) return
    reconcileThreadDraftFromLoadedMessages(replyThreadId, loaded)
  }
  reconcileReplyThreadDraft(nextMessages)

  try {
    if (draft.accountId && draft.draftMessageId) {
      await invoke('mail.discardDraft', {
        account_id: draft.accountId,
        draft_id: draft.draftMessageId,
        thread_id: draft.threadId,
      })
    } else {
      const response = await invoke('mail.delete', {
        thread_id: draft.threadId,
        message_ids: [draft.messageId],
        folder: draft.folderId,
      })
      if (!isDraftFolder(draft.folderId, draft.accountId)) assertDeleteAffected(response)
      applyMutationFolderUnreads(response as MutationResult)
    }
  } catch (error) {
    // Put back what this discard took and nothing else. The tab closes before
    // the discard runs, so the conversation may have changed since: a restored
    // compose tab has none loaded at all, and restoring the lists as they were
    // would blank the one that has loaded behind it.
    const openThread = ui$.selectedThread.get()
    const current = mail$.messages.get()
    const present = new Set([...nextMessages, ...current].map((message) => message.id))
    const missing = previousMessages.filter((message) => message.thread_id === openThread && !present.has(message.id))
    if (missing.length > 0) {
      const restored = [...current]
      for (const message of missing) {
        const later = restored.findIndex((item) => item.date > message.date)
        restored.splice(later === -1 ? restored.length : later, 0, message)
      }
      mail$.messages.set(restored)
    }
    if (viewKey() === previousView) {
      const cards = new Set([draft.threadId, draft.replyThreadId].filter(Boolean))
      const badges = new Map(
        previousThreads
          .filter((thread) => cards.has(thread.thread_id))
          .map((thread) => [thread.thread_id, thread.has_draft]),
      )
      const threads = removeThread
        ? restoreRemovedThread(mail$.threads.get(), previousThreads, draft.threadId)
        : mail$.threads.get()
      mail$.threads.set(
        threads.map((thread) =>
          badges.has(thread.thread_id) ? { ...thread, has_draft: badges.get(thread.thread_id) } : thread,
        ),
      )
    }
    if (options.throwOnError) throw error
    showToast(
      contextualErrorMessage(error, options.failureMessage ?? t('composer.status.couldNotDiscardDraft')),
      'error',
    )
    return false
  }
  // The draft is gone from here on, so a card reload that fails is not a failed
  // discard: the next refresh corrects the cards.
  try {
    await reloadThreadCards(kanbanKeys)
  } catch (error) {
    console.error('Reloading thread cards after a discard failed:', error)
  }
  // A mail.synced thread refresh can land while the server discard is on the
  // wire and reinsert the stale draft row after the optimistic removal above.
  // The discard has now succeeded, so remove that copy again before deriving
  // the conversation and thread-card draft state.
  const reconciledMessages = withoutDiscardedDraft(mail$.messages.get())
  if (reconciledMessages.length !== mail$.messages.get().length) mail$.messages.set(reconciledMessages)
  if (!removeThread) reconcileThreadDraftFromLoadedMessages(draft.threadId, reconciledMessages)
  reconcileReplyThreadDraft(reconciledMessages)
  const selectedAcc = ui$.selectedAccount.get()
  if (selectedAcc) void loadFolders(selectedAcc, false)
  if (draft.accountId && draft.accountId !== selectedAcc) void loadFolders(draft.accountId, false)
  return true
}

// Delete a single message (e.g. one draft) out of a thread, leaving the rest of
// the conversation intact. The message keeps its own folder_id so the delete
// targets the mailbox the message actually lives in, not the thread's folder.
// Drafts are discarded permanently (engine expunges them); other messages go to
// Trash. The confirm/toast wording reflects which.
/** Delete one message, after the user confirms. Resolves true once it is gone. */
export async function deleteMessage(message: Message): Promise<boolean> {
  if (!message?.id) return false

  // Local-only optimistic send (still sending, or failed): it has no
  // server-side copy under this id, so just drop it from the pane and forget
  // any pending retry payload — no backend round-trip, no confirm.
  if (isLocalSendId(message.id)) {
    discardPendingSend(message.id)
    // Also cancel any rescue keyed on it: without this the bubble the user
    // just deleted would be restored on the next thread load, and a rescue
    // still on the wire would leave its draft copy behind.
    cancelUnsentRescue(message.id)
    mail$.messages.set(mail$.messages.get().filter((item) => item.id !== message.id))
    return true
  }

  const isDraft = isDraftFolder(message.folder_id, message.account_id)
  const forever = !isDraft && deletesForever(message.account_id, message.folder_id)
  const confirmMessage = isDraft
    ? "Discard this draft? This can't be undone."
    : forever
      ? "This message will be permanently deleted. This can't be undone."
      : 'Move this message to Trash?'
  if (
    !(await confirmAction({
      title: isDraft ? 'Discard draft?' : forever ? 'Delete message forever?' : 'Move message to Trash?',
      message: confirmMessage,
      confirmLabel: isDraft ? 'Discard' : forever ? 'Delete forever' : 'Move to Trash',
      tone: 'danger',
    }))
  ) {
    return false
  }

  const threadId = message.thread_id
  const previousMessages = mail$.messages.get()
  const nextMessages = previousMessages.filter((item) => item.id !== message.id)
  const kanbanKeys = kanbanKeysWithThread(threadId)

  mail$.messages.set(nextMessages)

  try {
    const response = await invoke('mail.delete', {
      thread_id: threadId,
      message_ids: [message.id],
      folder: message.folder_id,
    })
    if (!isDraft) assertDeleteAffected(response)
    applyMutationFolderUnreads(response as MutationResult)
    showToast(
      isDraft
        ? t('mail.toast.draftDiscarded')
        : forever
          ? t('mail.toast.deletedCount', { count: 1 })
          : t('mail.toast.messageMovedToTrash'),
    )
    // No messages left from this thread: drop the now-empty conversation.
    if (!nextMessages.some((item) => item.thread_id === threadId)) {
      if (ui$.selectedThread.get() === threadId) {
        ui$.selectedThread.set('')
        requestThreadReselect()
      }
      removeKanbanThread(threadId)
    }
    await reloadThreadCards(kanbanKeys)
    const selectedAcc = ui$.selectedAccount.get()
    if (selectedAcc) void loadFolders(selectedAcc, false)
    if (message.account_id && message.account_id !== selectedAcc) {
      void loadFolders(message.account_id, false)
    }
    return true
  } catch (error) {
    mail$.messages.set(previousMessages)
    showToast(error instanceof Error ? error.message : t('mail.toast.deleteFailed'), 'error')
    return false
  }
}

// Moves (or archives) one email, not its whole conversation: right-click an
// email in a chat. The message id names its own folder and uid
// (account#folder#uid), so a sender chat's Sent reply moves out of Sent and an
// inbox email out of the inbox. Undo moves that one copy back.
export async function moveMessage(
  message: Message,
  targetFolderId: string,
  options: { targetAccountId?: string; archive?: boolean } = {},
): Promise<boolean> {
  if (!message?.id || isLocalSendId(message.id)) return false
  const accountId = message.account_id
  const sourceFolder = message.folder_id
  const targetAccountId = options.targetAccountId || accountId
  const crossAccount = targetAccountId !== accountId
  if (!options.archive && (!targetFolderId || (!crossAccount && sourceFolder === targetFolderId))) return false

  const threadId = message.thread_id
  const previousMessages = mail$.messages.get()
  const nextMessages = previousMessages.filter((item) => item.id !== message.id)
  const kanbanKeys = kanbanKeysWithThread(threadId)
  mail$.messages.set(nextMessages)

  try {
    const response = options.archive
      ? await invoke<{ folder?: string } & MovedCopiesResult & MutationResult>('mail.archive', {
          thread_id: message.id,
        })
      : await invoke<MovedCopiesResult & MutationResult>('mail.move', {
          thread_id: message.id,
          target_folder_id: targetFolderId,
          ...(crossAccount ? { target_account_id: targetAccountId } : {}),
        })
    assertMoveAffected(response, options.archive ? 'Archive' : 'Move')
    applyMutationFolderUnreads(response)
    const landedFolder = options.archive ? ((response as { folder?: string }).folder ?? '') : targetFolderId
    const copyUid = movedCopyIds(response)?.[0]
    const done = options.archive ? t('mail.toast.messageArchived') : t('mail.toast.messageMoved')
    if (!crossAccount && landedFolder && sourceFolder && copyUid) {
      showUndoToast(done, () => {
        void invoke('mail.move', {
          thread_id: `${accountId}#${landedFolder}#${copyUid}`,
          target_folder_id: sourceFolder,
          undo: true,
        })
          .then(async () => {
            await reloadThreadCards(kanbanKeysWithThread(threadId))
            if (ui$.selectedThread.get() === threadId) await loadThread(threadId)
            void loadFolders(accountId, false)
          })
          .catch((error) => showToast(mailTransferError(error, t('mail.toast.moveFailed')), 'error'))
      })
    } else {
      showToast(done)
    }
    if (!options.archive && !crossAccount) {
      rememberThreadTransfer({ action: 'move', sourceAccountId: accountId, targetAccountId, targetFolderId })
    }
    // No messages left in this chat: drop the now-empty conversation.
    if (!nextMessages.some((item) => item.thread_id === threadId)) {
      if (ui$.selectedThread.get() === threadId) {
        ui$.selectedThread.set('')
        requestThreadReselect()
      }
      removeKanbanThread(threadId)
    }
    await reloadThreadCards(kanbanKeys)
    void loadFolders(accountId, false)
    if (crossAccount) void loadFolders(targetAccountId, false)
    return true
  } catch (error) {
    mail$.messages.set(previousMessages)
    showToast(
      options.archive ? archiveFailureText(error) : mailTransferError(error, t('mail.toast.moveFailed')),
      'error',
    )
    return false
  }
}
