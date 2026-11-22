// Shared popover shell for the side navigation
// rail's right-click menus: a full-screen
// click-catcher plus a card positioned at the
// cursor.

import type { ReactNode } from 'react'
import { FloatingContextMenu } from '../menu/FloatingContextMenu'

// Side navigation rail menu rows are just the shared MenuItem; re-exported under the
// rail's local name so existing call sites keep importing from here.
export { MenuItem as RailMenuItem } from '../menu/MenuItem'

export function RailContextMenu({
  x,
  y,
  onClose,
  children,
}: {
  x: number
  y: number
  onClose: () => void
  children: ReactNode
}) {
  return (
    <FloatingContextMenu
      x={x}
      y={y}
      offset={4}
      onClose={onClose}
      overlay
      className="fixed z-50 min-w-[176px]"
      onContextMenu={(event) => {
        event.preventDefault()
        event.stopPropagation()
      }}
    >
      {children}
    </FloatingContextMenu>
  )
}
