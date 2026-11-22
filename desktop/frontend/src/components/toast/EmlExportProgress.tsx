// Floating status for a running bulk .eml export: how
// far along it is, and a way out. Sits above the toast
// slot so a toast raised meanwhile doesn't cover it.

import { Loader2 } from 'lucide-react'
import { useValue } from '@legendapp/state/react'
import { useTranslation } from '../../lib/i18n'
import { cancelEmlExport, emlExport$ } from '../../states/emlExport'

export function EmlExportProgress() {
  const { t } = useTranslation()
  const { active, done, total } = useValue(emlExport$)

  // Nothing to show until the save dialog is answered and the export starts.
  if (!active || total === 0) return null

  return (
    <div
      role="status"
      aria-live="polite"
      className="fixed bottom-16 left-1/2 z-[140] flex -translate-x-1/2 animate-slide-up items-center gap-2 rounded-full bg-black/80 py-2 pl-4 pr-2 text-xs font-semibold text-white shadow-xl"
    >
      <Loader2 size={14} className="shrink-0 animate-spin" />
      <span className="tabular-nums">{t('mail.toast.exportProgress', { done, total })}</span>
      <button
        onClick={cancelEmlExport}
        className="ml-1 shrink-0 cursor-pointer rounded-full bg-white/15 px-2.5 py-1 font-bold text-white transition-colors hover:bg-white/25"
      >
        {t('buttons.cancel')}
      </button>
    </div>
  )
}
