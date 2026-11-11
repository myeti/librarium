import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }))
import { backendError, mockInvoke } from '@/test/tauri'
import { useBookActionsStore } from './bookActions'
import { useLibraryStore } from './library'
import { useDeviceStore } from './device'
import { useNotificationsStore } from './notifications'
import type { Book } from '@/types'

function book(id: string, title = `Title ${id}`): Book {
    return {
        id, path: `/lib/${id}.epub`, filename: `${id}.epub`, format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01',
        metadata: { title, author: 'Author', categories: [] },
    }
}

async function openLibrary(books: Book[]) {
    mockInvoke.on('scan_library', () => books)
    const library = useLibraryStore()
    await library.openLibrary('/lib')
    return library
}

beforeEach(() => {
    setActivePinia(createPinia())
    mockInvoke.reset()
})

describe('book actions: move to Trash', () => {
    it('asks first, naming the book, then deletes and says so', async () => {
        const library = await openLibrary([book('a', 'Dune'), book('b')])
        const actions = useBookActionsStore()
        mockInvoke.on('delete_books', ({ bookIds }) => ({ deleted: bookIds, failures: [], libraryTrash: null }))

        actions.requestDelete(['a'], 'single')
        expect(actions.pendingDelete).toEqual({ ids: ['a'], origin: 'single' })
        expect(actions.deleteTitle).toBe('Move “Dune” to the Trash?')
        expect(mockInvoke.callsTo('delete_books')).toHaveLength(0)

        await actions.confirmDelete()
        expect(actions.pendingDelete).toBeNull()
        expect(library.books.map(b => b.id)).toEqual(['b'])
        expect(useNotificationsStore().items.map(n => n.type)).toEqual(['success'])
    })

    it('limits a selection to the books on screen (never deletes hidden books)', async () => {
        const library = await openLibrary([book('a', 'Dune'), book('b', 'Emma'), book('c', 'Dune Messiah')])
        const actions = useBookActionsStore()
        library.selectAll()
        library.setFilters({ search: 'dune' }) // prunes the selection to a, c
        library.selectedIds.add('b') // even if a hidden book is still selected somehow
        actions.requestDelete([...library.selectedIds], 'selection')
        expect(actions.pendingDelete?.ids.sort()).toEqual(['a', 'c'])
        expect(actions.deleteTitle).toBe('Move 2 books to the Trash?')
    })

    it('does nothing when no listed book is left to delete', async () => {
        await openLibrary([book('a')])
        const actions = useBookActionsStore()
        actions.requestDelete(['gone'], 'single')
        expect(actions.pendingDelete).toBeNull()
    })

    it('keeps the dialog open with the reason when the delete fails', async () => {
        const library = await openLibrary([book('a')])
        const actions = useBookActionsStore()
        mockInvoke.on('delete_books', () => backendError({ code: 'IO', message: 'volume is read-only' }))
        actions.requestDelete(['a'], 'single')
        await actions.confirmDelete()
        expect(actions.pendingDelete).not.toBeNull()
        expect(actions.deleteError).toContain('volume is read-only')
        expect(actions.deleting).toBe(false)
        expect(library.books).toHaveLength(1)
    })

    it('keeps a delete in progress when the dialogs are reset', async () => {
        await openLibrary([book('a')])
        const actions = useBookActionsStore()
        let release!: () => void
        mockInvoke.on('delete_books', () => new Promise(r => { release = () => r({ deleted: ['a'], failures: [], libraryTrash: null }) }))
        actions.requestDelete(['a'], 'single')
        actions.edit([book('a')])
        const running = actions.confirmDelete()
        actions.reset()
        expect(actions.editing).toEqual([])
        expect(actions.pendingDelete).not.toBeNull() // still deleting: stays
        release()
        await running
        expect(actions.pendingDelete).toBeNull()
    })
})

describe('book actions: remove from library', () => {
    it('asks first, then forgets the books (files kept)', async () => {
        const library = await openLibrary([book('a'), book('b')])
        const actions = useBookActionsStore()
        mockInvoke.on('forget_books', ({ bookIds }) => ({ forgotten: bookIds, failures: [] }))
        actions.requestForget(['a', 'b'], 'single')
        expect(actions.forgetTitle).toBe('Remove 2 books from the library?')
        await actions.confirmForget()
        expect(library.books).toEqual([])
        expect(actions.pendingForget).toBeNull()
    })
})

describe('book actions: send to the device', () => {
    function connect(deviceBooks: { filename: string, libraryId?: string }[] = []) {
        const device = useDeviceStore()
        device.devices = [{ id: 'k', name: 'Kindle', mountPath: '/Volumes/Kindle', deviceType: 'kindle', connection: 'massStorage', preferredFormat: 'azw3' }]
        device.selectedId = 'k'
        device.deviceBooks = deviceBooks.map(d => ({ path: `/Volumes/Kindle/documents/${d.filename}`, title: '', author: '', ...d }))
        const send = vi.spyOn(device, 'sendBooks').mockResolvedValue(undefined)
        return { device, send }
    }

    it('sends at once when none of the books is on the device', async () => {
        await openLibrary([book('a')])
        const { send } = connect()
        useBookActionsStore().send([book('a')])
        expect(send).toHaveBeenCalledWith([expect.objectContaining({ id: 'a' })])
    })

    it('asks before sending again a book that is already there', async () => {
        await openLibrary([book('a'), book('b')])
        const { send } = connect([{ filename: 'a.azw3', libraryId: 'a' }])
        const actions = useBookActionsStore()
        actions.send([book('a'), book('b')])
        expect(send).not.toHaveBeenCalled()
        expect(actions.pendingSendOnDevice).toBe(1)
        actions.confirmSend()
        expect(send).toHaveBeenCalledTimes(1)
        expect(actions.pendingSend).toEqual([])
    })
})

describe('book actions: tidy-up review', () => {
    function review(b: Book) {
        const actions = useBookActionsStore()
        actions.review = { book: b, metadata: { title: 'Dune' }, source: 'openlibrary', coverUrl: null, confident: true }
        return actions
    }

    it('saves the ticked fields, then closes the review', async () => {
        const library = await openLibrary([book('a', 'dune')])
        mockInvoke.on('update_metadata', ({ metadata }) => ({ ...book('a'), metadata }))
        const actions = review(library.books[0])
        await actions.applyReview({ title: 'Dune', author: undefined }, null)
        const sent = mockInvoke.callsTo('update_metadata')[0].args as { metadata: Record<string, unknown> }
        expect(sent.metadata.title).toBe('Dune')
        // An undefined field is not applied (it would clear the author).
        expect(sent.metadata.author).toBe('Author')
        expect(actions.review).toBeNull()
    })

    it('rejects when the metadata can’t be saved, and keeps the review open', async () => {
        const library = await openLibrary([book('a')])
        mockInvoke.on('update_metadata', () => backendError({ code: 'IO', message: 'disk full' }))
        const actions = review(library.books[0])
        await expect(actions.applyReview({ title: 'Dune' }, 'https://covers.example/1.jpg')).rejects.toMatchObject({ message: 'disk full' })
        expect(actions.review).not.toBeNull()
        // No cover download after a failed save, and no toast: the dialog shows the error.
        expect(mockInvoke.callsTo('save_cover_from_url')).toHaveLength(0)
        expect(useNotificationsStore().items).toEqual([])
    })

    it('warns when only the cover failed, and still closes', async () => {
        const library = await openLibrary([book('a')])
        mockInvoke.on('update_metadata', ({ metadata }) => ({ ...book('a'), metadata }))
        mockInvoke.on('save_cover_from_url', () => backendError({ code: 'NETWORK', message: 'timeout' }))
        const actions = review(library.books[0])
        await actions.applyReview({ title: 'Dune' }, 'https://covers.example/1.jpg')
        expect(actions.review).toBeNull()
        expect(useNotificationsStore().items.map(n => n.type)).toEqual(['warning'])
    })
})

describe('book actions: metadata', () => {
    it('rejects with a readable message when part of a batch edit failed', async () => {
        await openLibrary([book('a'), book('b')])
        mockInvoke.on('batch_update_metadata', () => ({ books: [book('a')], failures: [{ bookId: 'b', error: 'locked' }] }))
        await expect(useBookActionsStore().saveMetadata(['a', 'b'], { series: 'S' }, { action: 'none', base64: null }))
            .rejects.toThrow('1 of 2 books could not be updated: locked')
    })
})

describe('book actions: between the library and the e-reader', () => {
    function connect(files: { filename: string, title?: string, author?: string, libraryId?: string }[]) {
        const device = useDeviceStore()
        device.devices = [{ id: 'k', name: 'Kindle', mountPath: '/Volumes/Kindle', deviceType: 'kindle', connection: 'massStorage', preferredFormat: 'azw3' }]
        device.selectedId = 'k'
        device.deviceBooks = files.map(f => ({ path: `/Volumes/Kindle/documents/${f.filename}`, title: '', author: '', ...f }))
        return device
    }

    it('removes every copy of a library book from the e-reader, after asking, and keeps the library book', async () => {
        const library = await openLibrary([book('a', 'Dune')])
        const device = connect([{ filename: 'Dune.azw3', libraryId: 'a' }, { filename: 'Dune.mobi', libraryId: 'a' }])
        mockInvoke.on('delete_device_book', () => undefined)
        const actions = useBookActionsStore()

        actions.requestRemoveFromDevice([library.shelfById.get('a')!])
        expect(actions.pendingDeviceRemoval?.map(f => f.filename)).toEqual(['Dune.azw3', 'Dune.mobi'])
        expect(actions.deviceRemovalTitle).toBe('Remove 2 books from Kindle?')
        expect(mockInvoke.callsTo('delete_device_book')).toHaveLength(0)

        await actions.confirmRemoveFromDevice()
        expect(mockInvoke.callsTo('delete_device_book').map(c => c.args.path)).toEqual([
            '/Volumes/Kindle/documents/Dune.azw3', '/Volumes/Kindle/documents/Dune.mobi',
        ])
        expect(actions.pendingDeviceRemoval).toBeNull()
        expect(device.deviceBooks).toEqual([])
        expect(library.books.map(b => b.id)).toEqual(['a'])
        expect(library.shelf.map(i => i.presence)).toEqual(['libraryOnly'])
        expect(useNotificationsStore().items.map(n => n.message)).toEqual(['Removed 2 books from Kindle.'])
    })

    it('keeps the dialog open with the reason, listing only the files still there', async () => {
        const library = await openLibrary([])
        connect([{ filename: 'a.epub', title: 'A', author: 'X' }, { filename: 'b.epub', title: 'B', author: 'Y' }])
        mockInvoke.on('delete_device_book', ({ path }) => String(path).endsWith('b.epub')
            ? backendError({ code: 'DEVICE', message: 'read-only' })
            : undefined)
        const actions = useBookActionsStore()
        actions.requestRemoveFromDevice(library.shelf)
        await actions.confirmRemoveFromDevice()
        expect(actions.pendingDeviceRemoval?.map(f => f.filename)).toEqual(['b.epub'])
        expect(actions.deviceRemovalError).toContain('read-only')
        expect(actions.deviceRemovalTitle).toBe('Remove “B” from Kindle?')
    })

    it('asks nothing for books that aren\'t on the e-reader', async () => {
        const library = await openLibrary([book('a')])
        connect([])
        const actions = useBookActionsStore()
        actions.requestRemoveFromDevice([library.shelfById.get('a')!])
        expect(actions.pendingDeviceRemoval).toBeNull()
    })

    it('adds a file only on the e-reader to the library; it then shows as one book in both places', async () => {
        const library = await openLibrary([])
        connect([{ filename: 'Germinal.epub', title: 'Germinal', author: 'Émile Zola' }])
        mockInvoke.on('import_books', () => [{ ...book('g', 'Germinal'), filename: 'Germinal.epub', metadata: { title: 'Germinal', author: 'Émile Zola', categories: [] } }])
        const actions = useBookActionsStore()

        const adding = actions.addToLibrary(library.shelf)
        expect(actions.addingToLibrary).toEqual(['/Volumes/Kindle/documents/Germinal.epub'])
        await adding
        expect(mockInvoke.callsTo('import_books')[0].args).toEqual({ paths: ['/Volumes/Kindle/documents/Germinal.epub'], libraryPath: '/lib' })
        expect(actions.addingToLibrary).toEqual([])
        expect(library.shelf.map(i => [i.id, i.presence])).toEqual([['g', 'both']])
    })
})

describe('book actions: favorites', () => {
    /** A deferred `set_favorite`: the test settles it. */
    function pendingSave() {
        let settle!: (books: Book[] | Error) => void
        mockInvoke.on('set_favorite', () => new Promise<Book[]>((resolve, reject) => {
            settle = v => (v instanceof Error ? reject(v) : resolve(v))
        }))
        return (v: Book[] | Error) => settle(v)
    }

    it('stars at once, moves the book up, and keeps what the backend saved', async () => {
        const library = await openLibrary([book('a', 'Anathem'), book('b', 'Brave New World'), book('c', 'Contact')])
        const settle = pendingSave()

        const done = useBookActionsStore().toggleFavorite([library.bookById.get('c')!])
        // Before the backend answers: starred, and first.
        expect(library.bookById.get('c')!.favorite).toBe(true)
        expect(library.visibleItems.map(i => i.id)).toEqual(['c', 'a', 'b'])
        expect(mockInvoke.callsTo('set_favorite')[0].args).toEqual({ libraryPath: '/lib', bookIds: ['c'], favorite: true })

        settle([{ ...book('c', 'Contact'), favorite: true }])
        await done
        expect(library.bookById.get('c')!.favorite).toBe(true)
        expect(useNotificationsStore().items).toEqual([])
    })

    it('unstars a selection only when every book is a favorite', async () => {
        const library = await openLibrary([{ ...book('a'), favorite: true }, book('b')])
        mockInvoke.on('set_favorite', ({ bookIds, favorite }) =>
            (bookIds as string[]).map(id => ({ ...book(id), favorite: favorite as boolean })))
        const actions = useBookActionsStore()

        await actions.toggleFavorite(library.books)
        expect(mockInvoke.callsTo('set_favorite')[0].args.favorite).toBe(true)
        expect(library.books.every(b => b.favorite)).toBe(true)

        await actions.toggleFavorite(library.books)
        expect(mockInvoke.callsTo('set_favorite')[1].args.favorite).toBe(false)
        expect(library.books.some(b => b.favorite)).toBe(false)
    })

    it('puts the star back and says so when it can’t be saved', async () => {
        const library = await openLibrary([book('a'), book('b')])
        const settle = pendingSave()
        const done = useBookActionsStore().toggleFavorite([library.bookById.get('b')!])
        expect(library.bookById.get('b')!.favorite).toBe(true)

        settle(new Error('disk full'))
        await done
        expect(library.bookById.get('b')!.favorite).toBeFalsy()
        expect(library.visibleItems.map(i => i.id)).toEqual(['a', 'b'])
        const [toast] = useNotificationsStore().items
        expect(toast.type).toBe('error')
        expect(toast.message).toContain('disk full')
    })
})
