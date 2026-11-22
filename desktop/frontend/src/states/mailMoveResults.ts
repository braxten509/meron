// Interprets the identities and affected-message counts returned by mail mutations.
// Move, copy, archive and undo share these checks for server-side results.
// Folder-scoped thread IDs connect returned copies to the desktop conversation list.

import type { Message } from '../types'
import { invoke } from '../lib/bridge'
import { t } from '../lib/i18n'
import { mailTransferError } from '../lib/mailTransferErrors'
import { showToast } from './ui'
import { scheduleMeronFoldersRefresh } from './meronFolders'
import { releaseRemovedThread } from './mail'
import { refreshThreadLocation } from './mailMoveSelection'

export function assertDeleteAffected(response: unknown) {
  if (!response || typeof response !== 'object') return
  const deleted = (response as { deleted?: unknown }).deleted
  if (typeof deleted === 'number' && deleted <= 0) {
    throw new Error('Delete failed: no matching messages found')
  }
}

export function assertMoveAffected(response: unknown, label = 'Move') {
  if (!response || typeof response !== 'object') return
  const moved = (response as { moved?: unknown }).moved
  if (typeof moved === 'number' && moved <= 0) {
    throw new Error(`${label} failed: no matching messages found`)
  }
}

/** An archive's failure toast. An account with no archive mailbox answers a
 *  single email's Archive with "no archive folder": the plain message then,
 *  not the server's text. */
export function archiveFailureText(error: unknown): string {
  const detail = error instanceof Error ? error.message : String(error)
  if (/archive folder not found/i.test(detail)) return t('mail.toast.archiveFailed')
  return mailTransferError(error, t('mail.toast.archiveFailed'))
}

/** Archived Meron-only (the account has no archive mailbox): the answer of
 *  the filing into the built-in Archive. */
export type MeronOnlyArchive = {
  meron_only?: boolean
  changes?: Array<{ previous_folder_id?: string | null }>
}

/** Undo of a Meron-only archive: back into the Meron folder it came from, or
 *  out of Meron folders (it never left its mailbox). */
export async function undoMeronOnlyArchive(
  threadId: string,
  accountId: string | undefined,
  response: MeronOnlyArchive,
) {
  releaseRemovedThread(threadId)
  const previous = response.changes?.[0]?.previous_folder_id
  try {
    if (previous) await invoke('mail.moveToMeronFolder', { folder_id: previous, thread_ids: [threadId], undo: true })
    else await invoke('mail.moveOutOfMeronFolder', { thread_ids: [threadId], undo: true })
  } catch (error) {
    showToast(error instanceof Error ? error.message : t('notification.undoFailed'), 'error')
  }
  scheduleMeronFoldersRefresh()
  await refreshThreadLocation(accountId, false)
}

export function assertCopyAffected(response: unknown) {
  if (!response || typeof response !== 'object') return
  const copied = (response as { copied?: unknown }).copied
  if (typeof copied === 'number' && copied <= 0) {
    throw new Error('Copy failed: no matching messages found')
  }
}

// UIDs of the copies a move created in its target folder, as the message_ids
// an undo sends back so it moves exactly those — not every message of the
// conversation the target already held, and not by a uid: thread key, whose
// UID the move changed. Undefined when the core couldn't tell the copies apart
// from other mail there: then no Undo is offered, since any other way back
// could move mail the action never touched.
export type MovedCopiesResult = { target_uids?: number[]; meron_folder_id?: string; meron_source?: string }

export function movedCopyIds(response: MovedCopiesResult): string[] | undefined {
  return response.target_uids?.length ? response.target_uids.map(String) : undefined
}

// What an Undo hands the core besides the copies to move back: show them in
// the Inbox again (mail that was shown there from its own folder), or put them
// back in the Meron folder the action took the card out of.
export type UndoRestore = { showInInbox?: boolean; meronFolderId?: string; meronSource?: string }

export function undoRestore(thread: Message | undefined, response: MovedCopiesResult): UndoRestore {
  return {
    showInInbox: !!thread?.shown_in_inbox,
    ...(response.meron_folder_id
      ? { meronFolderId: response.meron_folder_id, meronSource: response.meron_source }
      : {}),
  }
}

export function threadAccountId(threadId: string): string {
  const separator = threadId.indexOf('#')
  return separator > 0 ? threadId.slice(0, separator) : ''
}

export function threadIdInFolder(
  threadId: string,
  accountId: string | undefined,
  folderId: string | undefined,
): string {
  const lastHash = threadId.lastIndexOf('#')
  if (!accountId || !folderId || lastHash <= 0) return threadId
  return `${accountId}#${folderId}#${threadId.slice(lastHash + 1)}`
}
