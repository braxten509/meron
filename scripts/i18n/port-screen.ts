// Ports an Android Compose screen to the shared UI package.
// Canonical translation keys replace simple Android string resource references.
// Run with the source and destination paths, then review calls carrying arguments or drawables.
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { androidName } from './catalog'

const root = join(import.meta.dir, '..', '..')
const [sourcePath, destinationPath] = process.argv.slice(2)

const en = JSON.parse(readFileSync(join(root, 'locales/en.json'), 'utf8')) as Record<string, string>
const keysByResource: Record<string, string> = {}
for (const catalogKey of Object.keys(en)) keysByResource[androidName(catalogKey)] = catalogKey

const stripImport =
  /^import (android\.|java\.|androidx\.core\.|androidx\.activity\.|org\.json\.|androidx\.compose\.ui\.platform\.LocalContext|androidx\.compose\.ui\.graphics\.asImageBitmap|androidx\.compose\.ui\.viewinterop\.AndroidView|androidx\.compose\.ui\.res\.stringResource|androidx\.compose\.ui\.res\.painterResource)/

let code = readFileSync(sourcePath, 'utf8')
code = code.replace(/^package jp\.nonbili\.meron$/m, 'package jp.nonbili.meron.ui')
code = code.replace(/MeronCoreNative\.isLoaded\(\)/g, 'coreLoaded')
code = code.replace(/MobileMailCommandClient\(JniMeronCore\(\)\)/g, 'MobileMailCommandClient(core)')
code = code.replace(/Dispatchers\.IO/g, 'ioDispatcher')
code = code
  .split('\n')
  .filter((line) => !stripImport.test(line))
  .join('\n')
code = code.replace(/stringResource\(R\.string\.([A-Za-z0-9_]+)\)/g, (match, name: string) => {
  const key = keysByResource[name]
  if (!key) {
    console.error('NO CANONICAL KEY for R.string.' + name)
    return match
  }
  return `tr("${key}")`
})

writeFileSync(destinationPath, code)
console.log(`ported ${sourcePath} -> ${destinationPath}`)
