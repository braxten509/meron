// Corner trash button for the settings-header pictures (account avatar,
// board image). Rendered as a sibling of the picture's change button
// inside a `relative group` wrapper: it shows on hover, and on keyboard
// focus.

import { Trash2 } from 'lucide-react'

export function RemoveImageBadge({
  label,
  disabled,
  onRemove,
}: {
  label: string
  disabled?: boolean
  onRemove: () => void
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onRemove}
      className="absolute -top-1.5 -right-1.5 z-10 flex h-5 w-5 items-center justify-center rounded-full border border-border bg-chats text-secondary shadow-sm opacity-0 group-hover:opacity-100 focus-visible:opacity-100 hover:text-rose-500 disabled:cursor-default cursor-pointer transition-opacity"
    >
      <Trash2 size={11} />
    </button>
  )
}
