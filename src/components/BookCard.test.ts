import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
// The shared IntersectionObserver, replaced by a switch the test flips.
const visibility = vi.hoisted(() => new Map<Element, (visible: boolean) => void>())
vi.mock('@/utils/visibility', () => ({
    watchVisibility: (el: Element, callback: (visible: boolean) => void) => {
        visibility.set(el, callback)
        return () => visibility.delete(el)
    },
}))
import { mockInvoke } from '@/test/tauri'
import BookCard from './BookCard.vue'
import { useLibraryStore } from '@/stores/library'
import { i18nPlugin } from '@/i18n'
import { deviceItemId, type ShelfItem } from '@/services/shelf'
import type { Book, DeviceBook } from '@/types'

function book(id: string): Book {
    return {
        id, path: `/lib/${id}.epub`, filename: `${id}.epub`, format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01',
        metadata: { title: `Title ${id}`, author: 'Author', categories: [] },
    }
}

/** Opens a library holding `books` and mounts one card per book. */
async function mountCards(books: Book[]) {
    const pinia = createPinia()
    setActivePinia(pinia)
    mockInvoke.on('scan_library', () => books)
    const store = useLibraryStore()
    await store.openLibrary('/lib')
    const cards = store.books.map(b => mountItem(store.shelfById.get(b.id)!, pinia))
    return { store, cards }
}

function mountItem(item: ShelfItem, pinia = createPinia()) {
    return mount(BookCard, {
        props: { item, deviceName: 'Kindle' },
        global: { plugins: [pinia, i18nPlugin], stubs: { 'vue-feather': true } },
    })
}

function onDevice(filename: string, title = '', author = ''): DeviceBook {
    return { path: `/Volumes/Kindle/documents/${filename}`, filename, title, author }
}

/** The cover of a mounted card: the element whose visibility is watched. */
function elementOf(card: { find(selector: string): { element: unknown } }): Element {
    return card.find('.book-cover').element as Element
}

function setVisible(el: Element, visible: boolean) {
    visibility.get(el)?.(visible)
}

beforeEach(() => {
    mockInvoke.reset()
    visibility.clear()
})

describe('BookCard covers', () => {
    it('asks for its cover only once visible, then shows it from the asset protocol', async () => {
        const { store, cards: [card] } = await mountCards([book('a')])
        mockInvoke.on('ensure_cover', () => ({ coverPath: '/lib/.covers/thumbs/a.jpg', coverVersion: 42 }))
        expect(card.find('img').exists()).toBe(false)
        expect(mockInvoke.callsTo('ensure_cover')).toHaveLength(0)

        setVisible(elementOf(card), true)
        await store.coverQueue.whenIdle()
        await flushPromises()

        const img = card.find('img')
        expect(img.attributes('src')).toBe(`asset://localhost/${encodeURIComponent('/lib/.covers/thumbs/a.jpg')}?v=42`)
        expect(img.attributes('loading')).toBe('lazy')
        expect(img.attributes('decoding')).toBe('async')
        expect(visibility.has(elementOf(card)), 'no more watching once it has a cover').toBe(false)
    })

    it('withdraws its request when it scrolls away before the request starts', async () => {
        const ids = ['a', 'b', 'c', 'd', 'e']
        const { store, cards } = await mountCards(ids.map(book))
        let release!: () => void
        const slow = new Promise(r => { release = () => r(null) })
        mockInvoke.on('ensure_cover', () => slow)

        cards.forEach(c => setVisible(elementOf(c), true)) // 4 start, 'e' waits
        setVisible(elementOf(cards[4]), false)
        release()
        await store.coverQueue.whenIdle()
        expect(mockInvoke.callsTo('ensure_cover').map(c => c.args.bookId)).toEqual(['a', 'b', 'c', 'd'])
    })

    it('stops watching and withdraws its request when unmounted', async () => {
        const ids = ['a', 'b', 'c', 'd', 'e']
        const { store, cards } = await mountCards(ids.map(book))
        let release!: () => void
        const slow = new Promise(r => { release = () => r(null) })
        mockInvoke.on('ensure_cover', () => slow)

        cards.forEach(c => setVisible(elementOf(c), true))
        const lastElement = elementOf(cards[4])
        cards[4].unmount() // filtered out of the grid
        expect(visibility.has(lastElement)).toBe(false)
        release()
        await store.coverQueue.whenIdle()
        expect(mockInvoke.callsTo('ensure_cover')).toHaveLength(4)
    })
})

describe('BookCard text', () => {
    /** Mounts one card for `metadata` (cover requests are irrelevant here). */
    async function cardFor(metadata: Partial<Book['metadata']>, props: Record<string, unknown> = {}) {
        const b: Book = { ...book('x'), metadata: { ...book('x').metadata, ...metadata } }
        const { cards: [card] } = await mountCards([b])
        await card.setProps(props)
        return card
    }

    it('always shows the title and author under the cover, with the full text as a tooltip', async () => {
        const long = 'A Very Long Title That Keeps Going To Check How Cards Truncate Their Text Gracefully'
        const card = await cardFor({ title: long, author: 'Someone With A Long Name & A Co-Author' })
        const title = card.find('.book-card__title')
        expect(title.text()).toBe(long)
        expect(title.attributes('title')).toBe(long)
        const author = card.find('.book-card__author')
        expect(author.classes()).toContain('truncate')
        expect(author.attributes('title')).toBe('Someone With A Long Name & A Co-Author')
    })

    it('reads "Unknown title" / "Unknown author" for empty values', async () => {
        const card = await cardFor({ title: '', author: '  ' })
        expect(card.find('.book-card__title').text()).toBe('Unknown title')
        expect(card.find('.book-card__author').text()).toBe('Unknown author')
    })

    it('shows the series with its volume, and never "Saga #" without one', async () => {
        expect((await cardFor({ series: 'Dune', seriesIndex: 2 })).find('.book-card__series').text()).toBe('Dune #2')
        expect((await cardFor({ series: 'Saga' })).find('.book-card__series').text()).toBe('Saga')
        expect((await cardFor({})).find('.book-card__series').exists()).toBe(false)
    })

    it('has a real, labelled checkbox reflecting the selection', async () => {
        const card = await cardFor({ title: 'Dune' }, { selected: true })
        const input = card.find('input[type="checkbox"]')
        expect((input.element as HTMLInputElement).checked).toBe(true)
        expect(input.attributes('aria-label')).toBe('Select “Dune”')
    })

    it('shows the reading progress as a label, only when the device reports some (no bar)', async () => {
        const card = await cardFor({})
        expect(card.find('.book-card__percent').exists()).toBe(false)

        const item = card.props('item') as ShelfItem
        await card.setProps({ item: { ...item, presence: 'both', copies: [onDevice('x.azw3')], progress: 42 } })
        expect(card.find('.book-card__percent').text()).toBe('42%')
        expect(card.find('[role="progressbar"]').exists()).toBe(false)

        // Never opened: the label says so.
        await card.setProps({ item: { ...item, presence: 'both', copies: [onDevice('x.azw3')], progress: 0 } })
        expect(card.find('.book-card__percent').text()).toBe('0%')
    })
})

describe('BookCard location', () => {
    it('marks nothing on a book only in the library', async () => {
        const { cards: [card] } = await mountCards([book('a')])
        expect(card.find('.book-card__where').exists()).toBe(false)
    })

    it('shows a book and a tablet for a book in both places, and says so', async () => {
        const { cards: [card] } = await mountCards([book('a')])
        const item = card.props('item') as ShelfItem
        await card.setProps({ item: { ...item, presence: 'both', copies: [onDevice('a.azw3')] } })
        const where = card.find('.book-card__where')
        expect(where.findAll('vue-feather-stub').map(i => i.attributes('type'))).toEqual(['book', 'tablet'])
        expect(where.text()).toBe('In the library and on Kindle')
    })

    it('shows a file only on the e-reader with a tablet and a generated cover from what the device says', () => {
        const file = onDevice('Germinal.epub', 'Germinal', 'Émile Zola')
        const card = mountItem({ id: deviceItemId(file), book: null, copies: [file], presence: 'deviceOnly', progress: 30 })
        const where = card.find('.book-card__where')
        expect(where.findAll('vue-feather-stub').map(i => i.attributes('type'))).toEqual(['tablet'])
        expect(where.text()).toBe('Only on Kindle')
        expect(card.find('.cover-gen-title').text()).toBe('Germinal')
        expect(card.find('.book-card__title').text()).toBe('Germinal')
        expect(card.find('.book-card__author').text()).toBe('Émile Zola')
        expect(card.find('.book-card__percent').text()).toBe('30%')
        expect(mockInvoke.callsTo('ensure_cover')).toHaveLength(0)
    })
})
