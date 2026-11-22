// Defines desktop theme tokens and their CSS variables. The theme editor, built-in
// palettes and persisted selections share those token names.
// Callers include useThemeChoice.ts and ThemeDialog.tsx.

import type { CSSProperties } from 'react'
import { flatten, isValidColor, parseColor, toHex } from './color'
import {
  type ThemeDef,
  type Appearance,
  type CustomThemeInput,
  type ThemeTokens,
  accentLabelVars,
  THEME_TOKEN_KEYS,
  TOKEN_CSS_VAR,
  type CustomTheme,
  deriveThemeTokens,
} from './themeTokens'
import { BUILTIN_THEMES, DEFAULT_LIGHT_ID, DEFAULT_DARK_ID } from './builtinPalettes'
export { BUILTIN_THEMES, DEFAULT_LIGHT_ID, DEFAULT_DARK_ID } from './builtinPalettes'
export {
  type Appearance,
  type ThemeTokens,
  TOKEN_CSS_VAR,
  THEME_TOKEN_KEYS,
  type ThemeDef,
  type CustomThemeInput,
  type CustomTheme,
  SURFACE_LADDER,
  applySurfaceRules,
  deriveThemeTokens,
  contrastRatio,
  accentLabelColor,
  sideNavInkColor,
  whiteLabelAccent,
  accentLabelVars,
} from './themeTokens'

export function builtinTheme(id: string): ThemeDef | undefined {
  return BUILTIN_THEMES.find((theme) => theme.id === id)
}

export function defaultThemeId(appearance: Appearance): string {
  return appearance === 'light' ? DEFAULT_LIGHT_ID : DEFAULT_DARK_ID
}

/** Editor seed for a new custom theme: the default theme's source palette. */
export function defaultCustomInput(appearance: Appearance): CustomThemeInput {
  return appearance === 'light'
    ? { appearance, bgApp: '#f0f2f1', surface: '#ffffff', sideNav: '#e6e9e7', accent: '#0e7a58', text: '#1b211e' }
    : { appearance, bgApp: '#0c100e', surface: '#151b18', sideNav: '#060908', accent: '#40a984', text: '#f2f5f3' }
}

export function newCustomThemeId(): string {
  return `custom-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`
}

export function isCustomThemeId(id: string): boolean {
  return id.startsWith('custom-')
}

/**
 * Inline-style record assigning every token to its CSS var. Used to scope a
 * theme to a subtree (swatches, editor preview): inline vars on a wrapper
 * override the inherited ones, so the token utilities (`bg-app`, `bg-accent`, ...) work inside it.
 */
export function cssVarStyle(tokens: ThemeTokens): CSSProperties {
  const style: Record<string, string> = accentLabelVars(tokens)
  for (const key of THEME_TOKEN_KEYS) {
    style[TOKEN_CSS_VAR[key]] = tokens[key]
  }
  return style as CSSProperties
}

function sanitizeTokens(raw: unknown): ThemeTokens | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const record = raw as Record<string, unknown>
  const out = {} as Record<keyof ThemeTokens, string>
  for (const key of THEME_TOKEN_KEYS) {
    const value = record[key]
    if (typeof value !== 'string' || !value) return null
    out[key] = value
  }
  return out
}

/**
 * The surface paints menus, dialogs and tooltips over other content, so a
 * translucent one is composited over the background it would have shown.
 */
export function withOpaqueSurface(source: CustomThemeInput): CustomThemeInput {
  if ((parseColor(source.surface)?.a ?? 1) >= 1) return source
  return { ...source, surface: flatten(source.surface, source.bgApp) }
}

const SOURCE_COLOR_KEYS = ['bgApp', 'surface', 'sideNav', 'accent', 'text'] as const

function sanitizeSource(raw: unknown): CustomThemeInput | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const record = raw as Record<string, unknown>
  const appearance = record.appearance
  if (appearance !== 'light' && appearance !== 'dark') return null
  const colors = {} as Record<(typeof SOURCE_COLOR_KEYS)[number], string>
  for (const key of SOURCE_COLOR_KEYS) {
    const value = record[key]
    if (typeof value !== 'string' || !isValidColor(value)) return null
    colors[key] = value
  }
  return withOpaqueSurface({ appearance, ...colors })
}

/**
 * Shareable one-line form of a custom theme's editor inputs, e.g.
 * "light,#f0f2f1,#ffffff,#121a16,#0e7a58,#1b211e": appearance, then background,
 * surface, side navigation, accent and text as hex. The field order is the
 * format; a future layout would need a new leading marker.
 */
export function serializeThemeSource(source: CustomThemeInput): string {
  const colors = SOURCE_COLOR_KEYS.map((key) => toHex(source[key]) ?? source[key])
  return [source.appearance, ...colors].join(',')
}

/** Inverse of serializeThemeSource; null for anything that isn't exactly that shape. */
export function parseThemeSource(text: string): CustomThemeInput | null {
  const parts = text
    .trim()
    .split(',')
    .map((part) => part.trim())
  if (parts.length !== SOURCE_COLOR_KEYS.length + 1) return null
  const [appearance, ...colors] = parts
  const hex = colors.map((color) => (color.startsWith('#') ? toHex(color) : null))
  if (hex.some((color) => color === null)) return null
  return sanitizeSource({
    appearance: appearance.toLowerCase(),
    ...Object.fromEntries(SOURCE_COLOR_KEYS.map((key, index) => [key, hex[index]])),
  })
}

/**
 * Validate the persisted `custom_themes` JSON. Entries with a valid source but
 * missing/corrupt tokens, or tokens derived from a surface that has since been
 * flattened, are re-derived; entries without a valid source are dropped. Follows the sanitizeKanbanBoards convention: null = unusable input.
 */
export function sanitizeCustomThemes(raw: unknown): CustomTheme[] | null {
  if (!Array.isArray(raw)) return null
  const out: CustomTheme[] = []
  const seen = new Set<string>()
  for (const item of raw) {
    if (!item || typeof item !== 'object' || Array.isArray(item)) continue
    const record = item as Record<string, unknown>
    if (typeof record.id !== 'string' || !isCustomThemeId(record.id) || seen.has(record.id)) continue
    const source = sanitizeSource(record.source)
    if (!source) continue
    const name = typeof record.name === 'string' && record.name.trim() ? record.name.trim() : 'Custom theme'
    const flattened = source.surface !== (record.source as Record<string, unknown>).surface
    const tokens = (flattened ? null : sanitizeTokens(record.tokens)) ?? deriveThemeTokens(source)
    seen.add(record.id)
    out.push({ id: record.id, name, appearance: source.appearance, tokens, source })
  }
  return out
}
