import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import {
    contrast, parseThemeBlocks, themePalette, themeTokens, THEMES,
    type Rgba, type ThemeId,
} from '@/test/themeTokens'

/**
 * WCAG AA contrast for every theme, read from the stylesheets themselves, so
 * a palette edit that makes text unreadable fails here instead of shipping.
 *
 * Thresholds (WCAG 2.2): 4.5:1 for normal text (everything in this app is
 * below 18.66px bold / 24px), 3:1 for non-text UI such as the focus ring.
 * Each pair is checked against every surface the text is drawn on,
 * including both ends of the gradients and the coloured glows of the page
 * background (worst case: the glow at full strength).
 */

const TEXT = 4.5
const UI = 3

const COVERS = ['--cover-1', '--cover-2', '--cover-3', '--cover-4', '--cover-5', '--cover-6']

/** Every token each theme must define itself (no inheriting Nocturne's). */
const THEME_COLOUR_TOKENS = [
    '--bg', '--bg-elevated', '--bg-hover', '--bg-input', '--bg-card', '--bg-card-hover', '--bg-modal',
    '--bg-cover-placeholder', '--bg-filter-bar', '--bg-header-sticky', '--glass',
    '--sky', '--body-overlay-1', '--body-overlay-2', '--motif', '--grain-opacity',
    '--modal-backdrop', '--border', '--border-hover', '--hairline', '--foil',
    '--text-primary', '--text-secondary', '--text-muted',
    '--accent', '--accent-strong', '--accent-dark', '--accent-muted', '--accent-text', '--on-accent',
    '--secondary', '--secondary-muted', '--secondary-text',
    '--danger', '--danger-muted', '--danger-strong', '--on-danger',
    '--success', '--success-muted', '--warning', '--warning-muted',
    '--fmt-epub', '--fmt-pdf', '--fmt-mobi', '--fmt-azw3', '--fmt-cbz',
    '--shadow-sm', '--shadow', '--shadow-lg', '--shadow-popover', '--glow-card', '--glow-focus',
    '--scrollbar-color', '--scrollbar-color-hover', '--select-arrow',
    // The theme's type, ornaments and generated covers.
    '--font-display', '--font-heading', '--font-serif', '--display-weight', '--display-tracking',
    '--heading-weight', '--heading-tracking', '--serif-weight', '--serif-scale',
    ...COVERS, '--cover-ink', '--cover-ink-soft', '--cover-foil',
]

// Read from disk: vitest stubs CSS imports (even ?raw) to empty strings.
const read = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8')
const blocks = parseThemeBlocks(read('./main.css'), read('./themes.css'))
const label = (theme: ThemeId) => theme || 'nocturne'

/** Lowest contrast of `fg` over all of the given backgrounds. */
function worst(fg: Rgba[], backgrounds: Rgba[][]): number {
    return Math.min(...backgrounds.flat().flatMap(bg => fg.map(f => contrast(f, bg))))
}

describe.each(THEMES.map(t => [label(t), t] as const))('theme %s', (_name, theme) => {
    const tokens = themeTokens(blocks, theme)
    const { colors, on } = themePalette(tokens)

    // Where text sits: the page (its sky, with the glows), raised surfaces, dialogs, cards.
    const page = [
        colors('--bg'), colors('--sky'),
        on('--body-overlay-1', '--bg'), on('--body-overlay-2', '--bg'),
        on('--body-overlay-1', '--sky'), on('--body-overlay-2', '--sky'),
    ]
    const surfaces = [colors('--bg-elevated'), colors('--bg-modal'), colors('--bg-card')]
    const everywhere = [...page, ...surfaces, colors('--bg-hover'), on('--bg-input', '--bg-modal')]

    it('defines every colour token itself', () => {
        const own = theme === '' ? blocks.get(':root') : blocks.get(theme)
        expect(own, `no block for theme "${theme}"`).toBeDefined()
        const missing = THEME_COLOUR_TOKENS.filter(t => own![t] === undefined)
        expect(missing).toEqual([])
    })

    it.each(['--text-primary', '--text-secondary', '--text-muted'])('%s is readable on every surface', (token) => {
        expect(worst(colors(token), everywhere)).toBeGreaterThanOrEqual(TEXT)
    })

    it('text on the cover placeholder (list rows, the panel) is readable', () => {
        const cover = [colors('--bg-cover-placeholder')]
        expect(worst(colors('--text-primary'), cover)).toBeGreaterThanOrEqual(TEXT)
        expect(worst(colors('--text-secondary'), cover)).toBeGreaterThanOrEqual(TEXT)
    })

    it('generated covers: the title and the author/series ink are readable on every cover colour', () => {
        const covers = COVERS.map(c => colors(c))
        expect(worst(colors('--cover-ink'), covers)).toBeGreaterThanOrEqual(TEXT)
        expect(worst(colors('--cover-ink-soft'), covers)).toBeGreaterThanOrEqual(TEXT)
    })

    it('keeps the text hierarchy: secondary is visibly stronger than muted', () => {
        const bg = [colors('--bg')]
        // At least 10% more contrast, or the two levels look the same.
        expect(worst(colors('--text-secondary'), bg)).toBeGreaterThan(1.1 * worst(colors('--text-muted'), bg))
        expect(worst(colors('--text-primary'), bg)).toBeGreaterThan(1.1 * worst(colors('--text-secondary'), bg))
    })

    it('accent text is readable on surfaces and on the accent tint (selected items)', () => {
        const backgrounds = [...page, ...surfaces, on('--accent-muted', '--bg'), on('--accent-muted', '--bg-elevated')]
        expect(worst(colors('--accent-text'), backgrounds)).toBeGreaterThanOrEqual(TEXT)
    })

    it('the primary button label is readable at rest and on hover', () => {
        const label = colors('--on-accent')
        expect(worst(label, [colors('--accent-strong'), colors('--accent-dark')])).toBeGreaterThanOrEqual(TEXT)
        expect(worst(label, [colors('--accent-hover')])).toBeGreaterThanOrEqual(TEXT)
    })

    it('the active navigation item is readable (accent text on its tint, over the sidebar glass)', () => {
        // AppSidebar: .router-link-active { color: --accent-text; background: --accent-muted }
        const backgrounds = [on('--accent-muted', '--bg'), on('--accent-muted', '--sky'), on('--glass', '--bg')]
        expect(worst(colors('--accent-text'), backgrounds)).toBeGreaterThanOrEqual(TEXT)
    })

    it('secondary text is readable on its tint (chips, badges)', () => {
        const backgrounds = [on('--secondary-muted', '--bg'), on('--secondary-muted', '--bg-elevated')]
        expect(worst(colors('--secondary-text'), backgrounds)).toBeGreaterThanOrEqual(TEXT)
    })

    it.each(['--danger', '--success', '--warning'])('%s text is readable on surfaces and on its muted tint', (token) => {
        const muted = `${token}-muted`
        const backgrounds = [...page, ...surfaces, on(muted, '--bg'), on(muted, '--bg-elevated')]
        expect(worst(colors(token), backgrounds)).toBeGreaterThanOrEqual(TEXT)
    })

    it('a neutral badge (secondary text on its 10% tint) is readable', () => {
        const tinted = { ...tokens, '--badge-bg': 'color-mix(in srgb, var(--text-secondary) 10%, transparent)' }
        const badge = themePalette(tinted)
        const backgrounds = [badge.on('--badge-bg', '--bg'), badge.on('--badge-bg', '--bg-elevated'), badge.on('--badge-bg', '--bg-hover')]
        expect(worst(colors('--text-secondary'), backgrounds)).toBeGreaterThanOrEqual(TEXT)
    })

    it('a solid danger fill keeps its label readable (danger button on hover)', () => {
        expect(worst(colors('--on-danger'), [colors('--danger-strong')])).toBeGreaterThanOrEqual(TEXT)
    })

    it.each(['--fmt-epub', '--fmt-pdf', '--fmt-mobi', '--fmt-azw3', '--fmt-cbz'])('%s badges are readable on their tint', (fmt) => {
        // main.css: .tag-<format> { color: fmt; background: fmt 10% }, on panels and on the page.
        const tinted = { ...tokens, '--tag-bg': `color-mix(in srgb, var(${fmt}) 10%, transparent)` }
        const tag = themePalette(tinted)
        const backgrounds = [tag.on('--tag-bg', '--bg-elevated'), tag.on('--tag-bg', '--bg'), tag.on('--tag-bg', '--bg-hover')]
        expect(worst(colors(fmt), backgrounds)).toBeGreaterThanOrEqual(TEXT)
    })

    it('the focus ring and the accent stand out from every surface (3:1, non-text)', () => {
        expect(worst(colors('--focus-ring-color'), everywhere)).toBeGreaterThanOrEqual(UI)
        expect(worst(colors('--accent'), [...page, ...surfaces])).toBeGreaterThanOrEqual(UI)
    })
})

describe('global stylesheet', () => {
    // Comments may mention Bulma (history); the rules may not use it.
    const main = read('./main.css').replace(/\/\*[\s\S]*?\*\//g, '')

    it('does not bring Bulma back, nor !important overrides', () => {
        expect(main).not.toMatch(/bulma/i)
        expect(main).not.toMatch(/!important/)
    })

    it('has no text smaller than 12px', () => {
        const sizes = [...main.matchAll(/font-size:\s*(\d+(?:\.\d+)?)px/g)].map(m => Number(m[1]))
        expect(sizes.filter(px => px < 12)).toEqual([])
    })
})

describe('theme token parser', () => {
    it('feeds a selector list to each of its selectors', () => {
        const parsed = parseThemeBlocks(':root, [data-theme=""], [data-theme="x"] { --a: #fff; } [data-theme="x"] { --b: #000; }')
        expect(parsed.get(':root')).toEqual({ '--a': '#fff' })
        expect(parsed.get('x')).toEqual({ '--a': '#fff', '--b': '#000' })
    })

    it('reads all four themes', () => {
        expect([...blocks.keys()].sort()).toEqual([':root', ...THEMES.filter(Boolean)].sort())
    })

    it('resolves var(), color-mix() and gradients', () => {
        const { colors } = themePalette({
            '--a': '#ff0000',
            '--b': 'var(--a)',
            '--half': 'color-mix(in srgb, var(--a) 50%, transparent)',
            '--grad': 'linear-gradient(90deg, #000 0%, var(--b) 100%)',
        })
        expect(colors('--b')).toEqual([[255, 0, 0, 1]])
        expect(colors('--half')).toEqual([[255, 0, 0, 0.5]])
        expect(colors('--grad')).toEqual([[0, 0, 0, 1], [255, 0, 0, 1]])
    })

    it('fails loudly on a value it does not understand', () => {
        const { colors } = themePalette({ '--x': 'oklch(0.7 0.1 30)' })
        expect(() => colors('--x')).toThrow(/can't resolve/)
    })

    it('measures WCAG contrast like the reference values', () => {
        expect(contrast([0, 0, 0, 1], [255, 255, 255, 1])).toBeCloseTo(21, 5)
        expect(contrast([118, 118, 118, 1], [255, 255, 255, 1])).toBeCloseTo(4.54, 2)
    })
})
