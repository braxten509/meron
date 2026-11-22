// Tracks an email export from the desktop mailbox. Progress and cancellation
// remain available while the native host writes the selected messages.
// Callers include BulkActionBar.tsx and ConversationHeader.tsx.

import { observable } from '@legendapp/state'
import { invoke } from '../lib/bridge'
import { t } from '../lib/i18n'
import { showToast } from './ui'

// Progress of the running bulk .eml export, shown by EmlExportProgress. The
// bridge runs one export at a time; `total` stays 0 until the save dialog has
// been answered and the first progress event arrives.
export const emlExport$ = observable({ active: false, done: 0, total: 0 })

type ExportResult = { saved: boolean; cancelled?: boolean; exported?: number; failed?: number }

// Raw .eml bytes aren't cached, so the bridge refetches every message over IMAP
// and packs them into one .zip (a lone message is saved as a bare .eml). It
// reports progress as it goes and resolves once the file is written; a message
// it can't fetch is skipped and counted in `failed`.
async function runEmlExport(payload: Record<string, unknown>) {
  if (emlExport$.active.peek()) return
  emlExport$.set({ active: true, done: 0, total: 0 })
  const off = (window as any).runtime?.EventsOn?.(
    'mail.exportProgress',
    (detail: { done?: number; failed?: number; total?: number }) => {
      emlExport$.done.set((detail?.done ?? 0) + (detail?.failed ?? 0))
      emlExport$.total.set(detail?.total ?? 0)
    },
  )
  try {
    const response = await invoke<ExportResult>('mail.exportEml', payload)
    if (response?.cancelled) {
      showToast(t('mail.toast.exportCancelled'))
    } else if (response?.saved) {
      const count = response.exported ?? 0
      const failed = response.failed ?? 0
      showToast(
        failed > 0 ? t('mail.toast.exportedWithSkipped', { count, failed }) : t('mail.toast.exportedCount', { count }),
        'success',
        failed > 0 ? 6000 : undefined,
      )
    }
  } catch {
    showToast(t('mail.toast.exportFailed'), 'error')
  } finally {
    if (typeof off === 'function') off()
    emlExport$.set({ active: false, done: 0, total: 0 })
  }
}

/** Export every message of the given threads. `name` seeds the default filename. */
export function exportThreadsAsEml(threads: { threadId: string; folderId?: string }[], name: string) {
  if (threads.length === 0) return Promise.resolve()
  return runEmlExport({
    threads: threads.map((thread) => ({
      thread_id: thread.threadId,
      ...(thread.folderId ? { folder: thread.folderId } : {}),
    })),
    name,
  })
}

/** Export every message the server holds in a folder, synced locally or not. */
export function exportFolderAsEml(accountId: string, folderId: string, name: string) {
  return runEmlExport({ account_id: accountId, folder_id: folderId, name })
}

export function cancelEmlExport() {
  void invoke('mail.exportEmlCancel', {}).catch(() => {})
}
