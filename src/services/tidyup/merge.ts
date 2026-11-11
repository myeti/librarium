/**
 * Pure helpers that decide what a tidy-up result may change on a book.
 *
 * Batch tidy-up applies results without a per-book review, so it has to be
 * conservative: it only fills fields that are empty, never overwrites a value
 * the user already has, and only trusts a result whose title actually looks
 * like the book's. Single-book tidy-up shows a review dialog instead, but
 * uses the same similarity measure to warn about doubtful matches.
 */
import type { BookMetadata } from '@/types'
import { matchTokens } from '@/utils/text'

/**
 * Minimum {@link titleSimilarity} for a match to be applied automatically.
 * 0.6 accepts "Harry Potter and the Philosopher's Stone" for "Harry Potter
 * and the Sorcerer's Stone" (3 of 4 significant words) but rejects "Dune
 * Messiah" for "Dune" (1 of 2).
 */
export const MIN_TITLE_SIMILARITY = 0.6

/**
 * Returns a copy without the keys whose value is `undefined`.
 *
 * Providers build results like `{ description: data.description }`, which
 * yields an explicit `undefined` when the field is missing. Spreading such an
 * object over a book's metadata would erase the book's own value, and the
 * backend treats an absent field in a full metadata update as "clear it".
 */
export function stripUndefined<T extends object>(obj: T): Partial<T> {
    const out: Partial<T> = {}
    for (const [key, value] of Object.entries(obj)) {
        if (value !== undefined) { (out as Record<string, unknown>)[key] = value }
    }
    return out
}

/** Placeholders the backend or other tools write when a value is unknown. */
const PLACEHOLDERS = new Set(['unknown', 'unknown title', 'unknown author', 'und', 'n/a'])

/**
 * Whether a metadata value counts as "not filled in". Empty strings, empty
 * lists and placeholders such as "Unknown author" or the language code "und"
 * (undetermined) are all empty, so tidy-up may replace them.
 */
export function isEmptyValue(value: unknown): boolean {
    if (value === undefined || value === null) { return true }
    if (typeof value === 'string') {
        const v = value.trim()
        return v === '' || PLACEHOLDERS.has(v.toLowerCase())
    }
    if (Array.isArray(value)) { return value.length === 0 }
    return false
}

/**
 * The fields of `incoming` that may be written on a book in batch mode: only
 * those that are empty on the book and non-empty in the result. Returns an
 * empty object when there is nothing to fill.
 */
export function fillEmptyFields(
    current: BookMetadata,
    incoming: Partial<BookMetadata>,
): Partial<BookMetadata> {
    const patch: Partial<BookMetadata> = {}
    for (const [key, value] of Object.entries(stripUndefined(incoming))) {
        const k = key as keyof BookMetadata
        if (isEmptyValue(value)) { continue }
        if (!isEmptyValue(current[k])) { continue }
        (patch as Record<string, unknown>)[k] = value
    }
    return patch
}

/**
 * Small words that carry no identity ("The Hobbit" and "Hobbit" are the same
 * book). English and French, the two UI languages.
 */
const STOPWORDS = new Set([
    'the', 'a', 'an', 'and', 'of', 'or',
    'le', 'la', 'les', 'un', 'une', 'des', 'de', 'du', 'et', 'ou',
])

function significantTokens(title: string): Set<string> {
    // Single letters are dropped too: they come from elisions and
    // possessives once punctuation is removed ("l'étranger" → "l etranger",
    // "Philosopher's" → "philosopher s").
    const all = matchTokens(title)
    const significant = all.filter(t => !STOPWORDS.has(t) && (t.length > 1 || /\d/.test(t)))
    // A title made only of small words ("Le Un") still needs to compare.
    return new Set(significant.length > 0 ? significant : all)
}

/**
 * The part before a subtitle separator: "Sapiens: A Brief History…" →
 * "Sapiens", "The Hobbit, or There and Back Again" → "The Hobbit". A bare
 * comma is not a separator: "Love, Rosie" and "Love, Actually" must not
 * both shrink to "Love".
 */
function mainTitle(title: string): string {
    return title.split(/\s*(?::|\s[-–—]\s|\(|\[|,\s+(?:or|ou)\s)/i)[0] ?? title
}

function overlap(a: Set<string>, b: Set<string>): number {
    if (a.size === 0 || b.size === 0) { return 0 }
    let shared = 0
    for (const t of a) { if (b.has(t)) { shared++ } }
    return shared / Math.max(a.size, b.size)
}

/**
 * How alike two titles are, from 0 (nothing in common) to 1 (same words).
 *
 * Titles are compared word by word after normalisation (case, accents and
 * punctuation ignored, small words dropped). The score is the share of words
 * in common relative to the longer title, so an extra word counts against
 * the match. Because providers often add or drop a subtitle, the main titles
 * (before ":", " - " or "(") are compared too and the best score wins.
 */
export function titleSimilarity(a: string | null | undefined, b: string | null | undefined): number {
    if (!a || !b) { return 0 }
    const variantsA = [a, mainTitle(a)]
    const variantsB = [b, mainTitle(b)]
    let best = 0
    for (const va of variantsA) {
        for (const vb of variantsB) {
            best = Math.max(best, overlap(significantTokens(va), significantTokens(vb)))
        }
    }
    return best
}

/** Whether a provider's title is close enough to the book's to apply it unreviewed. */
export function isConfidentMatch(bookTitle: string, resultTitle: string | undefined): boolean {
    return titleSimilarity(bookTitle, resultTitle) >= MIN_TITLE_SIMILARITY
}

/**
 * Open Library's cover API returns a grey "no cover" placeholder (HTTP 200)
 * when it has no image, which would replace a real cover with a blank one.
 * `?default=false` makes it answer 404 instead, which the backend rejects.
 */
export function withoutPlaceholderCover(url: string): string {
    try {
        const u = new URL(url)
        if (u.hostname === 'covers.openlibrary.org') { u.searchParams.set('default', 'false') }
        return u.toString()
    } catch {
        return url
    }
}
