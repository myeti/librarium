/**
 * Test helper: reads the theme stylesheets as text, resolves the colour
 * tokens of each theme and measures WCAG contrast ratios.
 *
 * It understands exactly the CSS the themes use, nothing more:
 * - blocks `:root { … }` or `[data-theme=""] { … }` (the default theme,
 *   Nocturne) and `[data-theme="x"] { … }` (the other three), whose custom
 *   properties are merged in that order: the theme blocks come after the
 *   default ones in themes.css, so the cascade does the same. A selector
 *   list (`:root, [data-theme=""] { … }`) feeds each of its selectors'
 *   blocks. The `:root[data-theme="x"]` form is read too;
 *   A `:root` block inside an @media rule is merged too (today only the
 *   reduced-motion durations, which aren't colours): keep colour tokens out
 *   of media queries, or teach this parser about them;
 * - colour values `#rgb`, `#rrggbb`, `rgb()`/`rgba()`, `transparent`, `white`
 *   and `black`, `var(--x)` (resolved through the merged tokens) and
 *   `color-mix(in srgb, A p%, B)` (premultiplied, as browsers do);
 * - `linear-gradient(…)`: every colour stop is returned, so a pair is checked
 *   against each end of the gradient.
 *
 * Anything else throws, so a new kind of value in the themes fails the test
 * loudly instead of being skipped.
 *
 * Contrast follows WCAG 2.x: relative luminance of sRGB colours, ratio
 * (L1 + 0.05) / (L2 + 0.05). A translucent foreground is first composited
 * over its background.
 */

/** An sRGB colour, channels 0–255, alpha 0–1. */
export type Rgba = [number, number, number, number]

/** The `data-theme` value of each theme ('' is the default: Nocturne). */
export const THEMES = ['', 'saphir', 'folklore', 'cerulean'] as const
export type ThemeId = typeof THEMES[number]

/** Custom properties per selector key: ':root' or the data-theme value. */
export type ThemeBlocks = Map<string, Record<string, string>>

/** Collects the custom properties of the `:root` and `:root[data-theme="…"]` blocks. */
export function parseThemeBlocks(...stylesheets: string[]): ThemeBlocks {
    const blocks: ThemeBlocks = new Map()
    for (const css of stylesheets) {
        const text = css.replace(/\/\*[\s\S]*?\*\//g, '')
        const selector = '(?::root(?:\\[data-theme="[^"]*"\\])?|\\[data-theme="[^"]*"\\])'
        const blockRe = new RegExp(`((?:${selector}\\s*,\\s*)*${selector})\\s*\\{([^{}]*)\\}`, 'g')
        for (const m of text.matchAll(blockRe)) {
            const declarations = [...m[2].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)]
            for (const sel of m[1].split(',')) {
                // `data-theme=""` is the default theme, like `:root`.
                const theme = sel.match(/data-theme="([^"]*)"/)
                const key = theme && theme[1] !== '' ? theme[1] : ':root'
                const decls = blocks.get(key) ?? {}
                for (const d of declarations) {
                    decls[d[1]] = d[2].trim()
                }
                blocks.set(key, decls)
            }
        }
    }
    return blocks
}

/** The tokens a theme ends up with: `:root`, overridden by its own block. */
export function themeTokens(blocks: ThemeBlocks, theme: ThemeId): Record<string, string> {
    const base = blocks.get(':root') ?? {}
    return theme === '' ? { ...base } : { ...base, ...(blocks.get(theme) ?? {}) }
}

const NAMED: Record<string, Rgba> = {
    transparent: [0, 0, 0, 0],
    white: [255, 255, 255, 1],
    black: [0, 0, 0, 1],
}

/** Splits on commas that are not inside parentheses. */
function splitTopLevel(s: string): string[] {
    const parts: string[] = []
    let depth = 0
    let start = 0
    for (let i = 0; i < s.length; i++) {
        if (s[i] === '(') { depth++ } else if (s[i] === ')') { depth-- } else if (s[i] === ',' && depth === 0) {
            parts.push(s.slice(start, i).trim())
            start = i + 1
        }
    }
    parts.push(s.slice(start).trim())
    return parts
}

function parseLiteral(value: string): Rgba | null {
    const v = value.trim().toLowerCase()
    if (v in NAMED) { return NAMED[v] }
    let m = v.match(/^#([0-9a-f]{3})$/)
    if (m) { return [...m[1]].map(x => parseInt(x + x, 16)).concat(1) as Rgba }
    m = v.match(/^#([0-9a-f]{6})$/)
    if (m) { return [0, 2, 4].map(i => parseInt(m![1].slice(i, i + 2), 16)).concat(1) as Rgba }
    m = v.match(/^rgba?\(([^)]*)\)$/)
    if (m) {
        const p = m[1].split(/[\s,/]+/).filter(Boolean).map(Number)
        if (p.length < 3 || p.some(Number.isNaN)) { return null }
        return [p[0], p[1], p[2], p[3] ?? 1]
    }
    return null
}

/** CSS color-mix in srgb: premultiplied interpolation, then un-premultiplied. */
function colorMix(a: Rgba, pa: number, b: Rgba, pb: number): Rgba {
    const alpha = a[3] * pa + b[3] * pb
    if (alpha === 0) { return [0, 0, 0, 0] }
    const ch = (i: number) => (a[i] * a[3] * pa + b[i] * b[3] * pb) / alpha
    return [ch(0), ch(1), ch(2), alpha]
}

/**
 * Resolves a token value to its colours: one for a plain colour, every stop
 * for a gradient. Throws on anything it doesn't understand.
 */
export function resolveColors(tokens: Record<string, string>, value: string, depth = 0): Rgba[] {
    if (depth > 20) { throw new Error(`var() cycle resolving ${value}`) }
    const v = value.trim()
    const literal = parseLiteral(v)
    if (literal) { return [literal] }

    let m = v.match(/^var\((--[\w-]+)(?:\s*,\s*(.+))?\)$/)
    if (m) {
        const ref = tokens[m[1]] ?? m[2]
        if (ref === undefined) { throw new Error(`undefined token ${m[1]}`) }
        return resolveColors(tokens, ref, depth + 1)
    }

    m = v.match(/^color-mix\(\s*in srgb\s*,(.*)\)$/s)
    if (m) {
        const [first, second] = splitTopLevel(m[1])
        const parseStop = (stop: string): { color: Rgba, pct: number | null } => {
            const pm = stop.match(/^(.*?)\s+([\d.]+)%$/s)
            const colorText = pm ? pm[1] : stop
            const colors = resolveColors(tokens, colorText, depth + 1)
            if (colors.length !== 1) { throw new Error(`gradient inside color-mix: ${stop}`) }
            return { color: colors[0], pct: pm ? Number(pm[2]) / 100 : null }
        }
        const a = parseStop(first)
        const b = parseStop(second)
        const pa = a.pct ?? (b.pct === null ? 0.5 : 1 - b.pct)
        const pb = b.pct ?? 1 - pa
        return [colorMix(a.color, pa, b.color, pb)]
    }

    m = v.match(/^linear-gradient\((.*)\)$/s)
    if (m) {
        const stops = splitTopLevel(m[1]).filter(s => !/^(to\s|[\d.-]+deg)/.test(s))
        return stops.map(s => {
            const colorText = s.replace(/\s+[\d.]+%$/, '')
            const colors = resolveColors(tokens, colorText, depth + 1)
            if (colors.length !== 1) { throw new Error(`nested gradient: ${s}`) }
            return colors[0]
        })
    }

    throw new Error(`can't resolve colour "${value}"`)
}

/** `fg` composited over `bg` (bg must be opaque for a meaningful result). */
export function over(fg: Rgba, bg: Rgba): Rgba {
    const a = fg[3]
    return [fg[0] * a + bg[0] * (1 - a), fg[1] * a + bg[1] * (1 - a), fg[2] * a + bg[2] * (1 - a), 1]
}

function channel(v: number): number {
    const c = v / 255
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
}

/** WCAG relative luminance. */
export function luminance(c: Rgba): number {
    return 0.2126 * channel(c[0]) + 0.7152 * channel(c[1]) + 0.0722 * channel(c[2])
}

/** WCAG contrast ratio of `fg` on an opaque `bg` (fg composited first). */
export function contrast(fg: Rgba, bg: Rgba): number {
    const f = over(fg, bg)
    const [hi, lo] = [luminance(f), luminance(bg)].sort((x, y) => y - x)
    return (hi + 0.05) / (lo + 0.05)
}

/**
 * A theme's colours, for writing pair checks:
 * `colors('--bg-modal')` returns every stop of the modal gradient;
 * `on('--accent-muted', '--bg')` composites a translucent layer over a
 * surface, the way a tinted background is actually seen.
 */
export function themePalette(tokens: Record<string, string>) {
    const colors = (token: string): Rgba[] => {
        const value = tokens[token]
        if (value === undefined) { throw new Error(`theme has no ${token}`) }
        return resolveColors(tokens, value)
    }
    const on = (layer: string, base: string): Rgba[] =>
        colors(base).flatMap(b => colors(layer).map(l => over(l, over(b, NAMED.white))))
    return { colors, on }
}
