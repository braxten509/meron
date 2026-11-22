// Derives desktop theme tokens from editable colors and an appearance mode.
// The palette registry and theme editor share contrast, accent and surface calculations.
// The CSS mapping defines how computed values reach each part of the interface.

import { darken, flatten, fromOklch, lighten, mix, parseColor, toHex, toOklch, withAlpha, withLightness } from './color'

// Theme registry and derivation. A theme is a complete set of values for the
// `--me-*` CSS custom properties in index.css. Built-ins live here; custom
// themes are derived from a handful of editor inputs (CustomThemeInput) and
// persisted with their full token map in settings (`custom_themes`).

export type Appearance = 'light' | 'dark'

/**
 * One value per CSS custom property slot. Colors are CSS color strings;
 * the two shadow slots are full box-shadow values.
 */
export type ThemeTokens = {
  bgApp: string
  bgChat: string
  bgChatOverlay: string
  bgSideNav: string
  bgChats: string
  bgHeader: string
  bgHover: string
  bgRaised: string
  bgActive: string
  border: string
  textPrimary: string
  textSecondary: string
  accent: string
  accentHover: string
  bubbleIn: string
  bubbleInText: string
  bubbleOut: string
  bubbleOutText: string
  composerBg: string
  composerBorder: string
  bubbleShadowIn: string
  bubbleShadowOut: string
}

export const TOKEN_CSS_VAR: Record<keyof ThemeTokens, string> = {
  bgApp: '--me-bg-app',
  bgChat: '--me-bg-chat',
  bgChatOverlay: '--me-bg-chat-overlay',
  bgSideNav: '--me-bg-sidenav',
  bgChats: '--me-bg-chats',
  bgHeader: '--me-bg-header',
  bgHover: '--me-bg-hover',
  bgRaised: '--me-bg-raised',
  bgActive: '--me-bg-active',
  border: '--me-border',
  textPrimary: '--me-text-primary',
  textSecondary: '--me-text-secondary',
  accent: '--me-accent',
  accentHover: '--me-accent-hover',
  bubbleIn: '--me-bubble-in',
  bubbleInText: '--me-bubble-in-text',
  bubbleOut: '--me-bubble-out',
  bubbleOutText: '--me-bubble-out-text',
  composerBg: '--me-composer-bg',
  composerBorder: '--me-composer-border',
  bubbleShadowIn: '--me-bubble-shadow-in',
  bubbleShadowOut: '--me-bubble-shadow-out',
}

export const THEME_TOKEN_KEYS = Object.keys(TOKEN_CSS_VAR) as (keyof ThemeTokens)[]

export type ThemeDef = {
  /** Built-ins: "light", "dark", "mist", ... Custom themes: "custom-<random>". */
  id: string
  name: string
  appearance: Appearance
  tokens: ThemeTokens
}

/** The 6 inputs the custom theme editor exposes; everything else is derived. */
export type CustomThemeInput = {
  appearance: Appearance
  /** Window background behind everything. */
  bgApp: string
  /** Panels: thread list, header, composer, incoming bubbles. */
  surface: string
  /** The account rail / side navigation. */
  sideNav: string
  accent: string
  /** Primary text color. */
  text: string
}

export type CustomTheme = ThemeDef & { source: CustomThemeInput }

/**
 * Place related surfaces at consistent OKLCH lightness offsets.
 *
 * Pane and row colors are relative to the list surface. Bubbles are relative to the conversation
 * background. Each token retains its hue and chroma.
 *
 * The rail is darkest, followed by the conversation and the list. In dark themes the bubbles and
 * row states rise above those surfaces. Light themes use smaller steps between tones.
 */
export const SURFACE_LADDER = {
  light: {
    chat: -0.035,
    sideNav: -0.07,
    hover: -0.045,
    active: -0.08,
    border: -0.09,
    bubbleIn: 0.035,
    bubbleOut: -0.035,
  },
  dark: { chat: -0.04, sideNav: -0.08, hover: 0.05, active: 0.08, border: 0.1, bubbleIn: 0.09, bubbleOut: 0.16 },
} satisfies Record<Appearance, Record<string, number>>

/** WCAG AA for normal text: secondary text and the accent (sender names, links, active tabs). */
const TEXT_MIN_CONTRAST = 4.5

/** `color` moved in lightness, away from `backgrounds`, until it reaches `min` contrast on all of them. */
function withMinContrast(color: string, backgrounds: string[], min: number, direction: 1 | -1): string {
  const oklch = toOklch(color)
  if (!oklch) return color
  let out = toHex(color) ?? color
  // Stop only at the end of the scale being moved toward: white sits at the
  // top of it and can still darken, black at the bottom and can still lighten.
  const hasRoom = () => (direction < 0 ? oklch.l > 0 : oklch.l < 1)
  while (Math.min(...backgrounds.map((bg) => contrastRatio(out, bg))) < min && hasRoom()) {
    oklch.l = Math.min(1, Math.max(0, oklch.l + direction * 0.005))
    out = fromOklch(oklch)
  }
  return out
}

/** `ink` at the alpha that lands on lightness `target` when composited over `base`. */
function tintToLightness(ink: string, base: string, target: number): string {
  let low = 0
  let high = 1
  for (let step = 0; step < 20; step++) {
    const alpha = (low + high) / 2
    const lightness = toOklch(flatten(withAlpha(ink, alpha), base))?.l ?? target
    const baseLightness = toOklch(base)?.l ?? target
    // Past the target in the ink's direction: back off.
    if (Math.abs(lightness - baseLightness) > Math.abs(target - baseLightness)) high = alpha
    else low = alpha
  }
  return withAlpha(ink, Math.round(((low + high) / 2) * 1000) / 1000)
}

/**
 * `text` made readable on `bg`: moved further to the side of `bg` it already
 * sits on (lighter or darker), or across to the other side when that side
 * runs out of room before reaching `min`.
 */
function readableOn(text: string, bg: string, min: number): string {
  const side = (toOklch(text)?.l ?? 0) > (toOklch(bg)?.l ?? 0) ? 1 : -1
  const kept = withMinContrast(text, [bg], min, side)
  return contrastRatio(kept, bg) >= min ? kept : withMinContrast(text, [bg], min, side === 1 ? -1 : 1)
}

/**
 * Place a theme's surfaces on SURFACE_LADDER and bring its secondary text,
 * accent and bubble text up to TEXT_MIN_CONTRAST. Custom themes are derived through this, and
 * every built-in is written as its output (themes.test.ts holds them to it).
 * The side nav is left alone: it is a custom-theme editor input.
 */
export function applySurfaceRules(tokens: ThemeTokens, appearance: Appearance): ThemeTokens {
  const light = appearance === 'light'
  const ladder = SURFACE_LADDER[appearance]
  const list = toOklch(tokens.bgChats)?.l ?? 0
  const bgChat = withLightness(tokens.bgChat, list + ladder.chat)
  const chat = list + ladder.chat
  const border = withLightness(flatten(tokens.border, tokens.bgChats), list + ladder.border)
  const bubbleIn = withLightness(tokens.bubbleIn, chat + ladder.bubbleIn)
  // Text darkens on a light theme and lightens on a dark one to gain contrast.
  const away = light ? -1 : 1
  const accent = withMinContrast(tokens.accent, [tokens.bgChats, bubbleIn], TEXT_MIN_CONTRAST, away)
  const accentLightness = toOklch(accent)?.l ?? 0
  const bubbleOut = withLightness(tokens.bubbleOut, chat + ladder.bubbleOut)
  return {
    ...tokens,
    bgChat,
    bgChatOverlay: withAlpha(bgChat, light ? 0.94 : 0.95),
    // Translucent, so a hover reads the same on the list, the conversation
    // background and the bubbles alike; tuned to the ladder over the list.
    bgHover: tintToLightness(tokens.textPrimary, tokens.bgChats, list + ladder.hover),
    bgActive: withLightness(flatten(tokens.bgActive, tokens.bgChats), list + ladder.active),
    border,
    composerBorder: border,
    textSecondary: withMinContrast(tokens.textSecondary, [tokens.bgChats, bgChat], TEXT_MIN_CONTRAST, away),
    accent,
    accentHover: withLightness(accent, accentLightness + (light ? -0.05 : 0.04)),
    bubbleIn,
    // The bubbles moved, so their text follows to stay readable on them.
    bubbleInText: readableOn(tokens.bubbleInText, bubbleIn, TEXT_MIN_CONTRAST),
    bubbleOut,
    bubbleOutText: readableOn(tokens.bubbleOutText, bubbleOut, TEXT_MIN_CONTRAST),
  }
}

/**
 * Expand the 6 editor inputs into a full token map. Formulas are tuned so the
 * Meron Light/Dark inputs reproduce (closely) the hand-picked index.css values.
 */
export function deriveThemeTokens(input: CustomThemeInput): ThemeTokens {
  const { appearance, bgApp, surface, sideNav, accent, text } = input
  const light = appearance === 'light'

  const bgChat = bgApp
  const border = mix(surface, text, 0.12)
  return applySurfaceRules(
    {
      bgApp,
      bgChat,
      bgChatOverlay: withAlpha(bgChat, light ? 0.94 : 0.95),
      bgSideNav: sideNav,
      bgChats: surface,
      bgHeader: surface,
      bgHover: light ? mix(surface, text, 0.06) : withAlpha(lighten(surface, 0.08), 0.6),
      bgRaised: light ? mix(surface, bgApp, 0.5) : withAlpha(surface, 0.4),
      bgActive: light ? mix(surface, text, 0.12) : lighten(surface, 0.1),
      border,
      textPrimary: text,
      textSecondary: mix(text, bgApp, 0.45),
      accent,
      accentHover: light ? darken(accent, 0.1) : lighten(accent, 0.12),
      bubbleIn: light ? surface : lighten(surface, 0.08),
      bubbleInText: text,
      bubbleOut: light ? mix(accent, '#ffffff', 0.8) : mix(accent, surface, 0.55),
      bubbleOutText: light ? darken(accent, 0.45) : mix(accent, '#ffffff', 0.75),
      composerBg: surface,
      composerBorder: border,
      bubbleShadowIn: light
        ? `0 2px 8px -2px ${withAlpha(text, 0.08)}, 0 1px 3px -1px ${withAlpha(text, 0.04)}`
        : '0 4px 12px -3px rgba(0, 0, 0, 0.3), 0 1px 4px -2px rgba(0, 0, 0, 0.2)',
      bubbleShadowOut: light
        ? `0 2px 8px -2px ${withAlpha(accent, 0.12)}, 0 1px 3px -1px ${withAlpha(accent, 0.06)}`
        : '0 4px 12px -3px rgba(0, 0, 0, 0.4), 0 1px 4px -2px rgba(0, 0, 0, 0.3)',
    },
    appearance,
  )
}

/** WCAG relative luminance of an opaque color; null if unparseable. */
function relativeLuminance(color: string): number | null {
  const rgb = parseColor(color)
  if (!rgb) return null
  const linear = [rgb.r, rgb.g, rgb.b].map((channel) => {
    const value = channel / 255
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  })
  return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
}

/** WCAG contrast ratio between two opaque colors, 1..21; 1 if either is unparseable. */
export function contrastRatio(a: string, b: string): number {
  const la = relativeLuminance(a)
  const lb = relativeLuminance(b)
  if (la === null || lb === null) return 1
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05)
}

/**
 * Label for an opaque fill: white whenever it meets WCAG AA there, as labels
 * and icons on a colored button usually are, else whichever of white and
 * black contrasts more.
 */
export function accentLabelColor(color: string): string {
  const luminance = relativeLuminance(color)
  if (luminance === null) return '#ffffff'
  const white = 1.05 / (luminance + 0.05)
  return white >= TEXT_MIN_CONTRAST || white >= (luminance + 0.05) / 0.05 ? '#ffffff' : '#000000'
}

/** WCAG AA for normal text. */
const SIDE_NAV_INK_MIN_CONTRAST = 4.5

/**
 * Foreground for glyphs, dividers and hover tints on the side nav: white on a
 * dark rail; on a light one, the theme's text color when it reads against the
 * rail itself, else black.
 */
export function sideNavInkColor(tokens: ThemeTokens): string {
  if (accentLabelColor(tokens.bgSideNav) === '#ffffff') return '#ffffff'
  return contrastRatio(tokens.textPrimary, tokens.bgSideNav) >= SIDE_NAV_INK_MIN_CONTRAST
    ? tokens.textPrimary
    : '#000000'
}

/**
 * The accent, darkened just enough for white text to meet WCAG AA on it: the
 * fill for count badges, which read as white numbers on every theme. Accents
 * that already pass come back unchanged.
 */
export function whiteLabelAccent(accent: string): string {
  const oklch = toOklch(accent)
  if (!oklch) return accent
  let fill = toHex(accent) ?? accent
  while (contrastRatio('#ffffff', fill) < 4.5 && oklch.l > 0) {
    oklch.l = Math.max(0, oklch.l - 0.01)
    fill = fromOklch(oklch)
  }
  return fill
}

// Derived rather than persisted so existing custom themes gain readable labels too.
export function accentLabelVars(tokens: ThemeTokens): Record<string, string> {
  return {
    '--me-accent-label': accentLabelColor(tokens.accent),
    '--me-accent-hover-label': accentLabelColor(tokens.accentHover),
    '--me-sidenav-ink': sideNavInkColor(tokens),
    '--me-accent-badge': whiteLabelAccent(tokens.accent),
  }
}
