import type { StoreProvider, StoreBook } from '../types'
import { useSettingsStore } from '@/stores/settings'

/** The parts of a Google Books `volumes` answer this provider reads. */
interface VolumesResponse {
    items?: {
        id: string
        volumeInfo?: {
            title?: string
            authors?: string[]
            language?: string
            categories?: string[]
            imageLinks?: { thumbnail?: string }
            infoLink?: string
        }
        accessInfo?: {
            epub?: { downloadLink?: string }
            pdf?: { downloadLink?: string }
        }
    }[]
}

/** Free Google Books ebooks with a direct download (needs the user's API key). */
export const googleBooksStoreProvider: StoreProvider = {
    id: 'googlebooks',
    label: 'Google Books',

    isAvailable(): boolean {
        return !!useSettingsStore().googleBooksApiKey
    },

    async search(query: string, signal?: AbortSignal): Promise<StoreBook[]> {
        const apiKey = useSettingsStore().googleBooksApiKey
        const params = new URLSearchParams({
            q:          query,
            filter:     'free-ebooks',
            maxResults: '20',
            key:        apiKey,
        })
        const res = await fetch(`https://www.googleapis.com/books/v1/volumes?${params.toString()}`, { signal })
        if (!res.ok) {throw new Error(`Google Books answered HTTP ${res.status}`)}
        const data = await res.json() as VolumesResponse

        const books: StoreBook[] = []
        for (const item of data.items ?? []) {
            const info   = item.volumeInfo ?? {}
            const access = item.accessInfo

            // Only include books with a real direct download link (not ACS-token-gated).
            const epubLink = access?.epub?.downloadLink
            const pdfLink  = access?.pdf?.downloadLink
            const downloadUrl = epubLink ?? pdfLink
            if (!downloadUrl) {continue}

            books.push({
                id:          `googlebooks-${item.id}`,
                source:      'googlebooks',
                title:       info.title ?? '',
                author:      info.authors?.[0] ?? '',
                language:    info.language,
                subjects:    info.categories,
                coverUrl:    info.imageLinks?.thumbnail?.replace('http://', 'https://'),
                downloadUrl,
                format:      epubLink ? 'epub' : 'pdf',
                sourceUrl:   info.infoLink,
            })
        }
        return books
    },
}
