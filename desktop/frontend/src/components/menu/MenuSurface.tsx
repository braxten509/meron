// Provides the common visual surface for desktop menus. Placement, dismissal and
// menu-specific actions remain with the hosting component.
// Callers include FloatingContextMenu.tsx and ComposerToolbar.tsx.

import type { ComponentProps } from 'react'
import { clsx } from '../../lib/classNames'

/** Shared, animation-free surface for positioned menus and their flyouts. */
export function MenuSurface({ className, ref, ...props }: ComponentProps<'div'>) {
  return <div {...props} ref={ref} className={clsx('menu-surface', className)} />
}
