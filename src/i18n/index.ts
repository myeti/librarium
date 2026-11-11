/**
 * Translations: `t('section.key', { name: value })` in code, `$t(...)` in
 * templates.
 *
 * - **Typed keys.** The keys are derived from en.ts, the reference file:
 *   `t('typo')` does not type-check, and fr.ts must have exactly the same
 *   keys (its type is `Translation<typeof en>`; a test checks it too).
 * - **Plurals.** A message that depends on a count is an object with one
 *   string per CLDR plural category (`{ one: '{n} book', other: '{n} books' }`),
 *   chosen with `Intl.PluralRules` for the current language from the `n`
 *   parameter, which is required for those keys. French puts 0 in `one`.
 * - **Fallbacks.** A key missing in the current language uses English; a key
 *   missing everywhere (only possible with a cast) shows its last segment.
 * - **Category names** are stored in English (they are written into book
 *   files); `categoryLabel()` translates them. Only fr.ts has a
 *   `categories` section: in English the name is its own label.
 *
 * The current language is a module-level ref, so anything that calls `t()`
 * during a render (templates, computed properties) re-renders when it changes.
 */
import { ref } from 'vue'
import type { App } from 'vue'
import en from './en'
import fr from './fr'
import type { MessageTree, PluralCategory, PluralKeys, PluralMessage, TextKeys, TranslationParams } from './types'
import { saveConfig } from '@/services/appConfig'

export type { TranslationParams } from './types'

export type Lang = 'en' | 'fr'
export const LANGS: readonly Lang[] = ['en', 'fr']

/** A key of a plain-string message. */
export type TextKey = TextKeys<typeof en>
/** A key of a message that depends on a count (`n` is required). */
export type PluralKey = PluralKeys<typeof en>
/** Any key `t()` accepts. */
export type TranslationKey = TextKey | PluralKey

const messages: Record<Lang, MessageTree> = { en, fr }

export function isLang(value: unknown): value is Lang {
    return typeof value === 'string' && (LANGS as readonly string[]).includes(value)
}

// Default to the system language; App.vue applies the saved choice at startup.
const systemLang: Lang = navigator.language.startsWith('fr') ? 'fr' : 'en'
export const currentLang = ref<Lang>(systemLang)

/** Switches the UI language and saves the choice. */
export function setLang(lang: Lang): void {
    currentLang.value = lang
    void saveConfig({ lang })
}

function lookup(tree: MessageTree, key: string): string | PluralMessage | undefined {
    let cur: string | PluralMessage | MessageTree | undefined = tree
    for (const part of key.split('.')) {
        if (cur === undefined || typeof cur === 'string') { return undefined }
        cur = (cur as MessageTree)[part]
    }
    if (typeof cur === 'string') { return cur }
    return cur !== undefined && typeof cur.other === 'string' ? cur as PluralMessage : undefined
}

const pluralRules = new Map<Lang, Intl.PluralRules>()

/** The form of `message` for the count `n` in `lang` (`other` when that form is absent). */
export function pluralForm(message: PluralMessage, lang: Lang, n: number): string {
    let rules = pluralRules.get(lang)
    if (!rules) {
        rules = new Intl.PluralRules(lang)
        pluralRules.set(lang, rules)
    }
    const category: PluralCategory = rules.select(n)
    return message[category] ?? message.other
}

function interpolate(text: string, params?: TranslationParams): string {
    if (!params) { return text }
    return text.replace(/\{(\w+)\}/g, (match, name: string) => {
        const value = params[name]
        return value === undefined ? match : String(value)
    })
}

/**
 * The message `key` in the current language, with `{name}` placeholders
 * replaced by `params`. For plural keys, `params.n` picks the form.
 */
export function t(key: PluralKey, params: TranslationParams & { n: number }): string
export function t(key: TextKey, params?: TranslationParams): string
export function t(key: TranslationKey, params?: TranslationParams): string {
    // Reading currentLang.value makes this reactive — Vue tracks it during render.
    const lang = currentLang.value
    const message = lookup(messages[lang], key) ?? lookup(messages.en, key)
    let text: string
    if (message === undefined) {
        text = key.split('.').pop() ?? key
    } else if (typeof message === 'string') {
        text = message
    } else {
        const n = typeof params?.n === 'number' ? params.n : Number(params?.n)
        text = Number.isFinite(n) ? pluralForm(message, lang, n) : message.other
    }
    return interpolate(text, params)
}

const categoryNames: Record<Lang, Record<string, string>> = { en: {}, fr: fr.categories }

/** The label of a category (stored in English, e.g. "Science Fiction") in the current language. */
export function categoryLabel(category: string): string {
    return categoryNames[currentLang.value][category] ?? category
}

export const i18nPlugin = {
    install(app: App) {
        app.config.globalProperties.$t = t
    },
}

declare module 'vue' {
    interface ComponentCustomProperties {
        $t: typeof t
    }
}
