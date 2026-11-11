/**
 * How text is ordered for people: one shared `Intl.Collator` per UI language.
 *
 * - locale-aware: accented letters sort with their base letter ("Élise"
 *   before "Zola"; a plain `<` puts every "É" after "Z");
 * - numeric: digit runs compare as numbers ("Tome 2" before "Tome 10");
 * - case- and accent-insensitive (`sensitivity: 'base'`): "dune" and "Dune"
 *   are equal, so the sort keeps their existing order (Array.prototype.sort
 *   is stable).
 *
 * Collators are expensive to create and cheap to use, hence the cache.
 */
import { currentLang, type Lang } from '@/i18n'

const collators = new Map<Lang, Intl.Collator>()

export function textCollator(lang: Lang = currentLang.value): Intl.Collator {
    let collator = collators.get(lang)
    if (!collator) {
        collator = new Intl.Collator(lang, { numeric: true, sensitivity: 'base', usage: 'sort' })
        collators.set(lang, collator)
    }
    return collator
}

/** Compares two strings for display order (see the module comment). */
export function compareText(a: string, b: string, lang: Lang = currentLang.value): number {
    return textCollator(lang).compare(a, b)
}

/** A sorted copy of `values`, in display order. */
export function sortedText(values: Iterable<string>, lang: Lang = currentLang.value): string[] {
    const collator = textCollator(lang)
    return [...values].sort((a, b) => collator.compare(a, b))
}
