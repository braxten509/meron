import { observable } from '@legendapp/state'
import type { Account, Message } from '../types'
import { invoke } from '../lib/bridge'
import { t } from '../lib/i18n'
import { isRssAccount } from '../lib/threadActions'
import { conversationRootOf, isSenderThreadId } from '../lib/senderThread'
import { MERON_ACCOUNT, isMeronView } from '../lib/unifiedFolders'
import { accounts$, isLocalAccountId } from './accounts'
import { confirmAction, promptText, showToast, showUndoToast, ui$ } from './ui'
import { loadThreads, mail$, releaseRemovedThread } from './mail'
import { removeThreadLocally } from './mailMoves'
import { updateCachedFolderUnread } from './mailFolders'
import { openMailAccount } from './kanban'

// Meron folders: folders that exist only inside Meron, on this device. A
// conversation moved into one leaves wherever it showed in Meron and shows only
// there; the mail server is never told. The core (meron-core/src/meron_folders.rs)
// owns the data; this module keeps the list for the side navigation and wraps
// the commands with the list, badge and Undo bookkeeping every surface shares.

export type MeronFolder = {
  id: string
  name: string
  unread: number
  count: number
  created_at: number
  /** New mail AI sorting files here notifies (this device only). */
  notify?: boolean
  /** `'archive'` for the built-in Archive folder (always there, can't be
   *  renamed or deleted); absent or null for the user's own. */
  builtin?: 'archive' | null
}

/** The built-in Archive Meron folder: whole conversations archived from now
 *  on are filed here as well as archived on the server. */
export const MERON_ARCHIVE_ID = 'mf_archive'

export function isMeronArchive(folder: { id: string; builtin?: string | null } | string | null | undefined): boolean {
  if (!folder) return false
  if (typeof folder === 'string') return folder === MERON_ARCHIVE_ID
  return folder.builtin === 'archive' || folder.id === MERON_ARCHIVE_ID
}

type FolderCount = { account_id: string; folder_id: string; unread: number }
type FoldersAnswer = { folders?: MeronFolder[]; folder_counts?: FolderCount[] }

export const meronFolders$ = observable({
  list: [] as MeronFolder[],
  // False until the first list answer, so the side navigation can keep its
  // rows' space steady instead of flashing "No Meron folders yet".
  loaded: false,
})

// Bumped by every list read and every mutation result. A list read only
// applies its answer while it is still the newest: a slow read that started
// before a create, rename, move or delete would otherwise put the old list
// back over the mutation's fresh one.
let loadVersion = 0

// Folders being marked read, with how many marks are still on the wire. The
// server takes several seconds per account, and every sync event meanwhile
// re-reads a count it has only partly cleared: the badge would tick down slowly
// instead of going to 0 at once. Until the mark answers, those reads show 0.
const markingRead = new Map<string, number>()

function applyAnswer(answer: FoldersAnswer | undefined, fromRead = false) {
  if (!fromRead) loadVersion += 1
  if (Array.isArray(answer?.folders)) {
    meronFolders$.list.set(
      answer.folders.map((folder) => (markingRead.has(folder.id) ? { ...folder, unread: 0 } : folder)),
    )
    meronFolders$.loaded.set(true)
  }
  for (const count of answer?.folder_counts ?? []) {
    updateCachedFolderUnread(count.account_id, count.folder_id, count.unread, 'mutation')
  }
}

export function meronFolderName(id: string | null | undefined): string {
  if (!id) return ''
  if (isMeronArchive(id)) return t('folders.roles.archive')
  return meronFolders$.list.peek().find((folder) => folder.id === id)?.name ?? ''
}

/** Total unread across every Meron folder, for the tray. That mail is hidden
 *  from the Inboxes' own counts, so without it the tray would go dark. The
 *  built-in Archive's unread counts only on its own row. */
export function meronUnreadTotal(): number {
  return meronFolders$.list
    .get()
    .reduce((sum, folder) => (isMeronArchive(folder) ? sum : sum + (folder.unread || 0)), 0)
}

/** Keep a folder's badge in step with the count a page load just read. */
export function setMeronFolderUnread(id: string, unread: number) {
  if (!Number.isFinite(unread)) return
  if (markingRead.has(id)) unread = 0
  const list = meronFolders$.list.peek()
  const index = list.findIndex((folder) => folder.id === id)
  if (index === -1 || list[index].unread === unread) return
  meronFolders$.list[index].unread.set(unread)
}

export async function loadMeronFolders() {
  const version = (loadVersion += 1)
  try {
    const answer = await invoke<FoldersAnswer>('mail.meronFolders', {})
    if (version !== loadVersion) return
    applyAnswer(answer, true)
    // The open Meron folder is gone (deleted, or a restored session that
    // named one that no longer exists): land on the unified inbox.
    const open = ui$.selectedFolder.peek()
    const listed = Array.isArray(answer.folders) ? answer.folders : null
    if (listed && isMeronView(ui$.selectedAccount.peek()) && !listed.some((folder) => folder.id === open)) {
      openMailAccount('unified', 'inbox')
    }
  } catch (error) {
    console.error('Failed to load Meron folders:', error)
  }
}

// A burst of sync events becomes one list read.
let refreshTimer: ReturnType<typeof setTimeout> | undefined
export function scheduleMeronFoldersRefresh() {
  clearTimeout(refreshTimer)
  refreshTimer = setTimeout(() => void loadMeronFolders(), 250)
}

export function openMeronFolder(id: string) {
  openMailAccount(MERON_ACCOUNT, id)
}

/** Whether a row can go in a Meron folder: a mail conversation of a server
 *  account, not a feed, a local folder's mail or a per-person row. */
export function canUseMeronFolders(
  row: { thread_id: string; account_id: string; group?: string } | undefined,
  accounts: Account[] = accounts$.peek(),
): boolean {
  if (!row?.thread_id || !row.account_id) return false
  if (row.group === 'sender' || isSenderThreadId(row.thread_id)) return false
  if (row.thread_id.includes('#rss#') || isLocalAccountId(row.account_id)) return false
  const account = accounts.find((item) => item.id === row.account_id)
  return !isRssAccount(account, row.account_id)
}

function nameProblem(name: string, exceptId = ''): string {
  if (!name) return t('meronFolders.nameRequired')
  const lower = name.toLocaleLowerCase()
  const taken = meronFolders$.list
    .peek()
    .some((folder) => folder.id !== exceptId && folder.name.toLocaleLowerCase() === lower)
  return taken ? t('meronFolders.nameTaken', { name }) : ''
}

/** Ask for a name and create a Meron folder. Resolves with the folder, or null
 *  when cancelled or refused. */
export async function promptCreateMeronFolder(): Promise<MeronFolder | null> {
  const raw = await promptText({
    title: t('meronFolders.new'),
    message: t('meronFolders.thisDeviceOnly'),
    placeholder: t('meronFolders.nameLabel'),
    confirmLabel: t('meronFolders.create'),
  })
  if (raw === null) return null
  const name = raw.trim()
  const problem = nameProblem(name)
  if (problem) {
    showToast(problem, 'error')
    return null
  }
  try {
    const answer = await invoke<FoldersAnswer & { folder?: MeronFolder | null }>('mail.createMeronFolder', { name })
    applyAnswer(answer)
    return answer.folder ?? null
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error), 'error')
    return null
  }
}

export async function promptRenameMeronFolder(folder: MeronFolder): Promise<boolean> {
  const raw = await promptText({
    title: t('meronFolders.renameTitle'),
    value: folder.name,
    placeholder: t('meronFolders.nameLabel'),
    confirmLabel: t('buttons.save'),
    cancelLabel: t('buttons.cancel'),
  })
  if (raw === null) return false
  const name = raw.trim()
  if (name === folder.name) return false
  const problem = nameProblem(name, folder.id)
  if (problem) {
    showToast(problem, 'error')
    return false
  }
  try {
    applyAnswer(await invoke<FoldersAnswer>('mail.renameMeronFolder', { id: folder.id, name }))
    return true
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error), 'error')
    return false
  }
}

/** Turn new-mail notifications for a Meron folder on or off: new mail AI
 *  sorting files there notifies, as an opted-in server folder's does. Same
 *  toasts as a server folder's. */
export async function setMeronFolderNotify(folder: MeronFolder, enabled: boolean): Promise<void> {
  try {
    applyAnswer(await invoke<FoldersAnswer>('mail.setMeronFolderNotify', { id: folder.id, enabled }))
    showToast(t(enabled ? 'folders.notify.enabled' : 'folders.notify.disabled', { folder: folder.name }))
  } catch (error) {
    showToast(error instanceof Error ? error.message : t('folders.notify.failed', { folder: folder.name }), 'error')
  }
}

/** Ask first, then delete a Meron folder; its conversations go back to where
 *  their mail is. Leaves the folder's view when it was open. */
export async function confirmDeleteMeronFolder(folder: MeronFolder): Promise<boolean> {
  const confirmed = await confirmAction({
    title: t('meronFolders.delete'),
    message: t('meronFolders.deleteConfirm', { name: folder.name }),
    confirmLabel: t('meronFolders.delete'),
    cancelLabel: t('buttons.cancel'),
    tone: 'danger',
  })
  if (!confirmed) return false
  try {
    applyAnswer(await invoke<FoldersAnswer>('mail.deleteMeronFolder', { id: folder.id }))
  } catch (error) {
    showToast(error instanceof Error ? error.message : String(error), 'error')
    return false
  }
  if (isMeronView(ui$.selectedAccount.peek()) && ui$.selectedFolder.peek() === folder.id) {
    openMailAccount('unified', 'inbox')
  } else {
    // Its conversations show in the list again.
    void loadThreads(false)
  }
  return true
}

type MoveOutAnswer = FoldersAnswer & {
  /** Conversations taken out of the built-in Archive, back in the Inbox. */
  unarchived?: Array<{ thread_id: string; account_id: string }>
  failures?: Array<{ message: string }>
  /** Conversations that left their Meron folders. */
  released?: number
}

type MoveInAnswer = FoldersAnswer & {
  moved?: number
  skipped?: Array<{ thread_id: string; reason: 'invalid' | 'not_allowed' | 'not_found' }>
  changes?: Array<{ thread_id: string; account_id: string; source_folder: string; previous_folder_id?: string | null }>
}

function rowOf(threadId: string): Message | undefined {
  return mail$.threads.peek().find((thread) => thread.thread_id === threadId)
}

/**
 * Move whole conversations into a Meron folder (or from one to another). The
 * rows leave the list at once; a failure puts them back. Offers Undo, which
 * returns each conversation to where it was: out of Meron folders, or into the
 * Meron folder it came from.
 */
export async function moveToMeronFolder(threadIds: string[], folderId: string, options: { undo?: boolean } = {}) {
  const ids = [...new Set(threadIds.filter(Boolean))].filter((id) => rowOf(id)?.meron_folder_id !== folderId)
  if (!ids.length || !folderId) return false
  const removals = ids.map((id) => ({ id, removal: removeThreadLocally(id) }))
  let answer: MoveInAnswer
  try {
    answer = await invoke<MoveInAnswer>('mail.moveToMeronFolder', { folder_id: folderId, thread_ids: ids })
  } catch (error) {
    for (const { removal } of removals) removal.rollback()
    showToast(error instanceof Error ? error.message : String(error), 'error')
    return false
  }
  applyAnswer(answer)
  const skipped = new Set((answer.skipped ?? []).map((item) => item.thread_id))
  for (const { id, removal } of removals) {
    if (skipped.has(id)) removal.rollback()
    else removal.complete()
  }
  void loadThreads(false)
  const moved = answer.moved ?? 0
  if (!moved) {
    showToast(t('meronFolders.notAllowed'), 'error')
    return false
  }
  const message = t('meronFolders.moved', { count: moved, name: meronFolderName(folderId) })
  const changes = answer.changes ?? []
  if (options.undo === false || !changes.length) {
    showToast(message)
    return true
  }
  showUndoToast(message, () => void undoMeronMove(changes), 'app', t('buttons.undo'))
  return true
}

async function undoMeronMove(changes: NonNullable<MoveInAnswer['changes']>) {
  // One step back per conversation. Subject branches of one conversation are
  // one entry; should a later branch's change say "no previous folder" (it
  // read the folder the first branch had just moved it into), the real
  // previous folder wins, so Undo never takes it out instead of back.
  const byConversation = new Map<string, { threadId: string; previous: string | null }>()
  for (const change of changes) {
    const key = conversationRootOf(change.thread_id)
    const known = byConversation.get(key)
    if (!known) byConversation.set(key, { threadId: change.thread_id, previous: change.previous_folder_id ?? null })
    else if (!known.previous && change.previous_folder_id) known.previous = change.previous_folder_id
  }
  const out: string[] = []
  const back = new Map<string, string[]>()
  for (const { threadId, previous } of byConversation.values()) {
    if (previous) back.set(previous, [...(back.get(previous) ?? []), threadId])
    else out.push(threadId)
  }
  for (const change of changes) releaseRemovedThread(change.thread_id)
  try {
    if (out.length) applyAnswer(await invoke<FoldersAnswer>('mail.moveOutOfMeronFolder', { thread_ids: out }))
    for (const [folderId, ids] of back) {
      applyAnswer(await invoke<FoldersAnswer>('mail.moveToMeronFolder', { folder_id: folderId, thread_ids: ids }))
    }
  } catch (error) {
    showToast(error instanceof Error ? error.message : t('notification.undoFailed'), 'error')
  }
  await loadThreads(false)
}

/** Take conversations out of their Meron folders: they show where their mail
 *  is again. Inside a Meron folder the rows leave its list at once. Offers
 *  Undo, which puts each conversation back into the Meron folder it left. */
export async function moveOutOfMeronFolder(threadIds: string[], options: { undo?: boolean } = {}) {
  const ids = [...new Set(threadIds.filter(Boolean))]
  if (!ids.length) return false
  const inMeronView = isMeronView(ui$.selectedAccount.peek())
  // Where each conversation was, read before the rows leave the list: the
  // row's own folder, or the open Meron folder for rows listed in it.
  const openFolder = inMeronView ? ui$.selectedFolder.peek() : ''
  const former = new Map<string, string[]>()
  const seen = new Set<string>()
  for (const id of ids) {
    const folderId = rowOf(id)?.meron_folder_id || openFolder
    const conversation = conversationRootOf(id)
    if (!folderId || seen.has(conversation)) continue
    seen.add(conversation)
    former.set(folderId, [...(former.get(folderId) ?? []), id])
  }
  // Out of the built-in Archive means back to the Inbox (un-archived on the
  // server too).
  const allArchive = ids.every((id) => isMeronArchive(rowOf(id)?.meron_folder_id || openFolder))
  const removals = inMeronView ? ids.map((id) => removeThreadLocally(id)) : []
  let answer: MoveOutAnswer
  try {
    answer = await invoke<MoveOutAnswer>('mail.moveOutOfMeronFolder', { thread_ids: ids })
  } catch (error) {
    for (const removal of removals) removal.rollback()
    showToast(error instanceof Error ? error.message : String(error), 'error')
    return false
  }
  applyAnswer(answer)
  const failure = answer.failures?.[0]?.message
  const released = answer.released ?? 0
  // Nothing left (moving out of Archive failed on the server): the rows come
  // back, and the error stays on screen.
  if (failure && released === 0) {
    for (const removal of removals) removal.rollback()
    void loadThreads(false)
    showToast(failure || t('mail.toast.moveFailed'), 'error')
    return false
  }
  for (const removal of removals) removal.complete()
  void loadThreads(false)
  // Some moved, some failed: the error stays visible instead of a success
  // toast with Undo over it.
  if (failure) {
    showToast(failure, 'error')
    return true
  }
  // Archive's conversations went back to the Inbox under new ids: an Undo
  // archives those again.
  const archived = former.get(MERON_ARCHIVE_ID)
  if (archived) {
    const back = (answer.unarchived ?? []).map((item) => item.thread_id).filter(Boolean)
    if (back.length) former.set(MERON_ARCHIVE_ID, back)
    else former.delete(MERON_ARCHIVE_ID)
  }
  // What really left; an answer without the count falls back to the rows.
  const count = released || seen.size || ids.length
  const message = allArchive ? t('meronFolders.movedToInbox', { count }) : t('meronFolders.movedOut', { count })
  if (options.undo === false || former.size === 0) {
    showToast(message)
    return true
  }
  showUndoToast(message, () => void undoMoveOut(former), 'app', t('buttons.undo'))
  return true
}

async function undoMoveOut(former: Map<string, string[]>) {
  for (const ids of former.values()) for (const id of ids) releaseRemovedThread(id)
  try {
    for (const [folderId, ids] of former) {
      applyAnswer(await invoke<FoldersAnswer>('mail.moveToMeronFolder', { folder_id: folderId, thread_ids: ids }))
    }
  } catch (error) {
    showToast(error instanceof Error ? error.message : t('notification.undoFailed'), 'error')
  }
  await loadThreads(false)
}

/** Mark every conversation in a Meron folder read, on the servers too. */
export async function markMeronFolderRead(folderId: string) {
  if (!folderId) return
  // The side navigation's menu can mark a folder that isn't the one open.
  const viewing = () => isMeronView(ui$.selectedAccount.peek()) && ui$.selectedFolder.peek() === folderId
  // Shown read at once, rows and badge; the reload below brings the truth either way.
  if (viewing()) {
    mail$.threads.set(
      mail$.threads
        .peek()
        .map((thread) =>
          thread.unread || thread.unread_count ? { ...thread, unread: false, unread_count: 0 } : thread,
        ),
    )
  }
  markingRead.set(folderId, (markingRead.get(folderId) ?? 0) + 1)
  setMeronFolderUnread(folderId, 0)
  const doneMarking = () => {
    const left = (markingRead.get(folderId) ?? 1) - 1
    if (left > 0) markingRead.set(folderId, left)
    else markingRead.delete(folderId)
  }
  try {
    const answer = await invoke<FoldersAnswer & { ok?: boolean; failures?: Array<{ message: string }> }>(
      'mail.markMeronFolderRead',
      { folder_id: folderId },
    )
    doneMarking()
    applyAnswer(answer)
    if (answer.failures?.length) showToast(answer.failures[0].message, 'error')
  } catch (error) {
    doneMarking()
    showToast(error instanceof Error ? error.message : String(error), 'error')
    // The badge was shown 0; read the real count back.
    scheduleMeronFoldersRefresh()
  }
  if (viewing()) await loadThreads(false)
}
