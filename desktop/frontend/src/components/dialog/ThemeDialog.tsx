// Presents the theme choices available for the desktop window. The selected
// appearance determines which stored choice applies.
// Imported by ThemeSettingsSection.tsx.

import { useState } from 'react'
import { Palette, Plus, X } from 'lucide-react'
import { useValue } from '@legendapp/state/react'
import { useTranslation } from '../../lib/i18n'
import { Dialog } from './Dialog'
import { BUILTIN_THEMES, type Appearance, type CustomTheme, type ThemeDef } from '../../lib/themes'
import { useChosenThemeIds } from '../../lib/useThemeChoice'
import { confirmAction } from '../../states/ui'
import { deleteCustomTheme, selectTheme, setThemeFollowSystem, settings$ } from '../../states/settings'
import { IconButton } from '../button/IconButton'
import { ToggleRow } from './AccountSettingsRows'
import { ThemeEditorDialog, copyThemeSource } from './ThemeEditorDialog'
import { ThemeSwatch } from './ThemeSwatch'

// Theme picker dialog (Settings -> General -> Theme -> Change), following the
// WallpaperDialog layout: light and dark sections of large swatches, with the
// custom-theme editor reachable from a dashed tile in each section. While the
// theme follows the system, each section holds its own pick.

type EditorState = { appearance: Appearance; theme: CustomTheme | null }

function ThemeSection({
  label,
  themes,
  customThemes,
  chosenIds,
  onEdit,
  onDelete,
  newTileAppearance,
}: {
  label: string
  themes: ThemeDef[]
  customThemes: CustomTheme[]
  chosenIds: string[]
  onEdit: (state: EditorState) => void
  onDelete: (theme: CustomTheme) => void
  newTileAppearance: Appearance
}) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col gap-3">
      <span className="text-[0.78125rem] font-semibold text-secondary">{label}</span>
      <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-3">
        {themes.map((item) => {
          const custom = customThemes.find((candidate) => candidate.id === item.id)
          return (
            <ThemeSwatch
              key={item.id}
              theme={item}
              large
              selected={chosenIds.includes(item.id)}
              onSelect={() => selectTheme(item)}
              onShare={custom ? () => copyThemeSource(custom.source, t('theme.shareCopied')) : undefined}
              onEdit={custom ? () => onEdit({ appearance: custom.appearance, theme: custom }) : undefined}
              onDelete={custom ? () => onDelete(custom) : undefined}
            />
          )
        })}
        <button
          type="button"
          onClick={() => onEdit({ appearance: newTileAppearance, theme: null })}
          className="flex min-h-[112px] flex-col items-center justify-center gap-1.5 rounded-xl border border-dashed border-border text-secondary hover:text-accent hover:border-accent/50 cursor-pointer transition-colors"
        >
          <Plus size={16} />
          <span className="text-[0.65625rem] font-bold">{t('theme.custom')}</span>
        </button>
      </div>
    </div>
  )
}

export function ThemeDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation()
  const [editor, setEditor] = useState<EditorState | null>(null)
  const followSystem = useValue(settings$.themeFollowSystem)
  const customThemes = useValue(settings$.customThemes)
  const chosenIds = useChosenThemeIds()

  const themes = [...BUILTIN_THEMES, ...customThemes]

  const onDelete = async (themeToDelete: CustomTheme) => {
    const confirmed = await confirmAction({
      title: t('theme.delete'),
      message: t('theme.deleteMessage', { name: themeToDelete.name }),
      confirmLabel: t('buttons.delete'),
      tone: 'danger',
    })
    if (confirmed) deleteCustomTheme(themeToDelete.id)
  }

  const sectionProps = {
    customThemes,
    chosenIds,
    onEdit: (state: EditorState) => setEditor(state),
    onDelete: (theme: CustomTheme) => void onDelete(theme),
  }

  return (
    <>
      <Dialog
        onClose={onClose}
        closeOnBackdrop={false}
        aria-label={t('common.theme')}
        backdropClassName="z-[70] bg-black/45 dark:bg-black/65"
        className="max-w-3xl h-[620px] max-h-[90vh] flex flex-col overflow-hidden"
      >
        <div className="flex items-center justify-between gap-3 border-b border-border/70 px-6 py-4 shrink-0">
          <div className="flex items-center gap-2">
            <Palette className="text-accent" size={16} />
            <h3 className="text-[0.875rem] font-bold leading-tight">{t('common.theme')}</h3>
          </div>
          <IconButton icon={X} iconSize={15} label={t('buttons.close')} size="sm" onClick={onClose} />
        </div>

        <div className="flex-1 overflow-y-auto p-6 flex flex-col gap-6">
          <div className="shrink-0 rounded-2xl bg-raised/80 border border-border/60 overflow-hidden">
            <ToggleRow
              title={t('theme.matchSystem')}
              hint={t('theme.matchSystemHint')}
              checked={followSystem}
              onChange={() => setThemeFollowSystem(!followSystem)}
            />
          </div>
          <ThemeSection
            label={t('theme.light')}
            themes={themes.filter((item) => item.appearance === 'light')}
            newTileAppearance="light"
            {...sectionProps}
          />
          <ThemeSection
            label={t('theme.dark')}
            themes={themes.filter((item) => item.appearance === 'dark')}
            newTileAppearance="dark"
            {...sectionProps}
          />
        </div>
      </Dialog>

      {editor && (
        <ThemeEditorDialog appearance={editor.appearance} initial={editor.theme} onClose={() => setEditor(null)} />
      )}
    </>
  )
}
