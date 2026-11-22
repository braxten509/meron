// Stores recent move and copy destinations for the desktop picker. Entries keep
// account and action identity while repeated destinations move to the front.
// Callers include mailFolders.ts and mailMoves.ts.

import { observable } from '@legendapp/state'

export type RecentThreadTransfer = {
  action: 'move' | 'copy'
  sourceAccountId: string
  targetAccountId: string
  targetFolderId: string
}

const STORAGE_KEY = 'meron.recentThreadTransfers'
const LIMIT = 100

export function parseRecentThreadTransfers(raw: string | null): RecentThreadTransfer[] {
  try {
    const entries: unknown = JSON.parse(raw ?? '[]')
    if (!Array.isArray(entries)) return []
    return entries
      .filter(
        (entry): entry is RecentThreadTransfer =>
          entry != null &&
          (entry.action === 'move' || entry.action === 'copy') &&
          ['sourceAccountId', 'targetAccountId', 'targetFolderId'].every(
            (key) => typeof entry[key] === 'string' && entry[key].length > 0,
          ),
      )
      .slice(0, LIMIT)
  } catch {
    return []
  }
}

function loadRecentThreadTransfers() {
  try {
    return parseRecentThreadTransfers(localStorage.getItem(STORAGE_KEY))
  } catch {
    return []
  }
}

export const recentThreadTransfers$ = observable<RecentThreadTransfer[]>(loadRecentThreadTransfers())

function saveRecentThreadTransfers(entries: RecentThreadTransfer[]) {
  recentThreadTransfers$.set(entries)
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(entries))
  } catch {
    // Keep session history when storage is unavailable or full.
  }
}

/** Record confirmed transfers, never implicit archive/delete moves or undo. */
export function rememberThreadTransfer(entry: RecentThreadTransfer) {
  const entries = [
    entry,
    ...recentThreadTransfers$
      .get()
      .filter(
        (previous) =>
          previous.action !== entry.action ||
          previous.sourceAccountId !== entry.sourceAccountId ||
          previous.targetAccountId !== entry.targetAccountId ||
          previous.targetFolderId !== entry.targetFolderId,
      ),
  ].slice(0, LIMIT)
  saveRecentThreadTransfers(entries)
}

/** Follow a folder rename, so recent targets in it are still offered. */
export function renameRecentTransferTargets(accountId: string, renames: Map<string, string>) {
  const entries = recentThreadTransfers$.get()
  if (!entries.some((entry) => entry.targetAccountId === accountId && renames.has(entry.targetFolderId))) return
  saveRecentThreadTransfers(
    entries.map((entry) => {
      const targetFolderId = entry.targetAccountId === accountId ? renames.get(entry.targetFolderId) : undefined
      return targetFolderId === undefined ? entry : { ...entry, targetFolderId }
    }),
  )
}
