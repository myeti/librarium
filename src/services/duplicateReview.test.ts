import { describe, expect, it } from 'vitest'
import { copiesToRemove, copyFacts, suggestKeeper } from './duplicateReview'
import type { DuplicateGroup } from './duplicates'
import type { Book, BookFormat, BookMetadata } from '@/types'

function book(id: string, format: BookFormat = 'epub', metadata: Partial<BookMetadata> = {}, dateAdded = '2026-01-01T00:00:00Z'): Book {
    return {
        id, path: `/lib/${id}.${format}`, filename: `${id}.${format}`, format, sizeBytes: 1000, dateAdded,
        metadata: { title: 'Dune', author: 'Frank Herbert', categories: [], ...metadata },
    }
}

const group = (...books: Book[]): DuplicateGroup => ({ key: 'title:dune::frank herbert', reason: 'titleAuthor', books })
const noCovers = () => false

describe('copyFacts', () => {
    it('counts the details a copy has, the cover included, and names the missing ones', () => {
        const facts = copyFacts(book('a', 'epub', { series: 'Dune', isbn: '9780441013593', categories: ['Science Fiction'] }), true)
        expect(facts.missing).toEqual(['language', 'description'])
        expect(facts.filled).toBe(facts.total - 2)
        expect(copyFacts(book('b', 'epub', { author: '  ' }), false).missing).toContain('author')
    })
})

describe('suggestKeeper', () => {
    it('suggests the most complete copy first', () => {
        const poor = book('poor', 'epub')
        const rich = book('rich', 'pdf', { isbn: '978', description: 'A desert planet.' })
        expect(suggestKeeper(group(poor, rich), noCovers)).toEqual({ bookId: 'rich', reason: 'complete' })
    })

    it('then the better format, then a cover, then the copy added first', () => {
        expect(suggestKeeper(group(book('pdf', 'pdf'), book('epub', 'epub')), noCovers)).toEqual({ bookId: 'epub', reason: 'format' })
        // Same details otherwise: having a cover counts as a detail, so it wins on completeness.
        expect(suggestKeeper(group(book('a'), book('b')), id => id === 'b')).toEqual({ bookId: 'b', reason: 'complete' })
        const older = book('older', 'epub', {}, '2025-01-01T00:00:00Z')
        const newer = book('newer', 'epub', {}, '2026-06-01T00:00:00Z')
        expect(suggestKeeper(group(newer, older), noCovers)).toEqual({ bookId: 'older', reason: 'oldest' })
    })

    it('falls back to library order for identical copies, and is stable', () => {
        const g = group(book('x'), book('y'))
        expect(suggestKeeper(g, noCovers)).toEqual({ bookId: 'x', reason: 'first' })
        expect(suggestKeeper(g, noCovers)).toEqual({ bookId: 'x', reason: 'first' })
        expect(suggestKeeper(group(), noCovers)).toBeNull()
    })
})

describe('copiesToRemove', () => {
    it('lists the other copies of each group with a kept copy, and skips groups without one', () => {
        const dune = group(book('d1'), book('d2'), book('d3'))
        const emma: DuplicateGroup = { key: 'isbn:1', reason: 'isbn', books: [book('e1'), book('e2')] }
        expect(copiesToRemove([dune, emma], new Set(['d2']))).toEqual(['d1', 'd3'])
        expect(copiesToRemove([dune, emma], new Set(['d2', 'e1']))).toEqual(['d1', 'd3', 'e2'])
        expect(copiesToRemove([dune, emma], new Set())).toEqual([])
        // Two "kept" copies in one group is no decision: nothing is removed there.
        expect(copiesToRemove([dune], new Set(['d1', 'd2']))).toEqual([])
    })
})
