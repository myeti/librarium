import { describe, expect, it } from 'vitest'
import type { Book, DeviceBook, FilterConfig, SortConfig } from '@/types'
import { DeviceIndex } from '@/services/deviceMatch'
import {
    buildShelf, deviceItemId, fileFormat, fileOf, isDeviceItemId, itemAuthor, itemTitle, visibleShelf,
    type ShelfItem,
} from './shelf'

function book(id: string, title: string, author = 'Frank Herbert', extra: Partial<Book['metadata']> & { filename?: string, dateAdded?: string } = {}): Book {
    const { filename, dateAdded, ...metadata } = extra
    return {
        id, path: `/lib/${id}.epub`, filename: filename ?? `${title}.epub`, format: 'epub', sizeBytes: 100,
        dateAdded: dateAdded ?? '2026-01-01T00:00:00Z', metadata: { title, author, categories: [], ...metadata },
    }
}

function file(filename: string, title = '', author = '', extra: Partial<DeviceBook> = {}): DeviceBook {
    return { path: `/Volumes/Kindle/documents/${filename}`, filename, title, author, sizeBytes: 50, ...extra }
}

const NONE: FilterConfig = { search: '', formats: [], authors: [], series: [], languages: [], categories: [], presence: [] }
const BY_TITLE: SortConfig = { field: 'title', order: 'asc' }
const t = (key: string) => key === 'book.unknownAuthor' ? 'Unknown author' : key

const ids = (items: ShelfItem[]) => items.map(i => i.id)

describe('buildShelf', () => {
    it('is the library alone without a device', () => {
        const shelf = buildShelf([book('a', 'Dune'), book('b', 'Emma')], [], {})
        expect(shelf.map(i => [i.id, i.presence])).toEqual([['a', 'libraryOnly'], ['b', 'libraryOnly']])
    })

    it('matches a copy Librarium sent by its id, whatever its title', () => {
        const shelf = buildShelf([book('a', 'Dune')], [file('Renamed.azw3', 'Something else', 'X', { libraryId: 'a' })], {})
        expect(shelf).toHaveLength(1)
        expect(shelf[0]).toMatchObject({ id: 'a', presence: 'both' })
        expect(shelf[0].copies.map(c => c.filename)).toEqual(['Renamed.azw3'])
    })

    it('matches other files by title and author, or by file name; the rest is device-only', () => {
        const shelf = buildShelf(
            [book('a', 'Dune'), book('b', 'Emma', 'Jane Austen', { filename: 'emma-austen.epub' })],
            [
                file('dune.mobi', 'DUNE', 'Herbert, Frank'),       // same title and author (sort form)
                file('emma-austen.azw3', 'Emma (ebook)', 'J. A.'),  // same file name
                file('other.pdf', 'Other', 'Someone'),              // nothing
                file('dune2.mobi', 'Dune', 'Somebody else'),        // same title only: not the same book
            ],
            {},
        )
        expect(shelf.map(i => [i.id, i.presence])).toEqual([
            ['a', 'both'],
            ['b', 'both'],
            [deviceItemId(file('other.pdf')), 'deviceOnly'],
            [deviceItemId(file('dune2.mobi')), 'deviceOnly'],
        ])
    })

    it('treats a copy from another user\'s library like any other file', () => {
        const shelf = buildShelf([book('a', 'Dune')], [file('x.epub', 'Dune', 'Frank Herbert', { libraryId: 'someone-elses' })], {})
        expect(shelf.map(i => i.presence)).toEqual(['both'])
        const alone = buildShelf([book('a', 'Emma', 'Jane Austen')], [file('x.epub', 'Dune', 'Frank Herbert', { libraryId: 'someone-elses' })], {})
        expect(alone.map(i => i.presence)).toEqual(['libraryOnly', 'deviceOnly'])
    })

    it('agrees with the "already on device" marks', () => {
        const books = [book('a', 'Dune'), book('b', 'Emma', 'Jane Austen'), book('c', 'Ulysses', 'James Joyce', { filename: 'u.epub' })]
        const files = [file('dune.mobi', 'Dune', 'Frank Herbert'), file('u.azw3', 'U', 'J'), file('z.pdf', 'Z', 'Y', { libraryId: 'b' })]
        const index = new DeviceIndex(files, new Set(books.map(b => b.id)))
        for (const item of buildShelf(books, files, {})) {
            if (item.book) { expect(item.presence === 'both', item.id).toBe(index.has(item.book)) }
        }
    })

    it('gives an item its progress on the device: the furthest of its copies, Kobo\'s "finished" as 100', () => {
        const shelf = buildShelf(
            [book('a', 'Dune'), book('b', 'Emma', 'Jane Austen')],
            [
                file('Dune.mobi', 'Dune', 'Frank Herbert'),
                file('Dune.azw3', 'x', 'x', { libraryId: 'a' }),
                file('Emma.kepub.epub', 'x', 'x', { libraryId: 'b', readStatus: 'finished' }),
                file('Other.epub', 'Other', 'Someone'),
            ],
            { 'Dune': 12, 'Dune.mobi': 0, 'Other': 40 },
        )
        expect(shelf.map(i => i.progress)).toEqual([12, 100, 40])
    })
})

describe('device-only items', () => {
    const lone = buildShelf([], [file('Some Book.kepub.epub', '', '')], {})[0]

    it('are told apart from library books by their id', () => {
        expect(isDeviceItemId(lone.id)).toBe(true)
        expect(isDeviceItemId('3f2b1c3e-6e0a-4c1b-9a43-3f1f3c0f9b11')).toBe(false)
        expect(fileOf(lone)?.filename).toBe('Some Book.kepub.epub')
    })

    it('show their file name when the device gives no title', () => {
        expect(itemTitle(lone, t)).toBe('Some Book.kepub')
        expect(itemAuthor(lone, t)).toBe('Unknown author')
    })

    it('get their format from the extension', () => {
        expect(fileFormat('a.kepub.epub')).toBe('epub')
        expect(fileFormat('a.AZW')).toBe('azw3')
        expect(fileFormat('a.prc')).toBe('mobi')
        expect(fileFormat('a.txt')).toBe('unknown')
    })
})

describe('visibleShelf', () => {
    const books = [
        book('a', 'Dune', 'Frank Herbert', { series: 'Dune', seriesIndex: 1, language: 'en', dateAdded: '2026-03-01T00:00:00Z' }),
        book('b', 'Emma', 'Jane Austen', { language: 'en', categories: ['Classics'] }),
    ]
    const files = [
        file('Dune.azw3', 'Dune', 'Frank Herbert'),
        file('zola.epub', 'Germinal', 'Émile Zola', { sizeBytes: 999 }),
        file('ann.pdf', 'Anna', 'Leo Tolstoy'),
    ]
    const shelf = buildShelf(books, files, { 'zola': 30 })
    const zola = deviceItemId(files[1])
    const anna = deviceItemId(files[2])

    it('shows everything, the library\'s books, or the device\'s', () => {
        expect(ids(visibleShelf(shelf, 'all', NONE, BY_TITLE))).toEqual([anna, 'a', 'b', zola])
        expect(ids(visibleShelf(shelf, 'library', NONE, BY_TITLE))).toEqual(['a', 'b'])
        expect(ids(visibleShelf(shelf, 'device', NONE, BY_TITLE))).toEqual([anna, 'a', zola])
    })

    it('filters by where a book is', () => {
        expect(ids(visibleShelf(shelf, 'all', { ...NONE, presence: ['libraryOnly'] }, BY_TITLE))).toEqual(['b'])
        expect(ids(visibleShelf(shelf, 'all', { ...NONE, presence: ['deviceOnly'] }, BY_TITLE))).toEqual([anna, zola])
    })

    it('searches device files by title, author and file name, ignoring accents', () => {
        expect(ids(visibleShelf(shelf, 'all', { ...NONE, search: 'emile' }, BY_TITLE))).toEqual([zola])
        expect(ids(visibleShelf(shelf, 'all', { ...NONE, search: 'ann.pdf' }, BY_TITLE))).toEqual([anna])
    })

    it('leaves device files out of filters on what they don\'t have', () => {
        expect(ids(visibleShelf(shelf, 'all', { ...NONE, languages: ['en'] }, BY_TITLE))).toEqual(['a', 'b'])
        expect(ids(visibleShelf(shelf, 'all', { ...NONE, categories: ['Classics'] }, BY_TITLE))).toEqual(['b'])
        expect(ids(visibleShelf(shelf, 'all', { ...NONE, formats: ['pdf'] }, BY_TITLE))).toEqual([anna])
        expect(ids(visibleShelf(shelf, 'all', { ...NONE, authors: ['Émile Zola'] }, BY_TITLE))).toEqual([zola])
    })

    it('puts unknown values last in both directions', () => {
        const byDate = (order: 'asc' | 'desc') => ids(visibleShelf(shelf, 'all', NONE, { field: 'dateAdded', order }))
        expect(byDate('asc').slice(0, 2)).toEqual(['b', 'a'])
        expect(byDate('desc').slice(0, 2)).toEqual(['a', 'b'])
        expect(byDate('desc').slice(2).sort()).toEqual([anna, zola].sort())

        const bySeries = ids(visibleShelf(shelf, 'all', NONE, { field: 'series', order: 'desc' }))
        expect(bySeries[0]).toBe('a')
    })

    it('sorts by reading progress, books without any last', () => {
        expect(ids(visibleShelf(shelf, 'all', NONE, { field: 'progress', order: 'desc' }))[0]).toBe(zola)
        expect(ids(visibleShelf(shelf, 'all', NONE, { field: 'progress', order: 'asc' }))[0]).toBe(zola)
    })

    it('puts the favorites first whatever the sort, each group in its order', () => {
        const starred = buildShelf(
            [
                book('a', 'Dune'), book('b', 'Emma', 'Jane Austen'),
                { ...book('c', 'Hyperion', 'Dan Simmons'), favorite: true }, { ...book('d', 'Anathem', 'Neal Stephenson'), favorite: true },
            ],
            [file('ann.pdf', 'Anna', 'Leo Tolstoy')],
            {},
        )
        const order = (sort: SortConfig) => ids(visibleShelf(starred, 'all', NONE, sort))
        const anna = deviceItemId(file('ann.pdf'))
        expect(order(BY_TITLE)).toEqual(['d', 'c', anna, 'a', 'b'])
        expect(order({ field: 'title', order: 'desc' })).toEqual(['c', 'd', 'b', 'a', anna])
        expect(order({ field: 'author', order: 'asc' })).toEqual(['c', 'd', 'a', 'b', anna])
        // A filter still applies to them.
        expect(ids(visibleShelf(starred, 'all', { ...NONE, search: 'hyperion' }, BY_TITLE))).toEqual(['c'])
    })
})
