import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
const dialogOpen = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: dialogOpen }))
import { backendError, mockInvoke } from '@/test/tauri'
import { DROP_REPEAT_WINDOW_MS, importablePaths, useImportsStore } from './imports'
import { useLibraryStore } from './library'
import { useNotificationsStore } from './notifications'
import type { Book, BookFormat } from '@/types'

function book(id: string, format: BookFormat = 'epub'): Book {
    return {
        id, path: `/lib/${id}.${format}`, filename: `${id}.${format}`, format, sizeBytes: 1, dateAdded: '2026-01-01',
        metadata: { title: id, author: 'A', categories: [] },
    }
}

/** A promise the test resolves by hand. */
function deferred<T>() {
    let resolve!: (v: T) => void
    const promise = new Promise<T>(r => { resolve = r })
    return { promise, resolve }
}

beforeEach(async () => {
    setActivePinia(createPinia())
    mockInvoke.reset()
    dialogOpen.mockReset()
    mockInvoke.on('scan_library', () => [])
    await useLibraryStore().openLibrary('/lib')
})

describe('importablePaths', () => {
    it('keeps the supported ebook files, once each, in order', () => {
        expect(importablePaths(['/a.epub', '/b.txt', '/C.PDF', '/a.epub', '/dir', '/d.azw3', '/e.cbz', '/f.mobi']))
            .toEqual(['/a.epub', '/C.PDF', '/d.azw3', '/e.cbz', '/f.mobi'])
    })
})

describe('imports store: dropped files', () => {
    it('shows the overlay while files are dragged over the window', () => {
        const imports = useImportsStore()
        expect(imports.onFileDrop({ type: 'enter' })).toBeNull()
        expect(imports.dragging).toBe(true)
        expect(imports.onFileDrop({ type: 'over' })).toBeNull()
        expect(imports.dragging).toBe(true)
        expect(imports.onFileDrop({ type: 'leave' })).toBeNull()
        expect(imports.dragging).toBe(false)
    })

    it('imports the ebooks of a drop and adds them to the library', async () => {
        const imports = useImportsStore()
        mockInvoke.on('import_books', ({ paths }) => (paths as string[]).map((_p, i) => book(`n${i}`)))
        expect(imports.onFileDrop({ type: 'enter' })).toBeNull()
        const done = imports.onFileDrop({ type: 'drop', paths: ['/x/a.epub', '/x/notes.txt', '/x/b.pdf'] })
        expect(imports.dragging).toBe(false)
        await done
        expect(mockInvoke.callsTo('import_books')[0].args.paths).toEqual(['/x/a.epub', '/x/b.pdf'])
        expect(useLibraryStore().books.map(b => b.id)).toEqual(['n0', 'n1'])
    })

    it('says so when nothing dropped can be imported', () => {
        const imports = useImportsStore()
        expect(imports.onFileDrop({ type: 'drop', paths: ['/x/notes.txt', '/x/folder'] })).toBeNull()
        expect(mockInvoke.callsTo('import_books')).toHaveLength(0)
        expect(useNotificationsStore().items.map(n => n.type)).toEqual(['info'])
    })

    it('ignores the same drop delivered twice (WebView2), but not a later identical drop', async () => {
        const imports = useImportsStore()
        mockInvoke.on('import_books', () => [])
        await imports.onFileDrop({ type: 'drop', paths: ['/a.epub'] }, 1000)
        expect(imports.onFileDrop({ type: 'drop', paths: ['/a.epub'] }, 1000 + DROP_REPEAT_WINDOW_MS - 1)).toBeNull()
        await imports.onFileDrop({ type: 'drop', paths: ['/a.epub'] }, 1000 + DROP_REPEAT_WINDOW_MS + 1)
        expect(mockInvoke.callsTo('import_books')).toHaveLength(2)
    })

    it('offers EPUB conversion for imported MOBI/AZW3 books', async () => {
        const imports = useImportsStore()
        mockInvoke.on('import_books', () => [book('m', 'mobi'), book('e', 'epub'), book('k', 'azw3')])
        await imports.importPaths(['/m.mobi', '/e.epub', '/k.azw3'])
        expect(imports.conversionOffer.map(b => b.id)).toEqual(['m', 'k'])
        imports.dismissConversionOffer()
        expect(imports.conversionOffer).toEqual([])
    })
})

describe('imports store: queue', () => {
    it('runs imports one at a time, in order', async () => {
        const imports = useImportsStore()
        const first = deferred<Book[]>()
        const started: string[] = []
        mockInvoke.on('import_books', ({ paths }) => {
            const p = (paths as string[])[0]
            started.push(p)
            return p === '/1.epub' ? first.promise : [book('two')]
        })
        const one = imports.importPaths(['/1.epub'])
        const two = imports.importPaths(['/2.epub'])
        expect(imports.busy).toBe(true)
        await Promise.resolve()
        await Promise.resolve()
        expect(started).toEqual(['/1.epub']) // the second waits
        first.resolve([book('one')])
        await one
        await two
        expect(started).toEqual(['/1.epub', '/2.epub'])
        expect(useLibraryStore().books.map(b => b.id)).toEqual(['one', 'two'])
        expect(imports.busy).toBe(false)
    })

    it('shows a failure and carries on with the next import', async () => {
        const imports = useImportsStore()
        mockInvoke.on('import_books', ({ paths }) => (paths as string[])[0] === '/bad.epub'
            ? backendError({ code: 'IO', message: 'disk full' })
            : [book('ok')])
        const bad = await imports.importPaths(['/bad.epub'])
        const good = await imports.importPaths(['/ok.epub'])
        expect(bad).toEqual([])
        expect(good.map(b => b.id)).toEqual(['ok'])
        const toasts = useNotificationsStore().items
        expect(toasts.map(n => n.type)).toEqual(['error'])
        expect(toasts[0].message).toContain('disk full')
    })

    it('imports what was picked in the file dialog (nothing when cancelled)', async () => {
        const imports = useImportsStore()
        mockInvoke.on('import_books', () => [])
        dialogOpen.mockResolvedValueOnce(null)
        await imports.pickAndImport()
        expect(mockInvoke.callsTo('import_books')).toHaveLength(0)
        dialogOpen.mockResolvedValueOnce(['/a.epub', '/b.pdf'])
        await imports.pickAndImport()
        expect(mockInvoke.callsTo('import_books')[0].args.paths).toEqual(['/a.epub', '/b.pdf'])
    })
})
