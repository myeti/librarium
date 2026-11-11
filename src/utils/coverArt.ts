import type { BookMetadata } from '@/types'

/** How many cover colours each theme has (`--cover-1` … `--cover-6` in themes.css). */
export const COVER_COLOR_COUNT = 6

/**
 * The cloth colour of a book's generated cover: a `var(--cover-N)` token,
 * the same for the book on every launch. Books of one series share theirs,
 * like a matching set on a shelf; other books are spread by their id (a
 * file only on the e-reader: its path).
 */
export function coverColor(book: { id: string, metadata?: Pick<BookMetadata, 'series'> }): string {
    const key = book.metadata?.series?.trim().toLowerCase() || book.id
    let hash = 0
    for (const c of key) { hash = (hash * 31 + c.charCodeAt(0)) >>> 0 }
    return `var(--cover-${(hash % COVER_COLOR_COUNT) + 1})`
}
