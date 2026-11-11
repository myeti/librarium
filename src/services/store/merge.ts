import { titleAuthorKey } from '@/services/duplicates'
import { compareText } from '@/utils/collator'
import { normalizeForMatch } from '@/utils/text'
import type { StoreBook } from './types'

/**
 * Discover asks several catalogs at once; their answers arrive in any
 * order and often hold the same book (Project Gutenberg's EPUB is on Open
 * Library too). `mergeResults` turns them into one list:
 *
 * - **one entry per book**: same title and author (case, accents and
 *   punctuation ignored; books without a title are never merged), keeping
 *   the better copy: an EPUB over a PDF, then one with a cover, then the
 *   catalog listed first;
 * - **a stable order**, whatever answered first: best match for the search
 *   first (the title is the search, starts with it, contains it; then the
 *   author does), then by title in the UI language.
 */
export function mergeResults(results: readonly StoreBook[], query: string, providerOrder: readonly string[] = []): StoreBook[] {
    const rankOfProvider = (source: string) => {
        const i = providerOrder.indexOf(source)
        return i === -1 ? providerOrder.length : i
    }
    const kept = new Map<string, StoreBook>()
    for (const book of results) {
        const key = titleAuthorKey(book.title, book.author) ?? `id:${book.id}`
        const current = kept.get(key)
        if (!current || isBetterCopy(book, current, rankOfProvider)) { kept.set(key, book) }
    }
    const q = normalizeForMatch(query)
    return [...kept.values()].sort((a, b) =>
        relevance(a, q) - relevance(b, q) || compareText(a.title, b.title) || compareText(a.author, b.author),
    )
}

function isBetterCopy(candidate: StoreBook, current: StoreBook, rankOfProvider: (source: string) => number): boolean {
    if (candidate.format !== current.format) { return candidate.format === 'epub' }
    if (!!candidate.coverUrl !== !!current.coverUrl) { return !!candidate.coverUrl }
    return rankOfProvider(candidate.source) < rankOfProvider(current.source)
}

/** 0 is the best match: the title is the search, starts with it, contains it; then the author. */
export function relevance(book: StoreBook, normalizedQuery: string): number {
    if (!normalizedQuery) { return 0 }
    const title = normalizeForMatch(book.title)
    const author = normalizeForMatch(book.author)
    if (title === normalizedQuery) { return 0 }
    if (title.startsWith(normalizedQuery)) { return 1 }
    if (title.includes(normalizedQuery)) { return 2 }
    if (author.includes(normalizedQuery)) { return 3 }
    return 4
}
