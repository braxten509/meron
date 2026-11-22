// The folded quote under a reply: the message it answers goes out
// beneath the body without filling the editor. The dots unfold it into
// the body to be edited; the right-click menu can also drop it from the
// reply.

import { useState } from 'react'
import { Trash2, UnfoldVertical } from 'lucide-react'
import { useTranslation } from '../../lib/i18n'
import { FloatingContextMenu } from '../menu/FloatingContextMenu'
import { MenuItem } from '../menu/MenuItem'

export function ComposerQuote({ onExpand, onRemove }: { onExpand: () => void; onRemove: () => void }) {
  const { t } = useTranslation()
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null)
  const showLabel = t('chat.showQuotedText')
  const removeLabel = t('composer.removeQuotedText')

  return (
    <div
      className="mt-2 flex items-center"
      onContextMenu={(event) => {
        event.preventDefault()
        setMenu({ x: event.clientX, y: event.clientY })
      }}
    >
      <button
        type="button"
        onClick={onExpand}
        title={showLabel}
        aria-label={showLabel}
        className="flex h-3.5 w-7 cursor-pointer items-center justify-center rounded-full border border-border/70 bg-black/5 font-sans text-[10px] leading-none font-bold tracking-[1px] text-secondary hover:bg-black/10 hover:text-primary dark:bg-white/10 dark:hover:bg-white/15"
      >
        •••
      </button>
      {menu && (
        <FloatingContextMenu
          x={menu.x}
          y={menu.y}
          offset={4}
          onClose={() => setMenu(null)}
          overlay
          className="fixed z-50 min-w-[176px]"
          onContextMenu={(event) => {
            event.preventDefault()
            event.stopPropagation()
          }}
        >
          <MenuItem
            icon={<UnfoldVertical size={13} className="text-secondary" />}
            label={showLabel}
            onClick={() => {
              setMenu(null)
              onExpand()
            }}
          />
          <MenuItem
            icon={<Trash2 size={13} />}
            label={removeLabel}
            danger
            onClick={() => {
              setMenu(null)
              onRemove()
            }}
          />
        </FloatingContextMenu>
      )}
    </div>
  )
}
