/**
 * Opening a book in the in-app reader (docs/IPC.md, "Reader").
 *
 * The backend only says *which file* to display: the book itself (EPUB,
 * PDF) or its cached EPUB conversion (MOBI, AZW, AZW3). The webview then
 * loads that file through Tauri's asset protocol (`convertFileSrc`), so a
 * book never travels over IPC (as base64 inside JSON, a large PDF would
 * take several GB of memory).
 *
 * - PDF: the asset URL goes straight into an iframe; the webview's PDF
 *   viewer reads the file itself (with Range requests where it wants them).
 * - EPUB: the file is fetched as an ArrayBuffer for epub.js.
 */
import { convertFileSrc } from '@tauri-apps/api/core'
import * as api from '@/services/api'
import { t } from '@/i18n'
import type { Book, BookFormat } from '@/types'

/** What the reader displays. */
export type OpenedBook =
    | { format: 'pdf', url: string }
    | { format: 'epub', data: ArrayBuffer }

/**
 * Whether opening a book of this format starts with a conversion to EPUB
 * (Kindle formats; `mobi` also covers AZW). Only the first open of a given
 * file converts; later ones come from the backend's cache.
 */
export function needsConversion(format: BookFormat): boolean {
    return format === 'mobi' || format === 'azw3'
}

/** Opens `book` (of the library at `libraryPath`) for the reader. Rejects with the backend's error, or a load failure. */
export async function openBook(libraryPath: string, book: Book): Promise<OpenedBook> {
    const source = await api.openForReading(libraryPath, book.id)
    const url = convertFileSrc(source.path)
    if (source.format === 'pdf') {
        return { format: 'pdf', url }
    }
    return { format: 'epub', data: await fetchBytes(url) }
}

/** The bytes at `url` (an asset URL). Rejects when the webview can't load it (outside the asset scope: 403). */
export async function fetchBytes(url: string): Promise<ArrayBuffer> {
    const response = await fetch(url)
    if (!response.ok) {
        throw new Error(t('reader.loadFailed', { status: response.status }))
    }
    return response.arrayBuffer()
}
