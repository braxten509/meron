// The dialog-variant provider picker: a vertical rail (mirrors the
// Settings dialog nav). Selecting one swaps the form shown in the
// panel to its right, so "pick a type -> fill in details" reads as one
// flow.

import type { SetupMode } from '../../states/ui'
import { useTranslation } from '../../lib/i18n'
import { PROVIDERS } from './providerIcons'

export function AccountProviderRail({ mode, setMode }: { mode: SetupMode; setMode: (mode: SetupMode) => void }) {
  const { t } = useTranslation()
  return (
    <nav className="w-44 shrink-0 border-r border-border/70 p-3 flex flex-col gap-1 bg-raised overflow-y-auto">
      {PROVIDERS.map((provider) => {
        const active = provider.isActive(mode)
        return (
          <button
            key={provider.id}
            onClick={() => setMode(provider.mode)}
            className={`flex items-center gap-2.5 rounded-xl px-2.5 py-2 text-left transition-colors cursor-pointer ${
              active ? 'bg-accent/10' : 'hover:bg-hover'
            }`}
          >
            <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-border/50 bg-raised">
              {provider.icon(16)}
            </span>
            <span className="flex min-w-0 flex-col leading-tight">
              <span className={`truncate text-[0.75rem] font-bold ${active ? 'text-accent' : 'text-primary'}`}>
                {provider.label}
              </span>
              <span className="truncate text-[0.59375rem] font-medium text-secondary">
                {t(provider.descriptionKey, { defaultValue: provider.defaultDescription })}
              </span>
            </span>
          </button>
        )
      })}
    </nav>
  )
}
