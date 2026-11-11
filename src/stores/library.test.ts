import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { backendError, mockInvoke } from '@/test/tauri'
import { COVER_CONCURRENCY, useLibraryStore } from './library'
import { useDeviceStore } from './device'
import { useNotificationsStore } from './notifications'
import { useOpdsStore } from './opds'
import type { Book, BookMetadata } from '@/types'

function book(id: string, metadata: Partial<BookMetadata> = {}, path = `/lib/${id}.epub`): Book {
    return {
        id, path, filename: path.split('/').pop()!, format: 'epub', sizeBytes: 1000,
        dateAdded: '2026-01-01T00:00:00Z',
        metadata: { title: `Title ${id}`, author: `Author ${id}`, categories: [], ...metadata },
    }
}

/** A promise the test resolves by hand, to model a slow backend call. */
function deferred<T>() {
    let resolve!: (v: T) => void
    const promise = new Promise<T>(r => { resolve = r })
    return { promise, resolve }
}

/** Opens a library at `path` holding `books`, with covers disabled. */
async function openWith(path: string, books: Book[]) {
    const store = useLibraryStore()
    mockInvoke.on('scan_library', () => books)
    await store.openLibrary(path)
    return store
}

beforeEach(() => {
    setActivePinia(createPinia())
    // The app creates it at startup; created inside the shelf getter instead,
    // its first read would invalidate that getter once.
    useDeviceStore()
    mockInvoke.reset()
    // Covers are irrelevant here: every book "has none".
    mockInvoke.on('ensure_cover', () => null)
})

describe('library store: opening a library', () => {
    it('discards a slow scan that finishes after another library was opened', async () => {
        const store = useLibraryStore()
        const slowA = deferred<Book[]>()
        mockInvoke.on('scan_library', ({ path }) => path === '/root/alice' ? slowA.promise : [book('b1')])

        const openingA = store.openLibrary('/root/alice')
        await store.openLibrary('/root/bob')
        expect(store.books.map(b => b.id)).toEqual(['b1'])

        // Alice's scan arrives late: it must not replace Bob's books.
        slowA.resolve([book('a1'), book('a2')])
        await openingA
        expect(store.libraryPath).toBe('/root/bob')
        expect(store.books.map(b => b.id)).toEqual(['b1'])
        expect(store.loading).toBe(false)
    })

    it('discards a late scan error from the previous library', async () => {
        const store = useLibraryStore()
        const slowA = deferred<Book[]>()
        mockInvoke.on('scan_library', ({ path }) => path === '/root/alice'
            ? slowA.promise.then(() => backendError('share not mounted'))
            : [book('b1')])

        const openingA = store.openLibrary('/root/alice')
        await store.openLibrary('/root/bob')
        slowA.resolve([])
        await openingA
        expect(store.error).toBeNull()
        expect(store.books.map(b => b.id)).toEqual(['b1'])
    })

    it('drops the result of an import started in the previous library', async () => {
        const store = await openWith('/root/alice', [book('a1')])
        const slowImport = deferred<Book[]>()
        mockInvoke.on('import_books', () => slowImport.promise)

        const importing = store.importBooks(['/Downloads/new.epub'])
        await openWith('/root/bob', [book('b1')])
        slowImport.resolve([book('a-new')])

        await expect(importing).resolves.toEqual([])
        expect(store.books.map(b => b.id)).toEqual(['b1'])
    })

    it('resets selection, filters and search when another library opens', async () => {
        const store = await openWith('/root/alice', [book('a1'), book('a2')])
        store.toggleSelect('a1')
        store.setFilters({ search: 'Title', formats: ['epub'] })

        await openWith('/root/bob', [book('b1')])
        expect(store.selectedIds.size).toBe(0)
        expect(store.filters).toEqual({ search: '', formats: [], authors: [], series: [], languages: [], categories: [], presence: [] })
    })

    it('closes the library when the path is null', async () => {
        const store = await openWith('/root/alice', [book('a1')])
        await store.openLibrary(null)
        expect(store.libraryPath).toBeNull()
        expect(store.books).toEqual([])
    })

    it('keeps the error to show it when the scan fails', async () => {
        const store = useLibraryStore()
        mockInvoke.on('scan_library', () => backendError('books.json is unreadable'))
        await store.openLibrary('/root/alice')
        expect(store.error).toBe('books.json is unreadable')
        expect(store.loading).toBe(false)
    })
})

describe('library store: selection', () => {
    it('prunes the selection to the visible books when the search changes', async () => {
        const store = await openWith('/lib', [
            book('dune', { title: 'Dune' }),
            book('emma', { title: 'Emma' }),
            book('ulysses', { title: 'Ulysses' }),
        ])
        store.selectAll()
        expect(store.selectedIds.size).toBe(3)

        store.setFilters({ search: 'dune' })
        expect([...store.selectedIds]).toEqual(['dune'])

        // Widening the filter again does not bring hidden books back.
        store.setFilters({ search: '' })
        expect([...store.selectedIds]).toEqual(['dune'])
    })

    it('prunes the selection when a format filter hides books', async () => {
        const pdf = { ...book('p1'), format: 'pdf' as const }
        const store = await openWith('/lib', [book('e1'), pdf])
        store.toggleSelect('e1')
        store.toggleSelect('p1')
        store.setFilters({ formats: ['pdf'] })
        expect([...store.selectedIds]).toEqual(['p1'])
    })

    it('select all only selects visible books', async () => {
        const store = await openWith('/lib', [book('a', { title: 'Alpha' }), book('b', { title: 'Beta' })])
        store.setFilters({ search: 'alpha' })
        store.selectAll()
        expect([...store.selectedIds]).toEqual(['a'])
    })
})

describe('library store: metadata', () => {
    it('sends only the patch for a batch edit, never book #1’s metadata', async () => {
        const store = await openWith('/lib', [
            book('a', { title: 'Dune', isbn: '111', description: 'Spice' }),
            book('b', { title: 'Emma', isbn: '222' }),
        ])
        mockInvoke.on('batch_update_metadata', ({ bookIds }) => ({
            books: (bookIds as string[]).map(id => book(id, { title: id === 'a' ? 'Dune' : 'Emma', author: 'Same Author' })),
            failures: [],
        }))

        const result = await store.batchUpdateMetadata(['a', 'b'], { author: 'Same Author', series: undefined })

        const call = mockInvoke.callsTo('batch_update_metadata')[0]
        expect(call.args).toEqual({ libraryPath: '/lib', bookIds: ['a', 'b'], patch: { author: 'Same Author' } })
        expect(result.failures).toEqual([])
        expect(store.books.find(b => b.id === 'b')!.metadata).toMatchObject({ title: 'Emma', author: 'Same Author' })
        expect(mockInvoke.callsTo('write_epub_metadata')).toHaveLength(0)
    })

    it('returns per-book failures and only applies the books that were saved', async () => {
        const store = await openWith('/lib', [book('a'), book('b')])
        mockInvoke.on('batch_update_metadata', () => ({
            books: [book('a', { author: 'New' })],
            failures: [{ id: 'b', error: 'file is locked' }],
        }))
        const result = await store.batchUpdateMetadata(['a', 'b'], { author: 'New' })
        expect(result.failures).toEqual([{ id: 'b', error: 'file is locked' }])
        expect(store.books.find(b => b.id === 'a')!.metadata.author).toBe('New')
        expect(store.books.find(b => b.id === 'b')!.metadata.author).toBe('Author b')
    })

    it('sends a single edit by id with the full merged metadata', async () => {
        const store = await openWith('/lib', [book('a', { title: 'Dune', description: 'Spice', series: 'Dune' })])
        mockInvoke.on('update_metadata', ({ metadata }) => ({ ...book('a'), metadata }))

        await store.updateMetadata('a', { title: 'Dune (1965)', series: undefined })

        const args = mockInvoke.callsTo('update_metadata')[0].args
        expect(args.libraryPath).toBe('/lib')
        expect(args.bookId).toBe('a')
        expect(args).not.toHaveProperty('path')
        // Kept: description. Changed: title. Cleared on purpose: series.
        expect(args.metadata).toEqual({ title: 'Dune (1965)', author: 'Author a', description: 'Spice', categories: [] })
    })

    it('rejects with the backend message so the dialog can show it', async () => {
        const store = await openWith('/lib', [book('a')])
        mockInvoke.on('update_metadata', () => backendError('EPUB is read-only'))
        await expect(store.updateMetadata('a', { title: 'x' })).rejects.toBe('EPUB is read-only')
        expect(store.books[0].metadata.title).toBe('Title a')
    })
})

describe('library store: delete', () => {
    it('deletes by id and only removes what the backend deleted', async () => {
        const store = await openWith('/lib', [book('a'), book('b'), book('c')])
        store.selectAll()
        mockInvoke.on('delete_books', () => ({
            deleted: ['a', 'b'], failures: [{ id: 'c', error: 'in use' }], libraryTrash: '/lib/.librarium/trash/2026',
        }))

        const result = await store.deleteBooks(['a', 'b', 'c'])

        expect(mockInvoke.callsTo('delete_books')[0].args).toEqual({ libraryPath: '/lib', bookIds: ['a', 'b', 'c'] })
        expect(store.books.map(b => b.id)).toEqual(['c'])
        expect([...store.selectedIds]).toEqual(['c'])
        expect(result.libraryTrash).toBe('/lib/.librarium/trash/2026')
    })

    it('tells a running OPDS server that the library changed', async () => {
        const store = await openWith('/lib', [book('a')])
        useOpdsStore().running = true
        mockInvoke.on('delete_books', () => ({ deleted: ['a'], failures: [], libraryTrash: null }))
        mockInvoke.on('invalidate_opds_cache', () => undefined)
        await store.deleteBooks(['a'])
        expect(mockInvoke.callsTo('invalidate_opds_cache')).toHaveLength(1)
    })

    it('rejects without a library', async () => {
        const store = useLibraryStore()
        await expect(store.deleteBooks(['a'])).rejects.toBeInstanceOf(Error)
        expect(mockInvoke.callsTo('delete_books')).toHaveLength(0)
    })
})

describe('library store: other id-based commands', () => {
    it('uses libraryPath + bookId for rename, reset, convert and covers', async () => {
        const store = await openWith('/lib', [book('a')])
        mockInvoke.on('rename_book', () => book('a'))
        mockInvoke.on('reset_book_data', () => book('a'))
        mockInvoke.on('convert_to_epub', () => book('a'))
        mockInvoke.on('save_cover_from_url', () => ({ coverPath: '/lib/.covers/thumbs/a.jpg', coverVersion: 1 }))
        mockInvoke.on('set_book_cover', () => ({ coverPath: '/lib/.covers/thumbs/a.jpg', coverVersion: 2 }))
        mockInvoke.on('delete_book_cover', () => undefined)

        await store.renameBook('a', 'Author - Title.epub')
        await store.resetBookData('a')
        await store.convertToEpub(store.books[0])
        await store.saveCoverFromUrl('a', 'https://covers.test/a.jpg')
        expect(store.coverOf('a')).toEqual({ coverPath: '/lib/.covers/thumbs/a.jpg', coverVersion: 1 })
        await store.setBookCover('a', 'BB')
        expect(store.coverOf('a')?.coverVersion).toBe(2)
        await store.deleteBookCover('a')

        expect(mockInvoke.callsTo('rename_book')[0].args).toEqual({ libraryPath: '/lib', bookId: 'a', newFilename: 'Author - Title.epub' })
        expect(mockInvoke.callsTo('reset_book_data')[0].args).toEqual({ libraryPath: '/lib', bookId: 'a' })
        expect(mockInvoke.callsTo('ensure_cover')[0].args).toEqual({ libraryPath: '/lib', bookId: 'a' })
        expect(mockInvoke.callsTo('convert_to_epub')[0].args).toEqual({ libraryPath: '/lib', bookId: 'a' })
        expect(mockInvoke.callsTo('save_cover_from_url')[0].args).toEqual({ libraryPath: '/lib', bookId: 'a', url: 'https://covers.test/a.jpg' })
        expect(mockInvoke.callsTo('set_book_cover')[0].args).toEqual({ libraryPath: '/lib', bookId: 'a', imageBase64: 'BB' })
        expect(mockInvoke.callsTo('delete_book_cover')[0].args).toEqual({ libraryPath: '/lib', bookId: 'a' })
        expect(store.coverOf('a')).toBeNull()
    })

    it('does not add a book twice', async () => {
        const store = await openWith('/lib', [book('a')])
        const added = store.addBooks([book('a'), book('b'), book('b')])
        expect(added.map(b => b.id)).toEqual(['b'])
        expect(store.books.map(b => b.id)).toEqual(['a', 'b'])
    })
})

describe('library store: remove from library (files kept)', () => {
    it('forgets by id and only removes what the backend forgot', async () => {
        const store = await openWith('/lib', [book('a'), book('b'), book('c')])
        store.toggleSelect('a')
        mockInvoke.on('forget_books', () => ({ forgotten: ['a'], failures: [{ id: 'b', error: 'nope' }] }))
        const result = await store.forgetBooks(['a', 'b'])
        expect(mockInvoke.callsTo('forget_books')[0].args).toEqual({ libraryPath: '/lib', bookIds: ['a', 'b'] })
        expect(result.failures.map(f => f.id)).toEqual(['b'])
        expect(store.books.map(b => b.id)).toEqual(['b', 'c'])
        expect(store.selectedIds.has('a')).toBe(false)
    })
})

describe('library store: unreachable library and other instances', () => {
    it('flags an unmounted library instead of a generic error', async () => {
        const store = useLibraryStore()
        mockInvoke.on('scan_library', () => backendError({
            code: 'LIBRARY_UNAVAILABLE',
            message: 'The library folder /Volumes/books/Alice can’t be reached. Is the drive or NAS connected?',
        }))
        await store.openLibrary('/Volumes/books/Alice')
        expect(store.unavailable).toBe(true)
        expect(store.error).toBe('The library folder /Volumes/books/Alice can’t be reached. Is the drive or NAS connected?')
        const toasts = useNotificationsStore().items
        expect(toasts.map(n => n.type)).toEqual(['warning'])
        expect(toasts[0].message).toContain('/Volumes/books/Alice')

        // The share is back: opening again clears the state.
        mockInvoke.on('scan_library', () => [book('a')])
        await store.openLibrary('/Volumes/books/Alice')
        expect(store.unavailable).toBe(false)
        expect(store.books.map(b => b.id)).toEqual(['a'])
    })

    it('tells a refused folder (macOS privacy settings) from a missing one', async () => {
        const store = useLibraryStore()
        const message = 'Librarium isn’t allowed to open the library folder "Alice". In System Settings › Privacy & Security › Files and Folders, allow Librarium to access Network Volumes…'
        mockInvoke.on('scan_library', () => backendError({ code: 'PERMISSION_DENIED', message }))
        await store.openLibrary('/Volumes/books/Alice')
        expect(store.permissionDenied).toBe(true)
        expect(store.unavailable).toBe(false)
        expect(store.error).toBe(message)
        const toasts = useNotificationsStore().items
        expect(toasts.map(n => [n.type, n.message])).toEqual([['warning', message]])

        // Allowed in System Settings: Retry opens it.
        mockInvoke.on('scan_library', () => [book('a')])
        await store.openLibrary('/Volumes/books/Alice')
        expect(store.permissionDenied).toBe(false)
        expect(store.books.map(b => b.id)).toEqual(['a'])
    })

    it('still understands the legacy "LIBRARY_UNAVAILABLE: …" string of older backends', async () => {
        const store = useLibraryStore()
        mockInvoke.on('scan_library', () => backendError('LIBRARY_UNAVAILABLE: The library folder /x can’t be reached.'))
        await store.openLibrary('/x')
        expect(store.unavailable).toBe(true)
        expect(store.error).toBe('The library folder /x can’t be reached.')
    })

    it('warns when another instance used the library recently', async () => {
        mockInvoke.on('library_lock_status', () => ({ otherInstance: { host: 'pc-salon', updatedAt: '2026-11-06T10:00:00Z' } }))
        await openWith('/lib', [book('a')])
        expect(mockInvoke.callsTo('library_lock_status')[0].args).toEqual({ libraryPath: '/lib' })
        const toasts = useNotificationsStore().items
        expect(toasts).toHaveLength(1)
        expect(toasts[0].type).toBe('warning')
        expect(toasts[0].message).toContain('pc-salon')
    })

    it('stays quiet when nobody else uses it, or when the check fails', async () => {
        mockInvoke.on('library_lock_status', () => ({ otherInstance: null }))
        await openWith('/lib', [book('a')])
        mockInvoke.on('library_lock_status', () => backendError('read-only share'))
        await openWith('/lib2', [book('b')])
        expect(useNotificationsStore().items).toEqual([])
    })
})

describe('library store: unknown titles and authors', () => {
    it('sorts empty titles and authors last, in both directions', async () => {
        const store = await openWith('/lib', [
            book('untitled', { title: '', author: '' }),
            book('b', { title: 'Beta', author: 'Zola' }),
            book('a', { title: 'Alpha', author: 'Asimov' }),
        ])
        store.setSort({ field: 'title', order: 'asc' })
        expect(store.filteredBooks.map(b => b.id)).toEqual(['a', 'b', 'untitled'])
        store.setSort({ field: 'title', order: 'desc' })
        expect(store.filteredBooks.map(b => b.id)).toEqual(['b', 'a', 'untitled'])
        store.setSort({ field: 'author', order: 'desc' })
        expect(store.filteredBooks.map(b => b.id)).toEqual(['b', 'a', 'untitled'])
    })

    it('does not offer books with unknown titles as duplicates of each other', async () => {
        const store = await openWith('/lib', [
            book('x', { title: '', author: '' }),
            book('y', { title: '', author: '' }),
            book('d1', { title: 'Dune', author: 'Herbert' }),
            book('d2', { title: 'Dune', author: 'Herbert' }),
        ])
        expect(store.duplicateGroups.map(g => g.books.map(b => b.id))).toEqual([['d1', 'd2']])
    })
})

describe('library store: lazy covers', () => {
    const cover = (id: string, v = 1) => ({ coverPath: `/lib/.covers/thumbs/${id}.jpg`, coverVersion: v })

    it('asks ensure_cover by id, at most COVER_CONCURRENCY at a time, and shows the results', async () => {
        const ids = ['a', 'b', 'c', 'd', 'e', 'f']
        const store = await openWith('/lib', ids.map(id => book(id)))
        const pending = new Map<string, ReturnType<typeof deferred<unknown>>>()
        mockInvoke.on('ensure_cover', ({ bookId }) => {
            const d = deferred<unknown>()
            pending.set(bookId as string, d)
            return d.promise
        })

        ids.forEach(id => store.requestCover(id))
        await flushPromises()
        expect(mockInvoke.callsTo('ensure_cover').map(c => c.args)).toEqual(
            ['a', 'b', 'c', 'd'].map(bookId => ({ libraryPath: '/lib', bookId })))

        pending.get('a')!.resolve(cover('a'))
        pending.get('b')!.resolve(null) // no cover
        await flushPromises()
        expect(mockInvoke.callsTo('ensure_cover')).toHaveLength(COVER_CONCURRENCY + 2)
        expect(store.coverOf('a')).toEqual(cover('a'))
        expect(store.coverOf('b')).toBeNull()

        for (const id of ['c', 'd', 'e', 'f']) { pending.get(id)!.resolve(null) }
        await store.coverQueue.whenIdle()
    })

    it('never asks twice in a session, nor for books the scan already gave a cover', async () => {
        const store = await openWith('/lib', [book('a'), { ...book('b'), ...cover('b') }])
        mockInvoke.on('ensure_cover', () => null)
        store.requestCover('a')
        store.requestCover('b')
        await store.coverQueue.whenIdle()
        store.requestCover('a')
        await store.coverQueue.whenIdle()
        expect(mockInvoke.callsTo('ensure_cover').map(c => c.args.bookId)).toEqual(['a'])

        // A failure isn't retried in a loop either.
        mockInvoke.on('ensure_cover', () => backendError('unreadable book'))
        await openWith('/lib', [book('c')])
        store.requestCover('c')
        store.requestCover('c')
        await store.coverQueue.whenIdle()
        store.requestCover('c')
        await store.coverQueue.whenIdle()
        expect(mockInvoke.callsTo('ensure_cover').map(c => c.args.bookId)).toEqual(['a', 'c'])
    })

    it('drops a withdrawn request before it starts', async () => {
        const ids = ['a', 'b', 'c', 'd', 'e']
        const store = await openWith('/lib', ids.map(id => book(id)))
        const slow = deferred<unknown>()
        mockInvoke.on('ensure_cover', () => slow.promise)
        ids.forEach(id => store.requestCover(id))
        store.cancelCoverRequest('e') // scrolled away while waiting
        slow.resolve(null)
        await store.coverQueue.whenIdle()
        expect(mockInvoke.callsTo('ensure_cover').map(c => c.args.bookId)).toEqual(['a', 'b', 'c', 'd'])
    })

    it('drops results and queued requests of the previous library', async () => {
        const ids = ['a1', 'a2', 'a3', 'a4', 'a5']
        const store = await openWith('/root/alice', ids.map(id => book(id)))
        const slow = deferred<unknown>()
        mockInvoke.on('ensure_cover', () => slow.promise)
        ids.forEach(id => store.requestCover(id))
        await flushPromises()
        expect(mockInvoke.callsTo('ensure_cover')).toHaveLength(4)

        // The new library has books with the same ids, to prove nothing leaks across.
        await openWith('/root/bob', [book('a1'), book('a2')])
        mockInvoke.on('ensure_cover', ({ bookId }) => cover(`bob-${String(bookId)}`))
        // Asked while alice's request for the same id is still running: not swallowed by it.
        store.requestCover('a1')
        slow.resolve(cover('alice'))
        await store.coverQueue.whenIdle()

        expect(mockInvoke.callsTo('ensure_cover').map(c => c.args)).toEqual([
            ...['a1', 'a2', 'a3', 'a4'].map(bookId => ({ libraryPath: '/root/alice', bookId })), // a5 never started
            { libraryPath: '/root/bob', bookId: 'a1' },
        ])
        expect(store.coverOf('a1')).toEqual(cover('bob-a1'))
        expect(store.coverOf('a2'), "alice's a2 result is dropped").toBeNull()
    })

    it('keeps loaded covers across a rescan and an edit', async () => {
        const store = await openWith('/lib', [book('a')])
        mockInvoke.on('ensure_cover', () => cover('a', 7))
        store.requestCover('a')
        await store.coverQueue.whenIdle()

        await store.refresh() // the scan knows no thumbnail for it (e.g. a full-size cover)
        expect(store.coverOf('a')).toEqual(cover('a', 7))

        mockInvoke.on('rename_book', () => book('a', {}, '/lib/renamed.epub'))
        await store.renameBook('a', 'renamed.epub')
        expect(store.books[0].path).toBe('/lib/renamed.epub')
        expect(store.coverOf('a')).toEqual(cover('a', 7))
    })

    it('ensureCover answers from state when it can, else asks the backend', async () => {
        const store = await openWith('/lib', [{ ...book('a'), ...cover('a') }, book('b')])
        mockInvoke.on('ensure_cover', () => null)
        expect(await store.ensureCover('a')).toEqual(cover('a'))
        expect(await store.ensureCover('b')).toBeNull()
        expect(mockInvoke.callsTo('ensure_cover').map(c => c.args.bookId)).toEqual(['b'])
        mockInvoke.on('ensure_cover', () => backendError('index unreadable'))
        await expect(store.ensureCover('b')).rejects.toBe('index unreadable')
    })

    it('retries a cover file that fails to load once, then keeps the placeholder', async () => {
        const store = await openWith('/lib', [{ ...book('a'), ...cover('a', 1) }])
        mockInvoke.on('ensure_cover', () => cover('a', 2))

        store.coverFailedToLoad('a') // e.g. the thumbnail was deleted meanwhile
        expect(store.coverOf('a')).toBeNull()
        store.requestCover('a') // what the visible card does when its cover clears
        await store.coverQueue.whenIdle()
        expect(store.coverOf('a')).toEqual(cover('a', 2))

        store.coverFailedToLoad('a')
        store.requestCover('a')
        await store.coverQueue.whenIdle()
        expect(mockInvoke.callsTo('ensure_cover')).toHaveLength(1)
        expect(store.coverOf('a')).toBeNull()
    })

    it('does not ask again after the user removed a cover', async () => {
        const store = await openWith('/lib', [{ ...book('a'), ...cover('a') }])
        mockInvoke.on('delete_book_cover', () => undefined)
        mockInvoke.on('ensure_cover', () => cover('a'))
        await store.deleteBookCover('a')
        store.requestCover('a')
        await store.coverQueue.whenIdle()
        expect(mockInvoke.callsTo('ensure_cover')).toHaveLength(0)
        expect(store.coverOf('a')).toBeNull()
    })
})

describe('library store: sorting and search', () => {
    it('sorts titles the way people read them (accents, numbers, case)', async () => {
        const store = await openWith('/lib', [
            book('z', { title: 'Zola' }),
            book('e', { title: 'Élise' }),
            book('t10', { title: 'Tome 10' }),
            book('t2', { title: 'tome 2' }),
        ])
        store.setSort({ field: 'title', order: 'asc' })
        expect(store.filteredBooks.map(b => b.id)).toEqual(['e', 't2', 't10', 'z'])
    })

    it('keeps series volumes in order', async () => {
        const store = await openWith('/lib', [
            book('d3', { series: 'Dune', seriesIndex: 3 }),
            book('d1', { series: 'Dune', seriesIndex: 1 }),
            book('a1', { series: 'Antoinette', seriesIndex: 1 }),
        ])
        store.setSort({ field: 'series', order: 'asc' })
        expect(store.filteredBooks.map(b => b.id)).toEqual(['a1', 'd1', 'd3'])
    })

    it('finds books whatever the case and accents of the search', async () => {
        const store = await openWith('/lib', [book('e', { title: 'Élise et les autres' }), book('z', { title: 'Zola' })])
        store.setFilters({ search: '  ELISE ' })
        expect(store.filteredBooks.map(b => b.id)).toEqual(['e'])
        store.setFilters({ search: 'Author z' }) // the author matches too
        expect(store.filteredBooks.map(b => b.id)).toEqual(['z'])
    })

    it('lists authors in display order for the filters', async () => {
        const store = await openWith('/lib', [
            book('a', { author: 'Zweig' }), book('b', { author: 'Éluard' }), book('c', { author: 'Asimov' }),
        ])
        expect(store.allAuthors).toEqual(['Asimov', 'Éluard', 'Zweig'])
    })
})

describe('library store: how books are held', () => {
    it('keeps books frozen: nothing can change one behind the store’s back', async () => {
        const store = await openWith('/lib', [book('a', { categories: ['Fiction'] })])
        const held = store.books[0]
        expect(Object.isFrozen(held)).toBe(true)
        expect(Object.isFrozen(held.metadata)).toBe(true)
        expect(() => { (held.metadata as { title: string }).title = 'x' }).toThrow(TypeError)
        expect(() => { held.metadata.categories!.push('x') }).toThrow(TypeError)
    })

    it('keeps covers out of the books, so a cover arriving does not re-filter the library', async () => {
        const store = await openWith('/lib', [book('a'), book('b')])
        const before = store.filteredBooks
        mockInvoke.on('ensure_cover', ({ bookId }) => ({ coverPath: `/lib/.covers/thumbs/${String(bookId)}.jpg`, coverVersion: 1 }))
        store.requestCover('a')
        await store.coverQueue.whenIdle()
        expect(store.coverOf('a')).not.toBeNull()
        expect(store.books[0].coverPath).toBeUndefined()
        // Same array: the getter didn't run again.
        expect(store.filteredBooks).toBe(before)
    })

    it('does not re-filter the library when the selection changes', async () => {
        const store = await openWith('/lib', [book('a'), book('b')])
        const before = store.filteredBooks
        store.toggleSelect('a')
        store.toggleSelect('b')
        expect(store.filteredBooks).toBe(before)
        expect(store.selectedBooks.map(b => b.id)).toEqual(['a', 'b'])
    })

    it('replaces an edited book with a new snapshot, and finds books by id', async () => {
        const store = await openWith('/lib', [book('a'), book('b')])
        const old = store.books[0]
        mockInvoke.on('update_metadata', ({ metadata }) => ({ ...book('a'), metadata }))
        await store.updateMetadata('a', { title: 'New' })
        expect(store.books[0]).not.toBe(old)
        expect(store.bookById.get('a')?.metadata.title).toBe('New')
        expect(store.bookById.get('missing')).toBeUndefined()
    })

    it('takes the covers the backend sends with added books', async () => {
        const store = await openWith('/lib', [])
        store.addBooks([{ ...book('n'), coverPath: '/lib/.covers/thumbs/n.jpg', coverVersion: 3 }])
        expect(store.coverOf('n')).toEqual({ coverPath: '/lib/.covers/thumbs/n.jpg', coverVersion: 3 })
        expect(store.books[0].coverPath).toBeUndefined()
    })
})

describe('library store: view mode', () => {
    beforeEach(() => { localStorage.removeItem('viewMode') })

    it('starts on the grid, and remembers the view chosen across launches', () => {
        expect(useLibraryStore().viewMode).toBe('grid')
        useLibraryStore().setViewMode('list')
        setActivePinia(createPinia())
        expect(useLibraryStore().viewMode).toBe('list')
        localStorage.setItem('viewMode', 'something else')
        setActivePinia(createPinia())
        expect(useLibraryStore().viewMode).toBe('grid')
    })
})

describe('library store: setting the selection', () => {
    it('replaces the selection, ignoring ids that are not listed', async () => {
        const store = await openWith('/lib', [book('a'), book('b'), book('c')])
        store.toggleSelect('a')
        store.setSelection(['b', 'c', 'zzz'])
        expect([...store.selectedIds].sort()).toEqual(['b', 'c'])
        expect(store.selectedBooks.map(b => b.id)).toEqual(['b', 'c'])
    })
})

describe('library store: the shelf (library and e-reader together)', () => {
    async function withDevice(books: Book[], files: { filename: string, title?: string, author?: string, libraryId?: string }[]) {
        const store = await openWith('/lib', books)
        const device = useDeviceStore()
        device.devices = [{ id: 'k', name: 'Kindle', mountPath: '/Volumes/Kindle', deviceType: 'kindle', connection: 'massStorage', preferredFormat: 'azw3' }]
        device.selectedId = 'k'
        device.deviceBooks = files.map(f => ({ path: `/Volumes/Kindle/documents/${f.filename}`, title: '', author: '', ...f }))
        return { store, device }
    }

    it('shows the e-reader\'s books with the library\'s, and filters them by scope', async () => {
        const { store } = await withDevice(
            [book('a', { title: 'Dune' }), book('b', { title: 'Emma' })],
            [{ filename: 'x.azw3', libraryId: 'a' }, { filename: 'Zola.epub', title: 'Germinal', author: 'Zola' }],
        )
        const germinal = 'device:/Volumes/Kindle/documents/Zola.epub'
        expect(store.visibleItems.map(i => i.id)).toEqual(['a', 'b', germinal])
        store.setScope('library')
        expect(store.visibleItems.map(i => i.id)).toEqual(['a', 'b'])
        store.setScope('device')
        expect(store.visibleItems.map(i => i.id)).toEqual(['a', germinal])
        // The library books shown, for library actions on "visible" books.
        expect(store.filteredBooks.map(b => b.id)).toEqual(['a'])
    })

    it('without a device, is the library alone, whatever the scope', async () => {
        const store = await openWith('/lib', [book('a')])
        store.setScope('device')
        expect(store.effectiveScope).toBe('all')
        expect(store.visibleItems.map(i => [i.id, i.presence])).toEqual([['a', 'libraryOnly']])
    })

    it('selects device files too, and drops them when they leave the screen', async () => {
        const { store, device } = await withDevice([book('a')], [{ filename: 'z.epub', title: 'Z', author: 'Y' }])
        const file = 'device:/Volumes/Kindle/documents/z.epub'
        store.selectAll()
        expect([...store.selectedIds]).toEqual(['a', file])
        expect(store.selectedBooks.map(b => b.id)).toEqual(['a'])
        expect(store.selectedItems.map(i => i.id)).toEqual(['a', file])

        store.setScope('library')
        expect([...store.selectedIds]).toEqual(['a'])

        store.setScope('all')
        store.setSelection(['a', file, 'device:/nowhere'])
        expect([...store.selectedIds]).toEqual(['a', file])
        // Unplugged: the device's books go (LibraryView prunes when the shelf changes).
        device.devices = []
        store.pruneSelection()
        expect([...store.selectedIds]).toEqual(['a'])
    })

    it('filters by where a book is ("not sent yet")', async () => {
        const { store } = await withDevice([book('a'), book('b')], [{ filename: 'x.azw3', libraryId: 'a' }])
        store.setFilters({ presence: ['libraryOnly'] })
        expect(store.visibleItems.map(i => i.id)).toEqual(['b'])
    })
})
