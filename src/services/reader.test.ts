import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { backendError, convertFileSrc, mockInvoke } from '@/test/tauri'
import { fetchBytes, needsConversion, openBook } from './reader'
import type { Book, BookFormat, ReaderSource } from '@/types'

function book(format: BookFormat, path = `/lib/Book.${format}`): Book {
    return {
        id: 'b1', path, filename: path.split('/').pop()!, format, sizeBytes: 10, dateAdded: '2026-01-01T00:00:00Z',
        metadata: { title: 'Book', author: 'A', categories: [] },
    }
}

function source(format: 'epub' | 'pdf', path: string, converted = false): ReaderSource {
    return { format, path, sizeBytes: 10, converted }
}

const fetchMock = vi.fn<typeof fetch>()

beforeEach(() => {
    mockInvoke.reset()
    fetchMock.mockReset()
    vi.stubGlobal('fetch', fetchMock)
})

afterEach(() => {
    vi.unstubAllGlobals()
})

describe('reader service', () => {
    it('gives a PDF to the viewer as its asset URL, without reading it', async () => {
        mockInvoke.on('open_for_reading', () => source('pdf', '/lib/Scan.pdf'))
        const opened = await openBook('/lib', book('pdf', '/lib/Scan.pdf'))
        expect(opened).toEqual({ format: 'pdf', url: convertFileSrc('/lib/Scan.pdf') })
        expect(mockInvoke.calls).toEqual([{ cmd: 'open_for_reading', args: { libraryPath: '/lib', bookId: 'b1' } }])
        expect(fetchMock).not.toHaveBeenCalled()
    })

    it('fetches an EPUB (or a Kindle book’s cached conversion) from its asset URL', async () => {
        const bytes = new Uint8Array([0x50, 0x4b, 3, 4]).buffer
        mockInvoke.on('open_for_reading', () => source('epub', '/cache/reader/b1-10-1.epub', true))
        fetchMock.mockResolvedValue(new Response(bytes, { status: 200 }))

        const opened = await openBook('/lib', book('azw3'))
        expect(fetchMock).toHaveBeenCalledWith(convertFileSrc('/cache/reader/b1-10-1.epub'))
        expect(opened.format).toBe('epub')
        expect(opened.format === 'epub' && new Uint8Array(opened.data)).toEqual(new Uint8Array([0x50, 0x4b, 3, 4]))
    })

    it('passes the backend’s error through, and reports a file the webview may not load', async () => {
        const error = { code: 'INVALID_INPUT', message: 'The reader can’t open CBZ files' }
        mockInvoke.on('open_for_reading', () => backendError(error))
        await expect(openBook('/lib', book('cbz'))).rejects.toBe(error)

        fetchMock.mockResolvedValue(new Response('forbidden', { status: 403 }))
        await expect(fetchBytes('asset://localhost/x')).rejects.toThrow('403')
    })

    it('only Kindle formats are converted first', () => {
        expect(['epub', 'pdf', 'mobi', 'azw3', 'cbz'].map(f => needsConversion(f as BookFormat))).toEqual([false, false, true, true, false])
    })
})
