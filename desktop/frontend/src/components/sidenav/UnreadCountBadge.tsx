/**
 * Displays an inbox unread count on a side-navigation avatar or button.
 * The badge sits at the top right of its parent and caps the label at 9999+.
 * A zero count leaves the avatar visible on its own.
 */
export function UnreadCountBadge({ count }: { count: number }) {
  if (count <= 0) return null
  return (
    <span className="pointer-events-none absolute -top-1 -right-1 flex h-4.5 min-w-4.5 items-center justify-center rounded-full bg-accent-badge px-1.5 text-[0.625rem] font-semibold leading-none text-white ring-2 ring-sidenav">
      {count > 9999 ? '9999+' : count}
    </span>
  )
}
