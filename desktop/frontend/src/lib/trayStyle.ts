// Applies the user's tray style through the native bridge. Writes made by quick
// successive changes must settle in the order the reader chose.
// Imported by settings.ts.

import { invoke } from './bridge'
import { createLatestBooleanUpdater } from './latestBooleanUpdater'

export function createTrayStyleUpdater(send = (monochrome: boolean) => invoke('tray.setStyle', { monochrome })) {
  return createLatestBooleanUpdater(send)
}

export const setTrayStyle = createTrayStyleUpdater()
