// Determines which desktop platforms offer a monochrome tray icon. Settings use
// this capability before presenting a style choice.
// Imported by SettingsDialog.tsx.

import { isLinuxPlatform, isMacPlatform } from './platform'

/**
 * Whether the monochrome tray icon is offered: the macOS
 * menu bar and Linux panels draw one-colour icons that follow the bar (a
 * template image on macOS, a symbolic icon the shell recolours on Linux), Windows keeps the colour icon.
 * The backend supplies the saved style/default at boot.
 */
export function offersMonochromeTrayIcon(userAgent: string, platform = ''): boolean {
  return isMacPlatform(userAgent, platform) || isLinuxPlatform(userAgent, platform)
}
