import type { StoreProvider, StoreBook } from '../types'
import { httpGetJson } from '@/services/http'

const OL_SEARCH  = 'https://openlibrary.org/search.json'
const OL_COVERS  = 'https://covers.openlibrary.org/b/id'
const IA_META    = 'https://archive.org/metadata'
const IA_DL      = 'https://archive.org/download'

/** The parts of an Open Library search answer this provider reads. */
interface OlSearchResponse {
    docs?: OlDoc[]
}

interface OlDoc {
    key: string
    title?: string
    author_name?: string[]
    language?: string | string[]
    cover_i?: number
    /** Internet Archive identifier(s) of the scans. */
    ia?: string | string[]
    subject?: string[]
}

/** The parts of an Internet Archive `metadata` answer this provider reads. */
interface IaMetadata {
    metadata?: Record<string, unknown>
    files?: { name: string, format?: string }[]
}

function first<T>(value: T | T[] | undefined): T | undefined {
    return Array.isArray(value) ? value[0] : value
}

/**
 * Lists an Internet Archive item's files. Goes through the backend
 * (`http_get`): archive.org isn't in the webview CSP's `connect-src`, so with
 * `fetch` every lookup was blocked in release builds and the Open Library
 * store always showed 0 results. Routing the few third-party hosts we need
 * through Rust keeps the CSP short instead of widening it.
 */
async function fetchIaFormats(
    iaId: string,
    signal?: AbortSignal,
): Promise<{ epub?: string; pdf?: string }> {
    const data = await httpGetJson<IaMetadata>(`${IA_META}/${encodeURIComponent(iaId)}`, { signal })

    // Skip items that require an Internet Archive account (CDL / copyright lending).
    if (data.metadata?.['access-restricted-item'] === 'true') {return {}}

    const files = data.files ?? []
    const epubFile = files.find(f => f.format?.toUpperCase() === 'EPUB')
    const pdfFile  = files.find(f => f.format?.toUpperCase() === 'PDF')

    return {
        epub: epubFile ? `${IA_DL}/${iaId}/${epubFile.name}` : undefined,
        pdf:  pdfFile  ? `${IA_DL}/${iaId}/${pdfFile.name}`  : undefined,
    }
}

/** Open Library books with a free full text on the Internet Archive (EPUB, else PDF). */
export const openLibraryStoreProvider: StoreProvider = {
    id: 'openlibrary',
    label: 'Open Library',

    isAvailable(): boolean {
        return true
    },

    async search(query: string, signal?: AbortSignal): Promise<StoreBook[]> {
        const params = new URLSearchParams({
            q:            query,
            has_fulltext: 'true',
            fields:       'key,title,author_name,language,cover_i,ia,subject',
            limit:        '10',
        })
        const res = await fetch(`${OL_SEARCH}?${params.toString()}`, { signal })
        // An error, not "no results": the Store view shows the catalogue as unreachable.
        if (!res.ok) {throw new Error(`Open Library answered HTTP ${res.status}`)}
        const data = await res.json() as OlSearchResponse

        // Keep only docs that have an Internet Archive identifier.
        const candidates = (data.docs ?? [])
            .map(doc => ({ doc, iaId: first(doc.ia) }))
            .filter((c): c is { doc: OlDoc, iaId: string } => Boolean(c.iaId))

        // Resolve IA file listings in parallel — one request per candidate.
        const settled = await Promise.allSettled(
            candidates.map(async ({ doc, iaId }) => ({ doc, files: await fetchIaFormats(iaId, signal) })),
        )

        // If every archive.org lookup failed, the catalogue is effectively
        // unreachable; saying "no results" would hide the problem.
        const firstFailure = settled.find((r): r is PromiseRejectedResult => r.status === 'rejected')
        if (candidates.length > 0 && firstFailure && settled.every(r => r.status === 'rejected')) {
            // Rethrown as it came (an HttpStatusError, a backend message…) for the Store view to show.
            throw firstFailure.reason as Error
        }

        const books: StoreBook[] = []
        for (const result of settled) {
            if (result.status !== 'fulfilled') {continue}
            const { doc, files } = result.value

            const downloadUrl = files.epub ?? files.pdf
            if (!downloadUrl) {continue}

            books.push({
                id:          `openlibrary-${doc.key}`,
                source:      'openlibrary',
                title:       doc.title ?? '',
                author:      doc.author_name?.[0] ?? '',
                language:    first(doc.language),
                subjects:    doc.subject?.slice(0, 5),
                // ?default=false: a missing cover is a 404 (the card shows its
                // initials) rather than a grey "no image" picture.
                coverUrl:    doc.cover_i ? `${OL_COVERS}/${doc.cover_i}-M.jpg?default=false` : undefined,
                downloadUrl,
                format:      files.epub ? 'epub' : 'pdf',
                sourceUrl:   `https://openlibrary.org${doc.key}`,
            })
        }

        return books
    },
}
