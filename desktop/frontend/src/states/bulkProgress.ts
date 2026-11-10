import { observable } from '@legendapp/state'
import { getFilteredThreads } from './mail'
import { clearBulkSelection, ui$ } from './ui'

// One running bulk Delete / Archive / Move: what the strip under the folder
// name shows, which rows wait greyed out, and the Stop request. Conversations
// are handled a group at a time (Archive: one at a time), so a large
// selection takes a while; this is what tells the reader it is working and
// lets them stop it.

/** How many conversations a bulk Delete or Move sends to the mail server in
 *  one go. The whole group costs about what one conversation does alone; the
 *  strip's count and Stop act between groups. */
export const BULK_GROUP_SIZE = 25

export type BulkJobKind = 'delete' | 'archive' | 'move'

export type BulkJob = {
  kind: BulkJobKind
  /** The list the action was started in (see {@link bulkViewKey}). */
  viewKey: string
  total: number
  done: number
  /** Thread ids still waiting their turn: their rows are greyed out. */
  pending: Record<string, true>
  stopRequested: boolean
}

export type BulkJobResult = {
  completed: number
  total: number
  /** The reader pressed Stop before every conversation was handled. */
  stopped: boolean
  /** A step threw; the conversations after it were left alone. */
  error?: unknown
}

export const bulkProgress$ = observable<{ job: BulkJob | null }>({ job: null })

/** The list a bulk action belongs to: an account's folder or a Meron folder.
 *  The strip shows only there, so it follows the reader out and back in. */
export function bulkViewKey(accountId: string, folderId: string): string {
  return `${accountId}\n${folderId}`
}

export function bulkJobRunning(): boolean {
  return bulkProgress$.job.peek() !== null
}

/** Ask the running job to stop. The conversation in progress still finishes. */
export function requestBulkStop() {
  if (bulkProgress$.job.peek()) bulkProgress$.job.stopRequested.set(true)
}

// The open conversation is about to be removed with the rest: step off it once,
// to the nearest row that stays, instead of opening (and marking read) each
// doomed neighbour in turn as the rows leave.
function moveSelectionOff(targetIds: Set<string>) {
  const selected = ui$.selectedThread.peek()
  if (!selected || !targetIds.has(selected)) return
  const visible = getFilteredThreads()
  const index = visible.findIndex((thread) => thread.thread_id === selected)
  const stays = (thread: { thread_id: string }) => !targetIds.has(thread.thread_id)
  const neighbour =
    index === -1
      ? visible.find(stays)
      : (visible.slice(index + 1).find(stays) ?? visible.slice(0, index).reverse().find(stays))
  ui$.selectedThread.set(neighbour?.thread_id ?? '')
}

/**
 * Run `step` for each group of `groupSize` targets in order, publishing
 * progress for the strip. `step` calls `done` for each conversation of its
 * group that went through, and throws when one did not. Stops before the next
 * group when the reader presses Stop or a step throws; whatever was not done
 * is left exactly as it was. Never throws: the caller words the outcome from
 * the result.
 */
export async function runBulkJob<T extends { threadId: string }>(
  kind: BulkJobKind,
  targets: T[],
  step: (group: T[], done: (item: T) => void) => Promise<void>,
  groupSize = 1,
): Promise<BulkJobResult> {
  const total = targets.length
  moveSelectionOff(new Set(targets.map((item) => item.threadId)))
  bulkProgress$.job.set({
    kind,
    viewKey: bulkViewKey(ui$.selectedAccount.peek(), ui$.selectedFolder.peek()),
    total,
    done: 0,
    pending: Object.fromEntries(targets.map((item) => [item.threadId, true as const])),
    stopRequested: false,
  })
  // Back to the folder name: the strip under it carries the count from here.
  clearBulkSelection()
  let completed = 0
  const done = (item: T) => {
    completed++
    const job = bulkProgress$.job.peek()
    if (!job) return
    const pending = { ...job.pending }
    delete pending[item.threadId]
    bulkProgress$.job.set({ ...job, done: completed, pending })
  }
  try {
    for (let start = 0; start < total; start += groupSize) {
      if (bulkProgress$.job.stopRequested.peek()) return { completed, total, stopped: true }
      await step(targets.slice(start, start + groupSize), done)
    }
    return { completed, total, stopped: false }
  } catch (error) {
    return { completed, total, stopped: false, error }
  } finally {
    bulkProgress$.job.set(null)
  }
}
