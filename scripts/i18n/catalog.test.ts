// Checks the catalog contracts that the platform generators rely on.
// Normalization cases cover flattened messages and plural variants.
// Validation and escaping cases protect translators’ arguments in generated files.
import { describe, expect, test } from 'bun:test'

import {
  androidEscape,
  androidLocaleDirectory,
  androidName,
  flattenObject,
  foldPlurals,
  iosLocale,
  placeholders,
  kotlinString,
  validateCatalogs,
} from './catalog'

describe('catalog normalization helpers', () => {
  test('flattens nested keys and converts i18next interpolation', () => {
    expect(
      flattenObject({
        buttons: { save: 'Save {{name}}' },
        common: { moreCount: '+{{count}} more' },
      }),
    ).toEqual({
      'buttons.save': 'Save {name}',
      'common.moreCount': '+{count} more',
    })
  })

  test('folds English plural suffixes into ICU messages', () => {
    expect(
      foldPlurals({
        'chat.fileItems_one': '{{count}} file',
        'chat.fileItems_other': '{{count}} files',
      }),
    ).toEqual({
      'chat.fileItems': '{count, plural, one {{{count}} file} other {{{count}} files}}',
    })
  })

  test('preserves richer Arabic plural categories', () => {
    expect(
      foldPlurals({
        'mail.threadCount_zero': 'لا رسائل',
        'mail.threadCount_one': 'رسالة واحدة',
        'mail.threadCount_two': 'رسالتان',
        'mail.threadCount_few': '{{count}} رسائل',
        'mail.threadCount_many': '{{count}} رسالة',
        'mail.threadCount_other': '{{count}} رسالة',
      }),
    ).toEqual({
      'mail.threadCount':
        '{count, plural, zero {لا رسائل} one {رسالة واحدة} two {رسالتان} few {{{count}} رسائل} many {{{count}} رسالة} other {{{count}} رسالة}}',
    })
  })

  test('does not fold incomplete plural groups', () => {
    expect(
      foldPlurals({
        'mail.threadCount_one': 'One thread',
      }),
    ).toEqual({
      'mail.threadCount_one': 'One thread',
    })
  })
})

describe('catalog validation helpers', () => {
  test('validates placeholder parity against English', () => {
    expect(() =>
      validateCatalogs({
        en: { 'welcome.message': 'Hello {name}' },
        ja: { 'welcome.message': 'こんにちは {name}' },
      }),
    ).not.toThrow()

    expect(() =>
      validateCatalogs({
        en: { 'welcome.message': 'Hello {name}' },
        ja: { 'welcome.message': 'こんにちは {user}' },
      }),
    ).toThrow(/placeholder mismatch/)
  })

  test('rejects a translation that loses the item count', () => {
    expect(() => validateCatalogs({ en: { more: '+{count} more' }, ru: { more: 'Ещё' } })).toThrow(
      /ru.more placeholder mismatch/,
    )
  })

  test('extracts the plural selector and de-duplicates repeated arguments', () => {
    expect(
      placeholders('{count, plural, one {{count} file from {sender}} other {{count} files from {sender}}}'),
    ).toEqual(['count', 'sender'])
  })
})

describe('platform mapping helpers', () => {
  test('maps special locale identifiers explicitly', () => {
    expect(androidLocaleDirectory('pt_BR')).toBe('values-pt-rBR')
    expect(iosLocale('pt_BR')).toBe('pt-BR')
  })

  test('preserves literal dollar expressions and controls in generated Kotlin', () => {
    expect(kotlinString('Cost: $amount or ${total}')).toBe('"Cost: \\$amount or \\${total}"')
    expect(kotlinString('"line"\n\\\t\f')).toBe('"\\"line\\"\\n\\\\\\t\\u000c"')
  })

  test('escapes Android XML string values', () => {
    expect(androidName('buttons.cancel')).toBe('buttons_cancel')
    expect(androidEscape("@Don't use <x> or 50%")).toBe("\\@Don\\'t use &lt;x&gt; or 50%%")
    expect(androidEscape('Version %1$s · Package %2$s')).toBe('Version %1$s · Package %2$s')
  })
})

describe('Russian catalog', () => {
  test('covers every English key and preserves named and positional placeholders', async () => {
    const en = (await import('../../locales/en.json')).default
    const ru = (await import('../../locales/ru.json')).default
    expect(() => validateCatalogs({ en, ru })).not.toThrow()
    for (const key of Object.keys(en) as (keyof typeof en)[]) {
      expect(ru[key].match(/%\d+\$[sd]/g) ?? []).toEqual(en[key].match(/%\d+\$[sd]/g) ?? [])
      if (en[key].includes(', plural,')) {
        for (const category of ['one', 'few', 'many', 'other']) {
          expect(ru[key]).toContain(`${category} {`)
        }
      }
    }
    expect(androidLocaleDirectory('ru')).toBe('values-ru')
    expect(iosLocale('ru')).toBe('ru')
  })
})
