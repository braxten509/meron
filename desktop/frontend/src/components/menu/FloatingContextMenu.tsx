// Places a menu in a portal and keeps it within the viewport. Dismissal and event
// propagation must cooperate with the originating React surface.
// Callers include FolderSwitcher.tsx and AddMailToTasksMenu.tsx.

import { useLayoutEffect, useRef, type MouseEvent, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { useEscapeKey } from '../../lib/useEscapeKey'
import { MenuSurface } from './MenuSurface'

const noop = () => {}

export function FloatingContextMenu({
  x,
  y,
  onClose,
  children,
  className,
  offset = 0,
  placement = 'down',
  margin = 8,
  overlay = false,
  overlayClassName = 'fixed inset-0 z-50',
  dataAttribute,
  onClick,
  onContextMenu,
}: {
  x: number
  y: number
  onClose?: () => void
  children: ReactNode
  className: string
  offset?: number
  placement?: 'up' | 'down'
  margin?: number
  overlay?: boolean
  overlayClassName?: string
  dataAttribute?: string
  onClick?: (event: MouseEvent<HTMLDivElement>) => void
  onContextMenu?: (event: MouseEvent<HTMLDivElement>) => void
}) {
  const menuRef = useRef<HTMLDivElement | null>(null)
  const dataProps = dataAttribute ? { [dataAttribute]: '' } : undefined

  useEscapeKey(onClose ?? noop, Boolean(onClose))

  useLayoutEffect(() => {
    const element = menuRef.current
    if (!element) return

    const { width, height } = element.getBoundingClientRect()
    const desiredLeft = x + offset
    const desiredTop = placement === 'up' ? y - offset - height : y + offset
    const left = Math.max(margin, Math.min(desiredLeft, window.innerWidth - width - margin))
    const top = Math.max(margin, Math.min(desiredTop, window.innerHeight - height - margin))

    element.style.left = `${left}px`
    element.style.top = `${top}px`
  })

  const content = (
    <>
      {overlay && onClose && (
        <div
          className={overlayClassName}
          // Portals bubble events through the React tree, not the DOM one: a
          // menu rendered from inside a clickable row (a message header) would
          // otherwise hand that row the dismissing click as well.
          onClick={(event) => {
            event.stopPropagation()
            onClose()
          }}
          onContextMenu={(event) => {
            event.preventDefault()
            event.stopPropagation()
            onClose()
          }}
        />
      )}
      <MenuSurface
        ref={menuRef}
        className={className}
        style={{ left: x + offset, top: y + offset }}
        onClick={onClick}
        onContextMenu={onContextMenu}
        {...dataProps}
      >
        {children}
      </MenuSurface>
    </>
  )

  if (typeof document === 'undefined') return content
  return createPortal(content, document.body)
}
