import { describe, expect, it } from 'vitest'
import { DeviceIndex, isSameBook, sameAuthor, sameTitle } from './deviceMatch'
import type { Book, DeviceBook } from '@/types'

function book(title: string, author: string, id = 'b1'): Book {
    return {
        id, path: `/lib/${title}.epub`, filename: `${title}.epub`, format: 'epub',
        sizeBytes: 1, dateAdded: '2026-01-01', metadata: { title, author },
    }
}

function onDevice(title: string, author: string, path = `/Volumes/Kindle/documents/${title}.azw3`): DeviceBook {
    return { path, filename: path.split('/').pop()!, title, author }
}

describe('sameTitle / sameAuthor', () => {
    it('ignore case, accents, punctuation and spacing', () => {
        expect(sameTitle('Les Misérables', 'les  miserables')).toBe(true)
        expect(sameTitle("L'Étranger", 'L’étranger')).toBe(true)
        expect(sameAuthor('Émile Zola', 'emile zola')).toBe(true)
    })

    it('accept the "Last, First" author form', () => {
        expect(sameAuthor('Herbert, Frank', 'Frank Herbert')).toBe(true)
    })

    it('never match empty values', () => {
        expect(sameTitle('', '')).toBe(false)
        expect(sameAuthor('', '')).toBe(false)
    })
})

describe('isSameBook', () => {
    it('requires both title and author to match', () => {
        expect(isSameBook(book('Dune', 'Frank Herbert'), onDevice('Dune', 'Herbert, Frank'))).toBe(true)
        expect(isSameBook(book('Dune', 'Frank Herbert'), onDevice('Dune', 'Brian Herbert'))).toBe(false)
        expect(isSameBook(book('Dune', 'Frank Herbert'), onDevice('Dune', ''))).toBe(false)
    })
})

describe('DeviceIndex ("already on device")', () => {
    const device = [
        onDevice('Dune', 'Herbert, Frank'),
        onDevice('Poems', 'Emily Dickinson'),
        onDevice('', '', '/Volumes/Kindle/documents/My Notes.azw3'),
    ]
    const index = new DeviceIndex(device)

    it('agrees with isSameBook on title and author', () => {
        const books = [
            book('Dune', 'Frank Herbert'), book('dune', 'FRANK HERBERT'), book('Poems', 'Sylvia Plath'),
            book('Emma', 'Jane Austen'), book('', ''),
        ]
        for (const b of books) {
            const bySlowScan = device.some(db => isSameBook(b, db)) || device.some(db =>
                db.filename.replace(/\.[^.]+$/, '').toLowerCase() === b.filename.replace(/\.[^.]+$/, '').toLowerCase())
            expect(index.has(b), b.metadata.title).toBe(bySlowScan)
        }
    })

    it('also matches by file name, whatever the extension and case', () => {
        expect(index.has({ ...book('Something else', 'Someone'), filename: 'my notes.epub' })).toBe(true)
    })

    it('never matches on an unknown title and author alone', () => {
        expect(index.has({ ...book('', ''), filename: 'other.epub' })).toBe(false)
    })
})

describe('DeviceIndex with library ids', () => {
    const sent = (title: string, author: string, libraryId: string): DeviceBook =>
        ({ ...onDevice(title, author), libraryId })

    it('matches the books Librarium sent by id, whatever their title', () => {
        const index = new DeviceIndex([sent('Renamed on purpose', 'Nobody', 'id-dune')], new Set(['id-dune']))
        expect(index.has(book('Dune', 'Frank Herbert', 'id-dune'))).toBe(true)
    })

    it('never lends a sent copy to another edition with the same title, author or file name', () => {
        const index = new DeviceIndex([sent('Dune', 'Frank Herbert', 'id-1965')], new Set(['id-1965', 'id-2005']))
        expect(index.has(book('Dune', 'Frank Herbert', 'id-1965'))).toBe(true)
        expect(index.has(book('Dune', 'Frank Herbert', 'id-2005'))).toBe(false)
    })

    it('matches another library’s copy like a book without an id', () => {
        // Sent from another user's library: same book, unknown id here.
        const index = new DeviceIndex([sent('Dune', 'Herbert, Frank', 'their-id')], new Set(['my-id']))
        expect(index.has(book('Dune', 'Frank Herbert', 'my-id'))).toBe(true)
    })

    it('without the library ids, a device book with an id is matched by id only', () => {
        const index = new DeviceIndex([sent('Dune', 'Frank Herbert', 'id-1965')])
        expect(index.has(book('Dune', 'Frank Herbert', 'id-2005'))).toBe(false)
        expect(index.has(book('Dune', 'Frank Herbert', 'id-1965'))).toBe(true)
    })
})
