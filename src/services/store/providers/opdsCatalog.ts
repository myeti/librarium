import type { StoreProvider, StoreBook, StoreFormat } from '../types'
import type { OpdsCatalogSource } from '@/stores/opdsSources'
import { httpGetJsonAt } from '@/services/http'

/** The parts of an OPDS 2.0 feed this provider reads. Every field is optional: catalogues vary. */
interface OpdsFeed {
    publications?: OpdsPublication[]
}

interface OpdsPublication {
    metadata?: {
        identifier?: string
        title?: string
        author?: unknown
        contributor?: unknown
        language?: string
    }
    links?: OpdsLink[]
    /** OPDS 2.0 puts covers here (`links` is for acquisitions); older feeds mix them in `links`. */
    images?: OpdsLink[]
}

interface OpdsLink {
    href?: string
    type?: string
    rel?: string | string[]
}

function hostnameFromUrl(url: string): string {
    try { return new URL(url).hostname } catch { return url }
}

/** A link's `rel`, which OPDS allows as one string or a list, as one string to search. */
function relOf(link: OpdsLink): string {
    return Array.isArray(link.rel) ? link.rel.join(' ') : link.rel ?? ''
}

/**
 * The library format of each acquisition media type the backend can import
 * (`download_and_import` checks the bytes against the extension, so the
 * extension must be the right one). Types are compared without parameters
 * (`application/epub+zip; charset=…`) and case.
 */
const FORMAT_BY_TYPE: Record<string, StoreFormat> = {
    'application/epub+zip': 'epub',
    'application/pdf': 'pdf',
    'application/x-mobipocket-ebook': 'mobi',
    'application/vnd.amazon.ebook': 'azw3',
    'application/vnd.amazon.mobi8-ebook': 'azw3',
    'application/x-mobi8-ebook': 'azw3',
    'application/vnd.comicbook+zip': 'cbz',
    'application/x-cbz': 'cbz',
}

/** Formats by file extension, for acquisition links that don't say their type. */
const FORMAT_BY_EXTENSION: Record<string, StoreFormat> = {
    epub: 'epub', pdf: 'pdf', mobi: 'mobi', azw3: 'azw3', cbz: 'cbz',
}

/** Which copy to download when a publication offers several: EPUB first (the reader and every device take it). */
const FORMAT_PREFERENCE: StoreFormat[] = ['epub', 'azw3', 'mobi', 'pdf', 'cbz']

/**
 * The format a link downloads, from its media type — or, when it has none,
 * from the extension of its URL. `undefined` for anything Librarium can't
 * import (HTML pages, FB2, audiobooks…): such a link is never offered.
 */
export function formatOfLink(link: OpdsLink, href: string): StoreFormat | undefined {
    if (link.type) {
        const type = link.type.split(';')[0].trim().toLowerCase()
        return FORMAT_BY_TYPE[type]
    }
    try {
        const extension = new URL(href).pathname.split('.').pop()?.toLowerCase() ?? ''
        return FORMAT_BY_EXTENSION[extension]
    } catch {
        return undefined
    }
}

/**
 * `href` made absolute against the feed it came from. OPDS links are often
 * relative (`/books/12.epub`, `covers/12.jpg`) and the backend only downloads
 * absolute http(s) URLs. `undefined` when the result isn't http(s) or the
 * link is malformed.
 */
export function resolveHref(href: string | undefined, feedUrl: string): string | undefined {
    if (!href) { return undefined }
    try {
        const url = new URL(href, feedUrl)
        return url.protocol === 'http:' || url.protocol === 'https:' ? url.toString() : undefined
    } catch {
        return undefined
    }
}

/** An acquisition link of a publication, resolved, with the format it downloads. */
interface Acquisition {
    url: string
    format: StoreFormat
}

/**
 * The copy of a publication to download: among its acquisition links (and
 * links typed as an EPUB, which some catalogues don't mark), the ones in a
 * format Librarium imports, the most preferred format first.
 */
export function pickAcquisition(links: OpdsLink[], feedUrl: string): Acquisition | undefined {
    const candidates: Acquisition[] = []
    for (const link of links) {
        const url = resolveHref(link.href, feedUrl)
        if (!url) { continue }
        const format = formatOfLink(link, url)
        if (!format) { continue }
        const rel = relOf(link)
        const isAcquisition = rel.includes('acquisition') || rel.includes('open-access')
        if (isAcquisition || format === 'epub') { candidates.push({ url, format }) }
    }
    const rank = (a: Acquisition) => FORMAT_PREFERENCE.indexOf(a.format)
    // A stable sort: links of the same format keep the feed's order.
    return candidates.sort((a, b) => rank(a) - rank(b))[0]
}

/** OPDS contributors are a name, a `{ name }` object, or a list of either. */
export function resolveAuthor(raw: unknown): string {
    const nameOf = (value: unknown): string | undefined => {
        if (typeof value === 'string') { return value }
        if (value && typeof value === 'object' && 'name' in value && typeof value.name === 'string') { return value.name }
        return undefined
    }
    // `||`, not `??`: an empty name is as unknown as a missing one. Unknown is '':
    // the view shows a translated placeholder.
    if (Array.isArray(raw)) { return nameOf(raw[0]) || '' }
    return nameOf(raw) || ''
}

/** A Discover provider for an OPDS 2.0 catalogue the user added in Settings › Catalogs. */
export function createOpdsCatalogProvider(source: OpdsCatalogSource): StoreProvider {
    const label = source.name.trim() || hostnameFromUrl(source.url)
    const providerId = `opds-${source.id}`

    return {
        id: providerId,
        label,

        isAvailable(): boolean {
            return !!source.url
        },

        async search(query: string, signal?: AbortSignal): Promise<StoreBook[]> {
            const base = new URL(source.url)
            base.searchParams.set('q', query)

            const headers: Record<string, string> = {
                Accept: 'application/opds+json, application/json',
            }
            if (source.apiKey) {
                headers['Authorization'] = `Bearer ${source.apiKey}`
            }

            // User-added catalogues can be on any host, which the webview CSP
            // can't allow in advance, so the request goes through the backend.
            // Failures propagate: the Store view shows "catalogue unreachable"
            // instead of pretending the catalogue had no results.
            const { data, url: feedUrl } = await httpGetJsonAt<OpdsFeed>(base.toString(), { signal, headers })

            const books: StoreBook[] = []
            for (const pub of data.publications ?? []) {
                const links = pub.links ?? []
                const acquisition = pickAcquisition(links, feedUrl)
                if (!acquisition) { continue }

                const images = [...(pub.images ?? []), ...links]
                const coverLink =
                    images.find(l => relOf(l).includes('opds-spec.org/image')) ??
                    images.find(l => (l.type ?? '').startsWith('image/'))

                const meta = pub.metadata ?? {}

                books.push({
                    id:          `${providerId}-${meta.identifier ?? meta.title ?? Math.random()}`,
                    source:      providerId,
                    title:       meta.title ?? '',
                    author:      resolveAuthor(meta.author ?? meta.contributor),
                    language:    meta.language,
                    coverUrl:    resolveHref(coverLink?.href, feedUrl),
                    downloadUrl: acquisition.url,
                    format:      acquisition.format,
                })
            }

            return books
        },
    }
}
