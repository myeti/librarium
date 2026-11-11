import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, type VueWrapper } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@/utils/visibility', () => ({ watchVisibility: () => () => {} }))
import { mockInvoke } from '@/test/tauri'
import { cleanup, keydown, mountUi, testPinia } from '@/test/mountUi'
import { useLibraryStore } from '@/stores/library'
import LibraryGrid from './LibraryGrid.vue'
import type { ShelfItem } from '@/services/shelf'
import type { Book } from '@/types'

// happy-dom has no layout: give the scroller a size (1000 × 800 → 4 columns
// of cards about 216 × 438 px with GRID_LAYOUT).
const WIDTH = 1000
const HEIGHT = 800
const descriptors = {
    clientWidth: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth'),
    clientHeight: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight'),
}
beforeAll(() => {
    Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
        configurable: true,
        get(this: HTMLElement) { return this.classList.contains('book-collection') ? WIDTH : 0 },
    })
    Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
        configurable: true,
        get(this: HTMLElement) { return this.classList.contains('book-collection') ? HEIGHT : 0 },
    })
})
afterAll(() => {
    if (descriptors.clientWidth) { Object.defineProperty(HTMLElement.prototype, 'clientWidth', descriptors.clientWidth) }
    if (descriptors.clientHeight) { Object.defineProperty(HTMLElement.prototype, 'clientHeight', descriptors.clientHeight) }
})

function book(i: number): Book {
    const id = `b${String(i).padStart(3, '0')}`
    return {
        id, path: `/lib/${id}.epub`, filename: `${id}.epub`, format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01',
        metadata: { title: `Book ${i}`, author: 'Author', categories: [] },
    }
}

/** Mounts the grid over a library of `count` books, with v-model:activeId wired like the view does. */
async function mountGrid(count: number) {
    testPinia()
    const books = Array.from({ length: count }, (_, i) => book(i))
    mockInvoke.on('scan_library', () => books)
    const library = useLibraryStore()
    await library.openLibrary('/lib')
    const opened: string[] = []
    const menus: string[] = []
    const wrapper: VueWrapper = mountUi(LibraryGrid, {
        props: {
            items: library.visibleItems,
            activeId: null,
            'onUpdate:activeId': (id: string | null) => { void wrapper.setProps({ activeId: id }) },
            onOpen: (item: ShelfItem) => opened.push(item.id),
            onMenu: (r: { item: ShelfItem }) => menus.push(r.item.id),
        },
    })
    await flushPromises()
    return { wrapper, library, opened, menus }
}

/** The mounted grid's root element (VTU types it loosely). */
function root(wrapper: VueWrapper): HTMLElement {
    return wrapper.element as HTMLElement
}

function options(wrapper: VueWrapper): HTMLElement[] {
    return Array.from(root(wrapper).querySelectorAll<HTMLElement>('[role="option"]'))
}

function option(wrapper: VueWrapper, index: number): HTMLElement {
    const el = root(wrapper).querySelector<HTMLElement>(`[role="option"][data-index="${index}"]`)
    if (!el) { throw new Error(`book ${index} is not rendered`) }
    return el
}

/** The grid's current `activeId` prop (kept in sync by the v-model wiring). */
function activeIdOf(wrapper: VueWrapper): unknown {
    return (wrapper.props() as Record<string, unknown>).activeId
}

function focusedIndex(): number | null {
    const el = document.activeElement as HTMLElement | null
    return el?.dataset.index !== undefined ? Number(el.dataset.index) : null
}

/** Presses a key on the focused book (or `target`) and lets Vue render. */
async function press(key: string, init: KeyboardEventInit = {}, target?: Element) {
    const event = keydown(target ?? document.activeElement ?? document.body, key, init)
    await flushPromises()
    return event
}

beforeEach(() => { mockInvoke.reset() })
afterEach(() => { cleanup() })

describe('BookCollection: virtual window', () => {
    it('renders only the rows on screen (plus overscan), however many books there are', async () => {
        const { wrapper } = await mountGrid(2000)
        const rendered = options(wrapper)
        // 800 px shows 2 rows of ~468 px; with 2 rows of overscan below: 4 rows of 4.
        expect(rendered.length).toBeGreaterThan(0)
        expect(rendered.length).toBeLessThanOrEqual(5 * 4)
        const canvas = root(wrapper).querySelector<HTMLElement>('[role="listbox"]')!
        expect(parseFloat(canvas.style.height)).toBeGreaterThan(500 * 400) // 500 rows tall
    })

    it('exposes the books as a multi-select listbox, each with its position in the whole list', async () => {
        const { wrapper } = await mountGrid(2000)
        const listbox = root(wrapper).querySelector('[role="listbox"]')!
        expect(listbox.getAttribute('aria-multiselectable')).toBe('true')
        expect(listbox.getAttribute('aria-label')).toBe('Books')
        const first = options(wrapper)[0]
        expect(first.getAttribute('aria-setsize')).toBe('2000')
        expect(first.getAttribute('aria-posinset')).toBe('1')
        expect(first.getAttribute('aria-selected')).toBe('false')
        expect(first.getAttribute('aria-label')).toBe('Book 0, Author')
    })
})

describe('BookCollection: keyboard', () => {
    it('has exactly one Tab stop, on the first book until one is active', async () => {
        const { wrapper } = await mountGrid(30)
        const stops = options(wrapper).filter(o => o.getAttribute('tabindex') === '0')
        expect(stops.map(o => o.dataset.index)).toEqual(['0'])
    })

    it('moves through the grid with the arrows (rows of 4), Home and End', async () => {
        const { wrapper } = await mountGrid(30)
        option(wrapper, 0).focus()
        await press('ArrowRight')
        expect(focusedIndex()).toBe(1)
        await press('ArrowDown')
        expect(focusedIndex()).toBe(5)
        await press('ArrowLeft')
        await press('ArrowUp')
        expect(focusedIndex()).toBe(0)
        await press('End')
        expect(focusedIndex()).toBe(29)
        expect(activeIdOf(wrapper)).toBe('b029')
        await press('Home')
        expect(focusedIndex()).toBe(0)
        // The Tab stop followed.
        expect(option(wrapper, 0).getAttribute('tabindex')).toBe('0')
    })

    it('keeps the active book rendered when it is scrolled far away', async () => {
        const { wrapper } = await mountGrid(2000)
        option(wrapper, 0).focus()
        await press('End')
        expect(focusedIndex()).toBe(1999)
        expect(option(wrapper, 1999).getAttribute('tabindex')).toBe('0')
    })

    it('toggles with Space, extends with Shift+arrows (and shrinks back)', async () => {
        const { wrapper, library } = await mountGrid(30)
        option(wrapper, 2).focus()
        await press(' ')
        expect([...library.selectedIds]).toEqual(['b002'])
        expect(option(wrapper, 2).getAttribute('aria-selected')).toBe('true')
        await press('ArrowRight', { shiftKey: true })
        await press('ArrowRight', { shiftKey: true })
        expect([...library.selectedIds].sort()).toEqual(['b002', 'b003', 'b004'])
        await press('ArrowLeft', { shiftKey: true })
        expect([...library.selectedIds].sort()).toEqual(['b002', 'b003'])
        await press('ArrowDown', { shiftKey: true }) // a whole row: from 3 to 7
        expect([...library.selectedIds].sort()).toEqual(['b002', 'b003', 'b004', 'b005', 'b006', 'b007'])
        await press(' ')
        expect(library.selectedIds.has('b007')).toBe(false)
    })

    it('opens the book on Enter and its menu on the ContextMenu key or Shift+F10', async () => {
        const { wrapper, opened, menus } = await mountGrid(30)
        option(wrapper, 0).focus()
        await press('ArrowRight')
        await press('Enter')
        expect(opened).toEqual(['b001'])
        await press('ContextMenu')
        await press('F10', { shiftKey: true })
        expect(menus).toEqual(['b001', 'b001'])
    })

    it('leaves global shortcuts alone (⌘A, Delete, Escape)', async () => {
        const { wrapper } = await mountGrid(30)
        option(wrapper, 0).focus()
        expect((await press('a', { metaKey: true })).defaultPrevented).toBe(false)
        expect((await press('Delete')).defaultPrevented).toBe(false)
        expect((await press('Escape')).defaultPrevented).toBe(false)
        // …while the keys it uses are taken (the global ← → don't also run).
        expect((await press('ArrowRight')).defaultPrevented).toBe(true)
    })
})

describe('BookCollection: mouse', () => {
    it('opens a book on click and makes it the active one', async () => {
        const { wrapper, opened } = await mountGrid(30)
        option(wrapper, 3).querySelector<HTMLElement>('.book-card__title')!.click()
        await flushPromises()
        expect(opened).toEqual(['b003'])
        expect(activeIdOf(wrapper)).toBe('b003')
    })

    it('toggles with Ctrl-click (⌘-click on macOS) and selects a range with Shift-click', async () => {
        const { wrapper, library, opened } = await mountGrid(30)
        const click = (index: number, init: MouseEventInit) => {
            option(wrapper, index).dispatchEvent(new MouseEvent('click', { bubbles: true, ...init }))
            return flushPromises()
        }
        // happy-dom isn't macOS: the command key is Ctrl.
        await click(1, { ctrlKey: true })
        await click(4, { ctrlKey: true })
        expect([...library.selectedIds].sort()).toEqual(['b001', 'b004'])
        await click(6, { shiftKey: true }) // from the anchor (4)
        expect([...library.selectedIds].sort()).toEqual(['b001', 'b004', 'b005', 'b006'])
        expect(opened).toEqual([])
    })

    it('toggles a book with its checkbox without opening it', async () => {
        const { wrapper, library, opened } = await mountGrid(30)
        const input = option(wrapper, 2).querySelector<HTMLInputElement>('input[type="checkbox"]')!
        expect(input.getAttribute('tabindex')).toBe('-1')
        input.click()
        await flushPromises()
        expect([...library.selectedIds]).toEqual(['b002'])
        expect(opened).toEqual([])
        expect(focusedIndex()).toBe(2) // keyboard focus is back on the book
    })

    it('asks for the menu on right-click', async () => {
        const { wrapper, menus } = await mountGrid(30)
        const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 50, clientY: 60 })
        option(wrapper, 5).dispatchEvent(event)
        await flushPromises()
        expect(event.defaultPrevented).toBe(true)
        expect(menus).toEqual(['b005'])
    })
})
