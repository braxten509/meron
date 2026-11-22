// Minimal color math for theme derivation (lib/themes.ts) and for judging the
// colors an email declares (components/chat/frameTheme.ts). Works on sRGB values
// parsed from hex, `rgb()`/`rgba()` and `hsl()`/`hsla()` strings — the theme
// editor only emits hex, but mail writes all of them.

export type Rgba = { r: number; g: number; b: number; a: number }

const clamp255 = (n: number) => Math.min(255, Math.max(0, Math.round(n)))
const clamp01 = (n: number) => Math.min(1, Math.max(0, n))

/** Parse "#rgb(a)", "#rrggbb(aa)", "rgb()/rgba()" or "hsl()/hsla()". */
export function parseColor(input: string): Rgba | null {
  const text = input.trim()

  const hex = text.match(/^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i)?.[1]
  if (hex) {
    if (hex.length <= 4) {
      return {
        r: parseInt(hex[0] + hex[0], 16),
        g: parseInt(hex[1] + hex[1], 16),
        b: parseInt(hex[2] + hex[2], 16),
        a: hex.length === 4 ? parseInt(hex[3] + hex[3], 16) / 255 : 1,
      }
    }
    return {
      r: parseInt(hex.slice(0, 2), 16),
      g: parseInt(hex.slice(2, 4), 16),
      b: parseInt(hex.slice(4, 6), 16),
      a: hex.length === 8 ? parseInt(hex.slice(6, 8), 16) / 255 : 1,
    }
  }

  const callback = text.match(/^(rgba?|hsla?)\(([^()]*)\)$/i)
  if (callback) {
    const isHsl = callback[1].toLowerCase().startsWith('hsl')
    const args = colorArguments(callback[2], isHsl ? HSL_CHANNELS : RGB_CHANNELS)
    if (!args) return null

    const alpha = args[3] === undefined ? 1 : percentOrNumber(args[3], 1)
    if (alpha === null) return null
    if (isHsl) {
      const hue = degrees(args[0])
      const saturation = percentOrNumber(args[1], 1)
      const lightness = percentOrNumber(args[2], 1)
      if (hue === null || saturation === null || lightness === null) return null
      return hslToRgb(hue, saturation, lightness, clamp01(alpha))
    }
    const [r, g, b] = args.map((arg) => percentOrNumber(arg, 255))
    if (r === null || g === null || b === null) return null
    return { r: clamp255(r), g: clamp255(g), b: clamp255(b), a: clamp01(alpha) }
  }

  return null
}

// One numeric component: a plain number, and — where CSS allows it — a
// percentage. The channel grammars differ: `rgb()` takes numbers or
// percentages, `hsl()` takes an angle and two percentages, and only the alpha
// is free-form in both.
const NUMBER = String.raw`[+-]?(?:\d+\.?\d*|\.\d+)`
const NUMBER_OR_PERCENT = String.raw`${NUMBER}%?`

/** `rgb(a, b, c[, d])` / `hsl(a, b, c[, d])` in the legacy comma grammar. */
function legacyArgs(channels: string[]): RegExp {
  const [first, second, third] = channels
  // Case-insensitive: CSS units are, and `180DEG` is the same angle as `180deg`.
  return new RegExp(
    String.raw`^\s*(${first})\s*,\s*(${second})\s*,\s*(${third})\s*(?:,\s*(${NUMBER_OR_PERCENT})\s*)?$`,
    'i',
  )
}

/** The modern space grammar, with at most one `/` before the alpha. */
function modernArgs(channels: string[]): RegExp {
  const [first, second, third] = channels
  return new RegExp(String.raw`^\s*(${first})\s+(${second})\s+(${third})\s*(?:\/\s*(${NUMBER_OR_PERCENT})\s*)?$`, 'i')
}

const RGB_CHANNELS = [NUMBER_OR_PERCENT, NUMBER_OR_PERCENT, NUMBER_OR_PERCENT]
// A hue is an angle — bare (degrees) or in any of the CSS angle units — never a
// percentage; saturation and lightness are always percentages, so `hsl(0 1 1)`
// and `hsl(50% 50% 50%)` are not colors.
const ANGLE = String.raw`${NUMBER}(?:deg|grad|rad|turn)?`
const HSL_CHANNELS = [ANGLE, String.raw`${NUMBER}%`, String.raw`${NUMBER}%`]

const TURNS: Record<string, number> = { deg: 1, grad: 360 / 400, rad: 180 / Math.PI, turn: 360 }

/** An angle in degrees, whatever unit it was written in. */
function degrees(value: string): number | null {
  const match = value.trim().match(/^([+-]?(?:\d+\.?\d*|\.\d+))(deg|grad|rad|turn)?$/i)
  if (!match) return null
  const number = Number.parseFloat(match[1])
  if (!Number.isFinite(number)) return null
  // CSS units are case-insensitive: `180DEG` is the same angle as `180deg`.
  return number * (match[2] ? TURNS[match[2].toLowerCase()] : 1)
}

/** The three or four components of a color function, in one consistent grammar. */
function colorArguments(args: string, channels: string[]): string[] | null {
  const match = args.match(legacyArgs(channels)) ?? args.match(modernArgs(channels))
  if (!match) return null
  return match.slice(1).filter((arg) => arg !== undefined)
}

/**
 * A component that may be written as a percentage of `full`, e.g. `50%`, or null
 * when it isn't a number at all. The whole token has to be numeric: `12px` is a
 * length, not a channel, and `parseFloat` would happily read it as 12.
 */
function percentOrNumber(value: string, full: number): number | null {
  const token = value.trim()
  if (!/^[+-]?(\d+\.?\d*|\.\d+)%?$/.test(token)) return null
  const number = Number.parseFloat(token)
  if (!Number.isFinite(number)) return null
  return token.endsWith('%') ? (number / 100) * full : number
}

function hslToRgb(hue: number, saturation: number, lightness: number, a: number): Rgba {
  const s = clamp01(saturation)
  const l = clamp01(lightness)
  const chroma = (1 - Math.abs(2 * l - 1)) * s
  const sector = (((hue % 360) + 360) % 360) / 60
  const x = chroma * (1 - Math.abs((sector % 2) - 1))
  const [r, g, b] = (
    [
      [chroma, x, 0],
      [x, chroma, 0],
      [0, chroma, x],
      [0, x, chroma],
      [x, 0, chroma],
      [chroma, 0, x],
    ] as const
  )[Math.floor(sector) % 6]
  const m = l - chroma / 2
  return { r: clamp255((r + m) * 255), g: clamp255((g + m) * 255), b: clamp255((b + m) * 255), a }
}

/** Serialize as "#rrggbb", or "rgba(r, g, b, a)" when alpha < 1. */
export function formatColor({ r, g, b, a }: Rgba): string {
  if (a >= 1) {
    const to2 = (n: number) => clamp255(n).toString(16).padStart(2, '0')
    return `#${to2(r)}${to2(g)}${to2(b)}`
  }
  return `rgba(${clamp255(r)}, ${clamp255(g)}, ${clamp255(b)}, ${Math.round(clamp01(a) * 100) / 100})`
}

/** "#rrggbb", or "#rrggbbaa" when translucent; null if unparseable. */
export function toHex(input: string): string | null {
  const parsed = parseColor(input)
  if (!parsed) return null
  const to2 = (n: number) => clamp255(n).toString(16).padStart(2, '0')
  const alpha = parsed.a >= 1 ? '' : to2(clamp01(parsed.a) * 255)
  return `#${to2(parsed.r)}${to2(parsed.g)}${to2(parsed.b)}${alpha}`
}

/** Linear blend of two colors: weight 0 -> a, 1 -> b. */
export function mix(colorA: string, colorB: string, weight: number): string {
  const a = parseColor(colorA)
  const b = parseColor(colorB)
  if (!a || !b) return colorA
  const w = clamp01(weight)
  return formatColor({
    r: a.r + (b.r - a.r) * w,
    g: a.g + (b.g - a.g) * w,
    b: a.b + (b.b - a.b) * w,
    a: a.a + (b.a - a.a) * w,
  })
}

/** Blend toward white by `amount` (0..1). */
export function lighten(color: string, amount: number): string {
  return mix(color, '#ffffff', amount)
}

/** Blend toward black by `amount` (0..1). */
export function darken(color: string, amount: number): string {
  return mix(color, '#000000', amount)
}

/** Same color with the given alpha. */
export function withAlpha(color: string, alpha: number): string {
  const parsed = parseColor(color)
  if (!parsed) return color
  return formatColor({ ...parsed, a: clamp01(alpha) })
}

/** `color` composited over the opaque `base`, as an opaque color. */
export function flatten(color: string, base: string): string {
  const top = parseColor(color)
  const bottom = parseColor(base)
  if (!top || !bottom) return color
  const blend = (a: number, b: number) => a * top.a + b * (1 - top.a)
  return formatColor({ r: blend(top.r, bottom.r), g: blend(top.g, bottom.g), b: blend(top.b, bottom.b), a: 1 })
}

/** Perceived luminance 0..1 (WCAG-ish, good enough to pick contrasting text). */
export function luminance(color: string): number {
  const parsed = parseColor(color)
  if (!parsed) return 0
  return (0.2126 * parsed.r + 0.7152 * parsed.g + 0.0722 * parsed.b) / 255
}

/** OKLCH coordinates: perceptual lightness 0..1, chroma, hue in degrees. */
export type Oklch = { l: number; c: number; h: number }

const toLinear = (channel: number) => {
  const value = channel / 255
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
}
const fromLinear = (value: number) =>
  255 * (value <= 0.0031308 ? 12.92 * value : 1.055 * Math.max(0, value) ** (1 / 2.4) - 0.055)

/** OKLCH of an opaque color; null if unparseable. */
export function toOklch(color: string): Oklch | null {
  const parsed = parseColor(color)
  if (!parsed) return null
  const [r, g, b] = [toLinear(parsed.r), toLinear(parsed.g), toLinear(parsed.b)]
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b)
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b)
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b)
  const lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s
  const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s
  const bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s
  return { l: lightness, c: Math.hypot(a, bb), h: ((Math.atan2(bb, a) * 180) / Math.PI + 360) % 360 }
}

/** "#rrggbb" for OKLCH coordinates, clipped to sRGB. */
export function fromOklch({ l: lightness, c, h }: Oklch): string {
  const a = c * Math.cos((h * Math.PI) / 180)
  const b = c * Math.sin((h * Math.PI) / 180)
  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3
  return formatColor({
    r: fromLinear(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
    g: fromLinear(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
    b: fromLinear(-0.0041960863 * l - 0.7034186147 * m + 1.7076949135 * s),
    a: 1,
  })
}

/** `color`'s hue and chroma at perceptual lightness `lightness` (0..1). */
export function withLightness(color: string, lightness: number): string {
  const oklch = toOklch(color)
  if (!oklch) return color
  return fromOklch({ ...oklch, l: clamp01(lightness) })
}

export function isValidColor(input: string): boolean {
  return parseColor(input) !== null
}
