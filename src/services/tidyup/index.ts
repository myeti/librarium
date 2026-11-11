import * as api from '@/services/api'
import type { QueryParams, TidyupOptions, TidyupOutcome, TidyupProvider, TidyupResult, TidyupQueryResult } from './types'
import { googleBooksProvider } from './providers/googleBooks'
import { openLibraryProvider } from './providers/openLibrary'
import { stripUndefined, titleSimilarity, MIN_TITLE_SIMILARITY } from './merge'
import { analytics } from '@/services/analytics'
import { isbnLookupForms } from '@/utils/isbn'
import { t } from '@/i18n'

export type { TidyupOptions, TidyupOutcome, TidyupResult }

/** Priority-ordered list of providers. The first confident result wins. */
const PROVIDERS: TidyupProvider[] = [
    googleBooksProvider,
    openLibraryProvider,
]

/** Ids of the providers tidy-up will query, in order (e.g. for the confirm dialog). */
export function availableTidyupSources(): string[] {
    return PROVIDERS.filter(p => p.isAvailable()).map(p => p.id)
}

/** The display name of a provider id ("Google Books"); an unknown id as is. */
export function tidyupSourceLabel(id: string | null | undefined): string {
    if (id === 'googlebooks' || id === 'openlibrary') { return t(`tidyup.sources.${id}`) }
    return id ?? ''
}

interface BookToLookUp {
    title:  string
    author: string
    isbn?:  string
}

/**
 * Asks one provider about a book. A book with a usable ISBN is looked up by
 * ISBN first; when that finds nothing (an ISBN the catalogue doesn't know, a
 * typo), the provider is asked again by title and author. Rejects when the
 * provider couldn't answer (timeout, HTTP error…), resolves with null when
 * it answered but doesn't know the book.
 */
async function askProvider(
    provider: TidyupProvider,
    book: BookToLookUp,
    language: () => Promise<string | undefined>,
): Promise<TidyupQueryResult | null> {
    if (isbnLookupForms(book.isbn).length > 0) {
        const params: QueryParams = { isbn: book.isbn }
        console.log(`[tidyup] querying ${provider.id}`, params)
        const byIsbn = await provider.query(params)
        if (byIsbn) { return byIsbn }
    }
    if (!book.title && !book.author) { return null }
    const params: QueryParams = { title: book.title, author: book.author, lang: await language() }
    console.log(`[tidyup] querying ${provider.id}`, params)
    return provider.query(params)
}

/**
 * Looks a book up on the metadata providers.
 *
 * Providers are tried in order. A result whose title closely matches the
 * book's is returned at once; a doubtful one is remembered and the next
 * provider is tried, since another catalogue may know the book better. If no
 * provider has a confident match, the best doubtful one is returned with
 * `confident: false`, unless `requireConfidentMatch` is set, in which case
 * the outcome is `'low-confidence'` and nothing is returned.
 *
 * The outcome tells "nobody could be asked" (`unreachable`: every provider
 * failed — offline, timeouts, rate limits after retries) apart from "nobody
 * knows this book" (`no-match`), so the UI can say which.
 */
export async function tidyupBook(current: BookToLookUp, options: TidyupOptions = {}): Promise<TidyupResult> {
    // The book's language narrows title searches; it is detected once, and
    // only if a title search is needed.
    let detected: Promise<string | undefined> | null = null
    const language = () => {
        detected ??= api.detectLanguage(`${current.title} ${current.author}`)
            .then(lang => lang ?? undefined, () => undefined)
        return detected
    }

    let queried = 0
    let failed  = 0
    let best: { source: string, result: TidyupQueryResult, similarity: number } | null = null

    for (const provider of PROVIDERS) {
        if (!provider.isAvailable()) {continue}
        queried++
        try {
            const result = await askProvider(provider, current, language)
            console.log(`[tidyup] ${provider.id} result:`, result ?? '(empty)')
            if (!result) {continue}
            const similarity = titleSimilarity(current.title, result.metadata.title)
            if (!best || similarity > best.similarity) {
                best = { source: provider.id, result, similarity }
            }
            if (similarity >= MIN_TITLE_SIMILARITY) {break}
        } catch (e) {
            failed++
            console.error(`[tidyup] ${provider.id} error:`, e)
            analytics.track('error', { component: 'tidy_up', error_type: 'provider_failed' })
        }
    }

    if (best) {
        const confident = best.similarity >= MIN_TITLE_SIMILARITY
        if (confident || !options.requireConfidentMatch) {
            analytics.track('tidy_up_search', { service: best.source, found: true, confident })
            return {
                found:      true,
                outcome:    'found',
                source:     best.source,
                metadata:   stripUndefined(best.result.metadata),
                coverUrl:   best.result.coverUrl ?? null,
                similarity: best.similarity,
                confident,
            }
        }
        analytics.track('tidy_up_search', { service: best.source, found: false, confident: false })
        return {
            found: false, outcome: 'low-confidence', source: best.source, metadata: {}, coverUrl: null,
            similarity: best.similarity, confident: false,
        }
    }

    // Every provider that ran threw, so we never actually got an answer. The
    // caller reports that differently from "we asked and nobody knew the book".
    const outcome: TidyupOutcome = queried > 0 && failed === queried ? 'unreachable' : 'no-match'
    analytics.track('tidy_up_search', { service: 'all', found: false })
    return { found: false, outcome, source: null, metadata: {}, coverUrl: null, similarity: 0, confident: false }
}
