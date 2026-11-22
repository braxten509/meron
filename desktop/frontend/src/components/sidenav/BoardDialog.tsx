// Modal for creating a kanban board.
// Renaming, image, and background live in the
// board's settings panel (Settings → Kanban
// Boards).

import { X } from 'lucide-react'
import { useTranslation } from '../../lib/i18n'
import { Dialog } from '../dialog/Dialog'
import { createKanbanBoard } from '../../states/kanban'
import { Button } from '../button/Button'
import { IconButton } from '../button/IconButton'
import { TextInput } from '../field/Field'

export type BoardDialogState = { mode: 'create'; name: string }

export function BoardDialog({
  state,
  onChange,
  onClose,
}: {
  state: BoardDialogState
  onChange: (state: BoardDialogState) => void
  onClose: () => void
}) {
  const { t } = useTranslation()
  return (
    <Dialog
      onClose={onClose}
      aria-label={t('kanban.actions.addBoard')}
      backdropClassName="z-[100] dark:bg-black/40 backdrop-blur-none"
      className="max-w-sm rounded-xl p-4"
    >
      <form
        onSubmit={(event) => {
          event.preventDefault()
          const name = state.name.trim()
          if (!name) return
          createKanbanBoard(name)
          onClose()
        }}
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 className="text-sm font-bold text-primary">{t('kanban.actions.addBoard')}</h2>
          <IconButton icon={X} iconSize={15} label={t('buttons.close')} size="sm" radius="lg" onClick={onClose} />
        </div>
        <label className="mb-1.5 block text-[0.6875rem] font-bold uppercase tracking-wide text-secondary">
          {t('kanban.board.name')}
        </label>
        <TextInput
          autoFocus
          value={state.name}
          onChange={(event) => onChange({ ...state, name: event.target.value })}
          fieldSize="md"
          surface="app"
          className="mb-4 w-full font-semibold"
        />
        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onClose}>
            {t('buttons.cancel')}
          </Button>
          <Button variant="primary" size="sm" type="submit" disabled={!state.name.trim()}>
            {t('kanban.actions.addBoardShort')}
          </Button>
        </div>
      </form>
    </Dialog>
  )
}
