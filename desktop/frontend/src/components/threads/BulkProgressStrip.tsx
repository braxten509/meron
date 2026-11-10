import { useValue } from '@legendapp/state/react'
import { useTranslation } from '../../lib/i18n'
import { bulkProgress$, bulkViewKey, requestBulkStop } from '../../states/bulkProgress'

const LABEL_KEYS = {
  delete: 'threads.bulkProgress.deleting',
  archive: 'threads.bulkProgress.archiving',
  move: 'threads.bulkProgress.moving',
} as const

/**
 * The strip under the folder name while a bulk Delete / Archive / Move runs:
 * "Deleting 5 of 13…", a bar, and Stop. Shown only in the list the action was
 * started in, so it is there again when the reader comes back to that folder.
 */
export function BulkProgressStrip({ accountId, folderId }: { accountId: string; folderId: string }) {
  const { t } = useTranslation()
  const job = useValue(bulkProgress$.job)
  if (!job || job.viewKey !== bulkViewKey(accountId, folderId)) return null
  // The count names the conversation being worked on; the bar shows what is finished.
  const current = Math.min(job.done + 1, job.total)
  const percent = job.total > 0 ? (job.done / job.total) * 100 : 0
  return (
    <div className="shrink-0 border-b border-border/60 bg-accent/[0.06] px-4 pb-2.5 pt-2" role="status">
      <div className="mb-1.5 flex items-center justify-between gap-3 text-[0.75rem] leading-5">
        <span className="min-w-0 truncate text-primary">
          {job.stopRequested
            ? t('threads.bulkProgress.stopping')
            : t(LABEL_KEYS[job.kind], { done: current, total: job.total })}
        </span>
        <button
          type="button"
          className="shrink-0 font-semibold text-accent hover:underline disabled:opacity-50 disabled:hover:no-underline"
          disabled={job.stopRequested}
          onClick={requestBulkStop}
        >
          {t('threads.bulkProgress.stop')}
        </button>
      </div>
      <div
        className="h-1 overflow-hidden rounded-full bg-border/70"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={job.total}
        aria-valuenow={job.done}
      >
        <div
          className="h-full rounded-full bg-accent transition-[width] duration-200"
          style={{ width: `${percent}%` }}
        />
      </div>
    </div>
  )
}
