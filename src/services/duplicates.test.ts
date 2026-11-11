import { describe, expect, it } from 'vitest'
import { findDuplicateGroups, titleAuthorKey } from './duplicates'
import type { Book, BookMetadata } from '@/types'

function book(id: string, metadata: Partial<BookMetadata>): Book {
    return {
        id, path: `/lib/${id}.epub`, filename: `${id}.epub`, format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01',
        metadata: { title: '', author: '', categories: [], ...metadata },
    }
}

const ids = (groups: ReturnType<typeof findDuplicateGroups>) => groups.map(g => g.books.map(b => b.id))

describe('duplicate detection', () => {
    it('keeps non-Latin titles apart (they must not collapse to one empty key)', () => {
        const groups = findDuplicateGroups([
            book('war', { title: 'Война и мир', author: 'Лев Толстой' }),
            book('crime', { title: 'Преступление и наказание', author: 'Фёдор Достоевский' }),
            book('genji', { title: '源氏物語', author: '紫式部' }),
            book('snow', { title: '雪国', author: '川端康成' }),
        ])
        expect(groups).toEqual([])
    })

    it('still finds a non-Latin book that is there twice', () => {
        const groups = findDuplicateGroups([
            book('a', { title: 'Война и мир', author: 'Лев Толстой' }),
            book('b', { title: 'ВОЙНА И МИР', author: 'Толстой Лев' }),
        ])
        expect(ids(groups)).toEqual([['a', 'b']])
    })

    it('ignores case, accents, punctuation and author word order', () => {
        const groups = findDuplicateGroups([
            book('a', { title: 'L’Étranger', author: 'Albert Camus' }),
            book('b', { title: 'l etranger', author: 'Camus, Albert' }),
            book('c', { title: 'La Peste', author: 'Albert Camus' }),
        ])
        expect(ids(groups)).toEqual([['a', 'b']])
        expect(groups[0].reason).toBe('titleAuthor')
    })

    it('matches an ISBN-10 with its ISBN-13', () => {
        const groups = findDuplicateGroups([
            book('a', { title: 'Dune', isbn: '0-441-01359-7' }),
            book('b', { title: 'Dune (Ace)', isbn: '978-0-441-01359-3' }),
        ])
        expect(ids(groups)).toEqual([['a', 'b']])
        expect(groups[0]).toMatchObject({ reason: 'isbn', isbn: '0-441-01359-7', key: 'isbn:9780441013593' })
    })

    it('never groups books whose title is unknown', () => {
        const groups = findDuplicateGroups([
            book('x', { title: '' }),
            book('y', { title: '' }),
            book('z', { title: '…' }),
        ])
        expect(groups).toEqual([])
    })

    it('does not put a book in two groups', () => {
        const groups = findDuplicateGroups([
            book('a', { title: 'Dune', author: 'Herbert', isbn: '9780441013593' }),
            book('b', { title: 'Dune', author: 'Herbert', isbn: '9780441013593' }),
            book('c', { title: 'Dune', author: 'Herbert' }),
        ])
        // a and b by ISBN; c alone is left (its title match partners are taken).
        expect(ids(groups)).toEqual([['a', 'b']])
    })

    it('keys titles and authors', () => {
        expect(titleAuthorKey('Dune', 'Frank Herbert')).toBe(titleAuthorKey('DUNE!', 'Herbert, Frank'))
        expect(titleAuthorKey('', 'Frank Herbert')).toBeNull()
    })
})
