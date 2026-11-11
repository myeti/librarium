import type { TidyupProvider, TidyupQueryResult, QueryParams } from '../types'
import { detectCategory } from '@/constants/categories'
import { useSettingsStore } from '@/stores/settings'
import { stripHtml } from '../utils'
import { stripUndefined } from '../merge'
import { fetchJson } from '../fetchJson'
import { redactSecrets } from '@/utils/redact'
import { isbnLookupForms } from '@/utils/isbn'

/** The parts of a Google Books `volumes` answer this provider reads. */
interface VolumesResponse {
    items?: { volumeInfo?: VolumeInfo }[]
}

interface VolumeInfo {
    title?: string
    subtitle?: string
    authors?: string[]
    language?: string
    description?: string
    categories?: string[]
    industryIdentifiers?: { type: string, identifier: string }[]
    imageLinks?: { thumbnail?: string }
}

/**
 * A value inside a `intitle:"…"` / `inauthor:"…"` phrase: double quotes
 * would end the phrase early, so they become spaces.
 */
function phrase(value: string): string {
    return `"${value.replace(/"/g, ' ').replace(/\s+/g, ' ').trim()}"`
}

/**
 * The search for these parameters: `isbn:<n>` when an ISBN is given (the
 * ISBN-13 form; Google indexes both), else the title and author as quoted
 * phrases. Quoting matters: without it, `intitle:The Hobbit` only requires
 * "The" in the title and searches "Hobbit" anywhere.
 */
export function googleBooksQuery({ isbn, title, author }: QueryParams): string | null {
    if (isbn) {
        const [best] = isbnLookupForms(isbn)
        return best ? `isbn:${best}` : null
    }
    const parts: string[] = []
    if (title)  {parts.push(`intitle:${phrase(title)}`)}
    if (author) {parts.push(`inauthor:${phrase(author)}`)}
    return parts.length > 0 ? parts.join(' ') : null
}

async function query(apiKey: string, params: QueryParams): Promise<TidyupQueryResult | null> {
    const q = googleBooksQuery(params)
    if (!q) { return null }

    const search = new URLSearchParams({ q, maxResults: '1', key: apiKey })
    if (params.lang && !params.isbn) {search.set('langRestrict', params.lang)}

    const url = `https://www.googleapis.com/books/v1/volumes?${search.toString()}`
    // The URL carries the user's API key: never log it as is.
    console.log('[tidyup:googlebooks] query', redactSecrets(url))
    // A failing request (bad key, quota, outage) rejects, which is not "no
    // match": the caller tells the user the service is unreachable.
    const data = await fetchJson<VolumesResponse>(url, { label: 'Google Books' })

    const volume = data.items?.[0]?.volumeInfo
    if (!volume) {return null}

    const idents = volume.industryIdentifiers ?? []
    const foundIsbn = idents.find(i => i.type === 'ISBN_13')?.identifier
        ?? idents.find(i => i.type === 'ISBN_10')?.identifier

    const detectedCat = detectCategory(volume.categories ?? [])

    const rawThumbnail = volume.imageLinks?.thumbnail
    const coverUrl = rawThumbnail
        ? rawThumbnail.replace('zoom=1', 'zoom=0').replace(/^http:\/\//, 'https://')
        : null

    return {
        // Missing fields are left out rather than set to undefined, so the
        // result can't erase a book's own values when merged.
        metadata: stripUndefined({
            title:       volume.title,
            subtitle:    volume.subtitle,
            author:      volume.authors?.[0],
            language:    volume.language,
            description: volume.description ? stripHtml(volume.description) : undefined,
            isbn:        foundIsbn,
            categories:  detectedCat ? [detectedCat] : undefined,
        }),
        coverUrl,
    }
}

export const googleBooksProvider: TidyupProvider = {
    id: 'googlebooks',

    isAvailable(): boolean {
        return !!useSettingsStore().googleBooksApiKey
    },

    query(params: QueryParams): Promise<TidyupQueryResult | null> {
        return query(useSettingsStore().googleBooksApiKey, params)
    },
}
