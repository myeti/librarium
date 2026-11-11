import { afterEach, describe, expect, it } from 'vitest'
import en from './en'
import fr from './fr'
import { categoryLabel, currentLang, pluralForm, t } from './index'
import type { MessageTree, PluralMessage } from './types'

/**
 * Sections that only French has, on purpose (see the comment in fr.ts):
 * category names are stored in English, so English needs no translation.
 */
const FR_ONLY_SECTIONS = ['categories']

function isPlural(value: unknown): value is PluralMessage {
    return typeof value === 'object' && value !== null && typeof (value as { other?: unknown }).other === 'string'
}

/** Every message of a tree, by dotted key (plural messages as one entry). */
function messagesOf(tree: MessageTree, prefix = ''): Map<string, string | PluralMessage> {
    const out = new Map<string, string | PluralMessage>()
    for (const [key, value] of Object.entries(tree)) {
        const path = prefix ? `${prefix}.${key}` : key
        if (typeof value === 'string' || isPlural(value)) {
            out.set(path, value)
        } else {
            for (const [k, v] of messagesOf(value, path)) { out.set(k, v) }
        }
    }
    return out
}

/** The `{name}` placeholders of a message (all forms of a plural). */
function placeholders(message: string | PluralMessage): string[] {
    const texts = typeof message === 'string' ? [message] : Object.values(message).filter((s): s is string => !!s)
    return [...new Set(texts.flatMap(s => [...s.matchAll(/\{(\w+)\}/g)].map(m => m[1])))].sort()
}

const enMessages = messagesOf(en)
const frMessages = new Map([...messagesOf(fr)].filter(([key]) => !FR_ONLY_SECTIONS.includes(key.split('.')[0])))

afterEach(() => { currentLang.value = 'en' })

describe('translation files', () => {
    it('have exactly the same keys in English and French (fr-only sections aside)', () => {
        const missingInFr = [...enMessages.keys()].filter(k => !frMessages.has(k))
        const extraInFr = [...frMessages.keys()].filter(k => !enMessages.has(k))
        expect(missingInFr).toEqual([])
        expect(extraInFr).toEqual([])
    })

    it('use plural forms for the same keys, with `other` always present', () => {
        for (const [key, message] of enMessages) {
            const french = frMessages.get(key)
            expect(isPlural(french), key).toBe(isPlural(message))
        }
    })

    it('use the same placeholders in both languages', () => {
        for (const [key, message] of enMessages) {
            expect(placeholders(frMessages.get(key)!), key).toEqual(placeholders(message))
        }
    })

    it('every plural message counts with {n}', () => {
        for (const [key, message] of enMessages) {
            if (isPlural(message)) { expect(placeholders(message), key).toContain('n') }
        }
    })

    it('never use "(s)" plural hacks', () => {
        const all = [...enMessages.values(), ...frMessages.values()]
            .flatMap(m => typeof m === 'string' ? [m] : Object.values(m))
        expect(all.filter(s => s?.includes('(s)'))).toEqual([])
    })
})

describe('t()', () => {
    it('replaces placeholders and leaves unknown ones visible', () => {
        expect(t('detail.sendTo', { device: 'Kindle' })).toBe('Send to Kindle')
        expect(t('detail.sendTo')).toBe('Send to {device}')
    })

    it('picks the plural form for n with the language’s rules', () => {
        expect(t('library.trashConfirmTitle', { n: 1 })).toBe('Move 1 book to the Trash?')
        expect(t('library.trashConfirmTitle', { n: 3 })).toBe('Move 3 books to the Trash?')
        expect(t('library.trashConfirmTitle', { n: 0 })).toBe('Move 0 books to the Trash?')

        currentLang.value = 'fr'
        // French counts 0 and 1 as singular.
        expect(t('library.trashConfirmTitle', { n: 0 })).toBe('Mettre 0 livre à la corbeille ?')
        expect(t('library.trashConfirmTitle', { n: 1 })).toBe('Mettre 1 livre à la corbeille ?')
        expect(t('library.trashConfirmTitle', { n: 2 })).toBe('Mettre 2 livres à la corbeille ?')
    })

    it('falls back to `other` for a category a language has no form for', () => {
        const message = { one: 'one', other: 'other' }
        // French uses "many" for a million: not spelled out here, so "other".
        expect(pluralForm(message, 'fr', 1_000_000)).toBe('other')
        expect(pluralForm(message, 'en', 1)).toBe('one')
    })

    it('follows the current language', () => {
        expect(t('common.cancel')).toBe('Cancel')
        currentLang.value = 'fr'
        expect(t('common.cancel')).toBe('Annuler')
    })

    it('shows the last segment of a key that exists nowhere', () => {
        // @ts-expect-error — not a key: rejected at compile time, tolerated at run time.
        expect(t('nowhere.toBeFound')).toBe('toBeFound')
    })

    it('rejects typos and plural keys without a count at compile time', () => {
        // These lines are checked by vue-tsc, not run.
        const check = () => {
            // @ts-expect-error — typo
            t('common.cancle')
            // @ts-expect-error — a plural message needs `n`
            t('library.trashDone')
        }
        expect(typeof check).toBe('function')
    })
})

describe('categoryLabel()', () => {
    it('shows the English name in English and the French one in French', () => {
        expect(categoryLabel('Science Fiction')).toBe('Science Fiction')
        currentLang.value = 'fr'
        expect(categoryLabel('Science Fiction')).toBe('Science-Fiction')
        // A category from a provider with no translation keeps its name.
        expect(categoryLabel('Steampunk')).toBe('Steampunk')
    })
})
