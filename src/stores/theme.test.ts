import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { backendError, mockInvoke } from '@/test/tauri'
import { dataThemeOf, isThemeName, themeNameFrom, THEME_NAMES, useThemeStore } from './theme'

describe('dataThemeOf', () => {
    it('names the themes as themes.css does (Nocturne is the default)', () => {
        expect(dataThemeOf('nocturne')).toBe('')
        expect(dataThemeOf('saphir')).toBe('saphir')
        expect(dataThemeOf('cerulean')).toBe('cerulean')
    })
})

describe('themeNameFrom', () => {
    it('keeps the current themes', () => {
        for (const name of THEME_NAMES) { expect(themeNameFrom(name)).toBe(name) }
    })

    it('moves the seasonal themes of 0.5 to their successors', () => {
        expect(themeNameFrom('seasonal')).toBe('saphir')
        expect(themeNameFrom('autumn')).toBe('folklore')
        expect(themeNameFrom('winter')).toBe('cerulean')
        expect(themeNameFrom('spring')).toBe('nocturne')
        expect(themeNameFrom('summer')).toBe('saphir')
    })

    it('ignores anything else', () => {
        expect(themeNameFrom('sepia')).toBeNull()
        expect(themeNameFrom('constructor')).toBeNull()
        expect(themeNameFrom(undefined)).toBeNull()
        expect(isThemeName('autumn')).toBe(false)
    })
})

describe('theme store', () => {
    beforeEach(() => {
        setActivePinia(createPinia())
        mockInvoke.reset()
        mockInvoke.on('set_config', () => undefined)
        document.documentElement.removeAttribute('data-theme')
    })

    it('starts as Nocturne, with no data-theme attribute', () => {
        const theme = useThemeStore()
        expect(theme.theme).toBe('nocturne')
        theme.applyTheme()
        expect(document.documentElement.hasAttribute('data-theme')).toBe(false)
    })

    it('applies a theme at once and saves it', async () => {
        const theme = useThemeStore()
        await theme.setTheme('saphir')
        expect(document.documentElement.getAttribute('data-theme')).toBe('saphir')
        await theme.setTheme('nocturne')
        expect(document.documentElement.hasAttribute('data-theme')).toBe(false)
        expect(mockInvoke.callsTo('set_config').map(c => c.args)).toEqual([
            { updates: { theme: 'saphir' } },
            { updates: { theme: 'nocturne' } },
        ])
    })

    it('keeps a change for the session when it can’t be saved, and says so (rejects)', async () => {
        mockInvoke.on('set_config', () => backendError({ code: 'IO', message: 'disk full' }))
        const theme = useThemeStore()
        await expect(theme.setTheme('folklore')).rejects.toMatchObject({ message: 'disk full' })
        expect(theme.theme).toBe('folklore')
        expect(document.documentElement.getAttribute('data-theme')).toBe('folklore')
    })
})
