// Small info icon with a hover/focus tooltip. The bubble is
// fixed-positioned from the icon's rect because settings panels scroll
// inside an overflow-hidden dialog, which would clip an
// absolutely-positioned bubble.

import { useState } from 'react'
import { Info } from 'lucide-react'

export function InfoTip({ hint }: { hint?: string }) {
  const [position, setPosition] = useState<{ x: number; y: number } | null>(null)

  if (!hint) return null

  const show = (element: HTMLElement) => {
    const rect = element.getBoundingClientRect()
    setPosition({ x: rect.left + rect.width / 2, y: rect.top })
  }

  return (
    <span
      tabIndex={0}
      className="inline-flex shrink-0 text-secondary/60 hover:text-secondary focus:text-secondary focus:outline-none cursor-help"
      onMouseEnter={(event) => show(event.currentTarget)}
      onMouseLeave={() => setPosition(null)}
      onFocus={(event) => show(event.currentTarget)}
      onBlur={() => setPosition(null)}
    >
      <Info size={12} />
      {position && (
        <span
          role="tooltip"
          className="fixed z-[60] w-max max-w-[280px] -translate-x-1/2 -translate-y-full whitespace-normal rounded-xl border border-border bg-chats px-3 py-2 text-[0.6875rem] font-normal leading-snug text-primary shadow-lg pointer-events-none"
          style={{ left: position.x, top: position.y - 6 }}
        >
          {hint}
        </span>
      )}
    </span>
  )
}
