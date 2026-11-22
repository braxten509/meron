// Resolves the active theme and the user's light and dark selections. Theme
// controls use these values when previewing or saving a choice.
// Callers include ThemeDialog.tsx and ThemeSettingsSection.tsx.

import { useMemo } from 'react'
import { useValue } from '@legendapp/state/react'
import { chosenThemeIds, resolveThemeDef, settings$, systemAppearance$ } from '../states/settings'
import type { ThemeDef } from './themes'

// The theme selection, subscribed to. The resolvers in states/settings read
// with peek(), so every input they depend on is subscribed here instead:
// editing the active custom theme or the OS switching appearance must repaint
// the same way picking a theme does.
function useThemeInputs(): unknown[] {
  return [
    useValue(settings$.themeId),
    useValue(settings$.themeFollowSystem),
    useValue(settings$.lightThemeId),
    useValue(settings$.darkThemeId),
    useValue(settings$.customThemes),
    useValue(systemAppearance$),
  ]
}

/** The theme being painted. */
export function useActiveTheme(): ThemeDef {
  const inputs = useThemeInputs()
  return useMemo(() => resolveThemeDef(), inputs)
}

/** Ids the picker marks as chosen (see chosenThemeIds). */
export function useChosenThemeIds(): string[] {
  const inputs = useThemeInputs()
  return useMemo(() => chosenThemeIds(), inputs)
}
