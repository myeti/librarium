import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
type Handler = (event: { payload: unknown }) => void
const listeners = new Map<string, Handler>()
vi.mock('@tauri-apps/api/event', () => ({
    listen: vi.fn((name: string, handler: Handler) => {
        listeners.set(name, handler)
        return Promise.resolve(() => { listeners.delete(name) })
    }),
}))
import { backendError, mockInvoke } from '@/test/tauri'
import { useCalibreStore } from './calibre'
import { useLibraryStore } from './library'
import { useOpdsStore } from './opds'
import type { Book, CalibreImportProgress, CalibreImportResult } from '@/types'

function book(id: string): Book {
    return {
        id, path: `/lib/${id}.epub`, filename: `${id}.epub`, format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01T00:00:00Z',
        metadata: { title: id, author: 'A', categories: [] },
    }
}

function result(overrides: Partial<CalibreImportResult> = {}): CalibreImportResult {
    return { imported: [], skipped: [], failed: [], cancelled: false, error: null, remaining: 0, ...overrides }
}

/** A promise the test resolves by hand: the import running in the backend. */
function deferred<T>() {
    let resolve!: (v: T) => void
    let reject!: (e: unknown) => void
    const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej })
    return { promise, resolve, reject }
}

function emitProgress(progress: CalibreImportProgress) {
    listeners.get('calibre-import-progress')!({ payload: progress })
}

async function openLibrary(books: Book[] = []) {
    mockInvoke.on('scan_library', () => books)
    const library = useLibraryStore()
    await library.openLibrary('/lib')
    return library
}

beforeEach(() => {
    setActivePinia(createPinia())
    mockInvoke.reset()
    listeners.clear()
    mockInvoke.on('ensure_cover', () => null)
    mockInvoke.on('invalidate_opds_cache', () => undefined)
})

describe('calibre store', () => {
    it('follows the progress, then adds the imported books and refreshes the OPDS cache', async () => {
        const library = await openLibrary([book('old')])
        useOpdsStore().running = true
        const store = useCalibreStore()
        const running = deferred<CalibreImportResult>()
        mockInvoke.on('import_from_calibre', () => running.promise)

        const importing = store.importBooks('/calibre', [1, 2, 2])
        await vi.waitFor(() => expect(mockInvoke.callsTo('import_from_calibre')).toHaveLength(1))
        expect(mockInvoke.callsTo('import_from_calibre')[0].args).toEqual({ calibrePath: '/calibre', bookIds: [1, 2, 2], libraryPath: '/lib' })
        expect(store.running).toBe(true)
        expect(store.progress).toEqual({ done: 0, total: 2, currentTitle: null })

        emitProgress({ done: 1, total: 2, currentTitle: 'Dune' })
        expect(store.progress?.currentTitle).toBe('Dune')
        expect(store.percent).toBe(50)

        running.resolve(result({ imported: [book('dune')] }))
        await importing
        expect(library.books.map(b => b.id)).toEqual(['old', 'dune'])
        expect(mockInvoke.callsTo('invalidate_opds_cache')).toHaveLength(1)
        expect(store.running).toBe(false)
        expect(store.progress).toBeNull()
        expect(listeners.has('calibre-import-progress')).toBe(false)
    })

    it('cancels: the import resolves with what it had imported', async () => {
        const library = await openLibrary()
        const store = useCalibreStore()
        const running = deferred<CalibreImportResult>()
        mockInvoke.on('import_from_calibre', () => running.promise)
        mockInvoke.on('cancel_calibre_import', () => { running.resolve(result({ imported: [book('a')], cancelled: true, remaining: 3 })); return true })

        const importing = store.importBooks('/calibre', [1, 2, 3, 4])
        await vi.waitFor(() => expect(store.running).toBe(true))
        await store.cancel()
        await store.cancel() // a second click does nothing
        expect(mockInvoke.callsTo('cancel_calibre_import')).toHaveLength(1)

        const outcome = await importing
        expect(outcome.cancelled).toBe(true)
        expect(library.books.map(b => b.id)).toEqual(['a'])
        expect(store.cancelling).toBe(false)
    })

    it('rescans when the import stopped on a library problem', async () => {
        await openLibrary()
        const store = useCalibreStore()
        mockInvoke.on('import_from_calibre', () => result({ imported: [book('a')], error: 'Could not save the library index', remaining: 5 }))
        await store.importBooks('/calibre', [1])
        // Opening the library scanned once; the import scanned again.
        expect(mockInvoke.callsTo('scan_library')).toHaveLength(2)
    })

    it('rescans after a rejection that may have copied books, not after a Calibre-side one', async () => {
        await openLibrary()
        const store = useCalibreStore()
        const copied = { code: 'IO', message: '3 book(s) were copied into the library, but the library index could not be updated' }
        mockInvoke.on('import_from_calibre', () => backendError(copied))
        await expect(store.importBooks('/calibre', [1])).rejects.toBe(copied)
        expect(mockInvoke.callsTo('scan_library')).toHaveLength(2)

        const noDatabase = { code: 'NOT_FOUND', message: 'There is no metadata.db in this folder' }
        mockInvoke.on('import_from_calibre', () => backendError(noDatabase))
        await expect(store.importBooks('/calibre', [1])).rejects.toBe(noDatabase)
        expect(mockInvoke.callsTo('scan_library')).toHaveLength(2)
        expect(store.running).toBe(false)
    })

    it('runs one import at a time', async () => {
        await openLibrary()
        const store = useCalibreStore()
        const running = deferred<CalibreImportResult>()
        mockInvoke.on('import_from_calibre', () => running.promise)
        const first = store.importBooks('/calibre', [1])
        await vi.waitFor(() => expect(store.running).toBe(true))

        await expect(store.importBooks('/calibre', [2])).rejects.toThrow(/^CONFLICT: /)
        expect(store.running).toBe(true)
        running.resolve(result())
        await first
        expect(mockInvoke.callsTo('import_from_calibre')).toHaveLength(1)
    })

    it('needs an open library', async () => {
        await expect(useCalibreStore().importBooks('/calibre', [1])).rejects.toThrow()
        expect(mockInvoke.callsTo('import_from_calibre')).toHaveLength(0)
    })
})
