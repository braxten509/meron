// Renders the minimize, maximize and close actions exposed by the desktop host.
// Their order follows the native decoration preferences.
// Imported by TitleBar.tsx.

import { Copy, Minus, Square, X, type LucideIcon } from 'lucide-react'
import { useValue } from '@legendapp/state/react'
import { useTranslation } from '../../lib/i18n'
import { windowChrome$, windowCommand, type WindowButton } from '../../lib/windowChrome'

// Lucide, like the title bar's other buttons (TitleBar.tsx), at their size, so
// the row reads as one set of icons.
const ICONS: Record<WindowButton | 'restore', LucideIcon> = {
  minimize: Minus,
  maximize: Square,
  restore: Copy,
  close: X,
}

const ICON_SIZE = 17

// The Windows caption glyphs (ChromeMinimize, ChromeMaximize, ChromeRestore,
// ChromeClose) in Segoe Fluent Icons (Windows 11) and Segoe MDL2 Assets
// (Windows 10), which share the code points.
const CAPTION_GLYPHS: Record<WindowButton | 'restore', string> = {
  minimize: '\uE921',
  maximize: '\uE922',
  restore: '\uE923',
  close: '\uE8BB',
}

const COMMAND = { minimize: 'minimise', maximize: 'toggleMaximise', close: 'close' } as const

/**
 * Window controls for one side of the integrated title bar, in the desktop's
 * button layout. Renders nothing with the system title bar. On Windows they
 * are the system's caption buttons: 46px wide, the title bar's full height,
 * flush with the corner. On Linux they are laid out like GNOME's (libadwaita
 * 1.8 default.css, windowcontrols: a 24px hover circle in a button padded
 * 5px, 3px apart) but drawn like the title bar's own flat buttons: lucide
 * icons in their colors (index.css). The icon is nested in the circle's SVG:
 * as separate boxes the circle landed on fractional pixels while the icon was
 * snapped to whole ones, leaving it visibly off-centre.
 */
export function WindowControls({ side }: { side: 'start' | 'end' }) {
  const { t } = useTranslation()
  const integrated = useValue(windowChrome$.integrated)
  const buttons = useValue(windowChrome$.layout[side])
  const maximised = useValue(windowChrome$.maximised)
  const windows = useValue(windowChrome$.platform) === 'windows'

  if (!integrated || buttons.length === 0) return null

  const label = (button: WindowButton) =>
    button === 'close'
      ? t('buttons.close')
      : button === 'minimize'
        ? t('window.minimize')
        : maximised
          ? t('window.restore')
          : t('window.maximize')

  if (windows) {
    return (
      <div className="flex shrink-0 self-stretch">
        {buttons.map((button) => (
          <button
            key={button}
            type="button"
            className={`caption-button${button === 'close' ? ' caption-button-close' : ''}`}
            aria-label={label(button)}
            title={label(button)}
            onClick={() => windowCommand(COMMAND[button])}
          >
            <span aria-hidden>{CAPTION_GLYPHS[button === 'maximize' && maximised ? 'restore' : button]}</span>
          </button>
        ))}
      </div>
    )
  }

  return (
    <div className="window-controls flex shrink-0 items-center gap-[3px]">
      {buttons.map((button) => (
        <button
          key={button}
          type="button"
          className="window-control"
          aria-label={label(button)}
          title={label(button)}
          onClick={() => windowCommand(COMMAND[button])}
        >
          <svg viewBox="0 0 24 24" width="24" height="24" aria-hidden>
            <circle className="window-control-circle" cx="12" cy="12" r="12" />
            <WindowIcon button={button === 'maximize' && maximised ? 'restore' : button} />
          </svg>
        </button>
      ))}
    </div>
  )
}

function WindowIcon({ button }: { button: WindowButton | 'restore' }) {
  const Icon = ICONS[button]
  const offset = (24 - ICON_SIZE) / 2
  return <Icon x={offset} y={offset} size={ICON_SIZE} />
}
