import type { StoreProvider, StoreBook } from '../types'

const BASE_URL = 'https://gutendex.com/books'

/** The parts of a Gutendex `/books` answer this provider reads. */
interface GutendexResponse {
    results?: GutendexBook[]
}

interface GutendexBook {
    id: number
    title?: string
    authors?: { name?: string }[]
    languages?: string[]
    subjects?: string[]
    /** MIME type → URL. */
    formats?: Record<string, string | undefined>
}

/** Project Gutenberg's catalogue, through the Gutendex API (EPUB, else PDF). */
export const gutendexProvider: StoreProvider = {
    id: 'gutendex',
    label: 'Project Gutenberg',

    isAvailable(): boolean {
        return true
    },

    async search(query: string, signal?: AbortSignal): Promise<StoreBook[]> {
        const params = new URLSearchParams({ search: query })
        const res = await fetch(`${BASE_URL}/?${params.toString()}`, { signal })
        if (!res.ok) {throw new Error(`Gutendex answered HTTP ${res.status}`)}
        const data = await res.json() as GutendexResponse

        const books: StoreBook[] = []
        for (const item of data.results ?? []) {
            const formats = item.formats ?? {}
            const epubUrl = formats['application/epub+zip']
            const pdfUrl  = formats['application/pdf']
            const downloadUrl = epubUrl ?? pdfUrl
            if (!downloadUrl) {continue}

            books.push({
                id:          `gutendex-${item.id}`,
                source:      'gutendex',
                title:       item.title ?? '',
                author:      item.authors?.[0]?.name ?? '',
                language:    item.languages?.[0],
                subjects:    item.subjects,
                coverUrl:    formats['image/jpeg'],
                downloadUrl,
                format:      epubUrl ? 'epub' : 'pdf',
                sourceUrl:   `https://www.gutenberg.org/ebooks/${item.id}`,
            })
        }
        return books
    },
}
