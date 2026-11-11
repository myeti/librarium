import { defineStore } from 'pinia'
import { saveConfigOrThrow } from '@/services/appConfig'

/**
 * A theme: a book cover, dark like the covers it comes from. Each brings
 * its palette, its lettering, its ornaments, its generated covers and its
 * background effect (themes.css, docs/DESIGN.md).
 */
export type ThemeName = 'nocturne' | 'saphir' | 'folklore' | 'cerulean'

/** In the order Settings shows them: the default first. */
export const THEME_NAMES: readonly ThemeName[] = ['nocturne', 'saphir', 'folklore', 'cerulean']

/** The theme of a first launch. */
export const DEFAULT_THEME: ThemeName = 'nocturne'

/**
 * Older theme names (four seasons, and "seasonal", which picked one by
 * date), and the theme each is read as: the nearest in colour and mood.
 */
const RETIRED_THEMES: Readonly<Record<string, ThemeName>> = {
    seasonal: 'saphir',
    autumn: 'folklore',  // warm reds and amber
    winter: 'cerulean',  // snow, cool silver-blue
    spring: 'nocturne',  // blossom pinks → coral
    summer: 'saphir',    // azure → sapphire
}

/** Whether a saved value is a theme this version knows. */
export function isThemeName(value: unknown): value is ThemeName {
    return typeof value === 'string' && (THEME_NAMES as readonly string[]).includes(value)
}

/**
 * The theme a saved setting stands for: a current theme as is, a retired
 * one (0.5's seasons) as its successor, anything else `null` (the default
 * is kept).
 */
export function themeNameFrom(value: unknown): ThemeName | null {
    if (isThemeName(value)) { return value }
    if (typeof value === 'string' && Object.prototype.hasOwnProperty.call(RETIRED_THEMES, value)) { return RETIRED_THEMES[value] }
    return null
}

/**
 * The `data-theme` attribute of a theme: '' for the default (Nocturne,
 * `:root` in themes.css), else the theme's name.
 */
export function dataThemeOf(theme: ThemeName): string {
    return theme === DEFAULT_THEME ? '' : theme
}

interface ThemeState {
    theme: ThemeName
}

/**
 * The colour theme, applied as `data-theme` on <html> (themes.css). App.vue
 * calls `applyTheme()` whenever `dataTheme` changes. Every theme is dark:
 * there is no light mode (a saved `themeMode` from 0.6 previews is ignored).
 *
 * `setTheme` applies at once and saves in the app config; it rejects when
 * the setting couldn't be saved (it stays applied for this session), so
 * Settings can say so.
 */
export const useThemeStore = defineStore('theme', {
    state: (): ThemeState => ({
        theme: DEFAULT_THEME,
    }),

    getters: {
        /** The `data-theme` attribute for the current theme ('' = default palette). */
        dataTheme: (state): string => dataThemeOf(state.theme),
    },

    actions: {
        applyTheme() {
            const attr = this.dataTheme
            if (attr) {
                document.documentElement.setAttribute('data-theme', attr)
            } else {
                document.documentElement.removeAttribute('data-theme')
            }
        },

        setTheme(name: ThemeName): Promise<void> {
            this.theme = name
            this.applyTheme()
            return saveConfigOrThrow({ theme: name })
        },
    },
})
