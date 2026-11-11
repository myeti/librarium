import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, type VueWrapper } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(() => Promise.resolve(null)), save: vi.fn(() => Promise.resolve(null)) }))
vi.mock('@/utils/visibility', () => ({ watchVisibility: () => () => {} }))
import { backendError, mockInvoke } from '@/test/tauri'
import { cleanup, keydown, mountUi, testPinia } from '@/test/mountUi'
import { useLibraryStore } from '@/stores/library'
import { useShortcutsStore } from '@/stores/shortcuts'
import { useDeviceStore } from '@/stores/device'
import { useBookActionsStore } from '@/stores/bookActions'
import { useNotificationsStore } from '@/stores/notifications'
import LibraryView from './LibraryView.vue'
import type { Book } from '@/types'

// happy-dom has no layout: the collection's scroller gets a size (4 columns).
const sizes = {
    clientWidth: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth'),
    clientHeight: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight'),
}
beforeAll(() => {
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
        configurable: true,
        get(this: HTMLElement) { return this.classList.contains('book-collection') ? 1000 : 0 },
    })
    Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
        configurable: true,
        get(this: HTMLElement) { return this.classList.contains('book-collection') ? 800 : 0 },
    })
})
afterAll(() => {
    if (sizes.clientWidth) { Object.defineProperty(HTMLElement.prototype, 'clientWidth', sizes.clientWidth) }
    if (sizes.clientHeight) { Object.defineProperty(HTMLElement.prototype, 'clientHeight', sizes.clientHeight) }
})

function book(i: number, title = `Book ${i}`): Book {
    const id = `b${i}`
    return {
        id, path: `/lib/${id}.epub`, filename: `${id}.epub`, format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01',
        metadata: { title, author: 'Author', categories: [] },
    }
}

async function mountView(count = 12) {
    testPinia()
    const shortcuts = useShortcutsStore()
    shortcuts.install(window)
    const books = Array.from({ length: count }, (_, i) => book(i))
    mockInvoke.on('scan_library', () => books)
    mockInvoke.on('ensure_cover', () => null)
    const library = useLibraryStore()
    library.setViewMode('grid')
    await library.openLibrary('/lib')
    const view: VueWrapper = mountUi(LibraryView)
    await flushPromises()
    return { view, library, shortcuts }
}

function root(view: VueWrapper): HTMLElement {
    return view.element as HTMLElement
}

function option(view: VueWrapper, index: number): HTMLElement {
    return root(view).querySelector<HTMLElement>(`[role="option"][data-index="${index}"]`)!
}

/** The details dialog (teleported to <body>), if open. */
function details(): HTMLElement | null {
    return document.querySelector<HTMLElement>('.book-detail')
}

function detailsTitle(): string | null {
    return details()?.querySelector('h2')?.textContent?.trim() ?? null
}

async function press(key: string, init: KeyboardEventInit = {}) {
    keydown(document.activeElement ?? document.body, key, init)
    await flushPromises()
}

beforeEach(() => {
    mockInvoke.reset()
})

afterEach(() => {
    useShortcutsStore().reset()
    cleanup()
    vi.unstubAllGlobals()
})

describe('LibraryView: details and keyboard', () => {
    it('opens the focused book in a dialog over the books with Enter; ← → show the previous / next book', async () => {
        const { view } = await mountView()
        option(view, 0).focus()
        await press('Enter')
        expect(detailsTitle()).toBe('Book 0')
        // A dialog: the books don't share the screen with it.
        expect(details()!.closest('dialog')).not.toBeNull()
        expect(root(view).querySelector('.library-panel')).toBeNull()
        // Focus starts on the title, inside the dialog.
        expect(document.activeElement?.textContent?.trim()).toBe('Book 0')
        await press('ArrowRight')
        expect(detailsTitle()).toBe('Book 1')
        await press('ArrowRight')
        await press('ArrowLeft')
        expect(detailsTitle()).toBe('Book 1')
        expect(details()!.textContent).toContain('2 / 12')
    })

    it('has previous / next buttons, off at the ends', async () => {
        const { view } = await mountView(3)
        option(view, 2).click()
        await flushPromises()
        const button = (label: string) => details()!.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!
        expect(button('Next book').disabled).toBe(true)
        button('Previous book').click()
        await flushPromises()
        expect(detailsTitle()).toBe('Book 1')
        button('Previous book').click()
        await flushPromises()
        expect(button('Previous book').disabled).toBe(true)
    })

    it('Escape closes the details, and focus goes back to the last book shown', async () => {
        const { view } = await mountView()
        option(view, 0).focus()
        await press('Enter')
        await press('ArrowRight')
        await press('Escape')
        await new Promise(r => setTimeout(r, 0))
        await flushPromises()
        expect(details()).toBeNull()
        expect((document.activeElement as HTMLElement).dataset.index).toBe('1')
    })

    it('Escape clears the selection', async () => {
        const { view, library } = await mountView()
        option(view, 0).focus()
        await press(' ')
        expect(library.selectedIds.size).toBe(1)
        await press('Escape')
        expect(library.selectedIds.size).toBe(0)
    })

    it('swaps the filter bar for the selection toolbar while books are selected', async () => {
        const { view, library } = await mountView()
        expect(root(view).querySelector('.filter-bar')).not.toBeNull()
        library.toggleSelect('b1')
        await flushPromises()
        expect(root(view).querySelector('.filter-bar')).toBeNull()
        expect(root(view).querySelector('.selection-toolbar')?.textContent).toContain('1 selected')
        library.clearSelection()
        await flushPromises()
        expect(root(view).querySelector('.filter-bar')).not.toBeNull()
    })

    it('⌘A (Ctrl+A) selects every book shown, and again none', async () => {
        const { view, library } = await mountView()
        option(view, 0).focus()
        await press('a', { ctrlKey: true })
        expect(library.selectedIds.size).toBe(12)
        await press('a', { ctrlKey: true })
        expect(library.selectedIds.size).toBe(0)
    })

    it('Delete on a focused book asks to move that book to the Trash', async () => {
        const { view } = await mountView()
        option(view, 3).focus()
        await press('Delete')
        const dialog = document.querySelector('dialog')
        expect(dialog?.textContent).toContain('Move “Book 3” to the Trash?')
        expect(mockInvoke.callsTo('delete_books')).toHaveLength(0)
    })

    it('registers its shortcuts while shown, and only then', async () => {
        const { view, shortcuts } = await mountView()
        expect(shortcuts.active).toEqual(expect.arrayContaining([
            'library.selectAll', 'library.delete', 'library.close', 'library.search',
            'view.toggleDetails', 'library.export',
        ]))
        view.unmount()
        expect(shortcuts.active.filter(id => id.startsWith('library.') || id.startsWith('view.'))).toEqual([])
    })

    it('⌘I (or the menu) opens the details of the selected book, and closes them', async () => {
        const { view, shortcuts } = await mountView()
        option(view, 2).focus()
        await press(' ')
        expect(shortcuts.runCommand('view.toggleDetails', 'menu')).toBe(true)
        await flushPromises()
        expect(detailsTitle()).toBe('Book 2')
        // Over the details (the only dialog), it closes them.
        expect(shortcuts.runCommand('view.toggleDetails', 'menu')).toBe(true)
        await flushPromises()
        expect(detailsTitle()).toBeNull()
    })

    it('Delete in the details asks to move the book to the Trash', async () => {
        const { view } = await mountView()
        option(view, 4).focus()
        await press('Enter')
        await press('Delete')
        expect(useBookActionsStore().pendingDelete).toEqual({ ids: ['b4'], origin: 'single' })
        // ⌘I doesn't reach under the confirmation.
        expect(useShortcutsStore().runCommand('view.toggleDetails', 'menu')).toBe(false)
        expect(detailsTitle()).toBe('Book 4')
    })
})

describe('LibraryView: a library that can’t be opened', () => {
    it('shows a permission refusal with the backend’s message (where to allow it), not "is the NAS connected?"', async () => {
        testPinia()
        const message = 'Librarium isn’t allowed to open the library folder "Alice". In System Settings › Privacy & Security › Files and Folders, allow Librarium to access Network Volumes…'
        mockInvoke.on('scan_library', () => backendError({ code: 'PERMISSION_DENIED', message }))
        await useLibraryStore().openLibrary('/Volumes/books/Alice')
        const view = mountUi(LibraryView)
        await flushPromises()
        const text = root(view).textContent ?? ''
        expect(text).toContain(message)
        expect(text).not.toContain('Is the drive or NAS connected')
    })
})

describe('LibraryView: the e-reader\'s books in the library', () => {
    function connect(files: { filename: string, title?: string, author?: string, libraryId?: string }[]) {
        const device = useDeviceStore()
        device.devices = [{ id: 'k', name: 'Kindle', mountPath: '/Volumes/Kindle', deviceType: 'kindle', connection: 'massStorage', preferredFormat: 'azw3' }]
        device.selectedId = 'k'
        device.deviceBooks = files.map(f => ({ path: `/Volumes/Kindle/documents/${f.filename}`, title: '', author: '', ...f }))
        return device
    }

    afterEach(cleanup)

    it('opens on the device\'s books from /library?scope=device, then clears the address', async () => {
        testPinia()
        mockInvoke.on('scan_library', () => [book(0)])
        const library = useLibraryStore()
        await library.openLibrary('/lib')
        connect([{ filename: 'x.azw3', libraryId: 'b0' }])
        const replace = vi.fn(() => Promise.resolve())
        mountUi(LibraryView, { mocks: { $route: { query: { scope: 'device' } }, $router: { replace } } })
        await flushPromises()
        expect(library.scope).toBe('device')
        expect(replace).toHaveBeenCalledWith({ query: {} })
    })

    it('shows everything again, and says why, when the device goes away', async () => {
        const { library } = await mountView(2)
        const device = connect([])
        library.setScope('device')
        await flushPromises()
        device.devices = []
        await flushPromises()
        expect(library.scope).toBe('all')
        expect(useNotificationsStore().items.map(n => n.message)).toEqual(['The e-reader was disconnected: all your books are shown again.'])
    })

    it('shows a file only on the e-reader in the details too, and Delete offers to remove it from the device', async () => {
        const { view, library } = await mountView(0)
        connect([{ filename: 'Germinal.epub', title: 'Germinal', author: 'Émile Zola' }])
        await flushPromises()
        expect(library.visibleItems).toHaveLength(1)
        option(view, 0).click()
        await flushPromises()
        expect(detailsTitle()).toBe('Germinal')
        expect(details()!.textContent).toContain('Add to library')

        await press('Delete')
        expect(useBookActionsStore().pendingDeviceRemoval?.map(f => f.filename)).toEqual(['Germinal.epub'])
    })
})
