import { describe, expect, it } from 'vitest'
import { mergeResults, relevance } from './merge'
import type { StoreBook } from './types'

function book(id: string, title: string, author: string, extra: Partial<StoreBook> = {}): StoreBook {
    return { id, source: id.split('-')[0], title, author, downloadUrl: `https://x/${id}`, format: 'epub', ...extra }
}

describe('mergeResults', () => {
    it('keeps one entry per book, the better copy: EPUB, then a cover, then the first catalog', () => {
        const merged = mergeResults([
            book('openlibrary-1', 'Emma', 'Jane Austen', { format: 'pdf', coverUrl: 'c' }),
            book('gutendex-1', 'EMMA', 'Austen, Jane'.split(', ').reverse().join(' ')),
            book('opds-1', 'Émma!', 'jane austen', { coverUrl: 'c' }),
        ], 'emma', ['gutendex', 'openlibrary', 'opds'])
        expect(merged.map(b => b.id)).toEqual(['opds-1'])

        const sameBoth = mergeResults([
            book('openlibrary-2', 'Persuasion', 'Jane Austen'),
            book('gutendex-2', 'Persuasion', 'Jane Austen'),
        ], '', ['gutendex', 'openlibrary'])
        expect(sameBoth.map(b => b.id)).toEqual(['gutendex-2'])
    })

    it('never merges books without a title', () => {
        const merged = mergeResults([book('a-1', '', 'X'), book('a-2', '', 'X')], '')
        expect(merged).toHaveLength(2)
    })

    it('puts the best matches first, then sorts by title, whatever answered first', () => {
        const results = [
            book('a-1', 'Pride and Prejudice', 'Jane Austen'),
            book('a-2', 'Jane Austen: a life', 'Claire Tomalin'),
            book('a-3', 'Jane Austen', 'Anonymous'),
            book('a-4', 'Lady Susan', ''),
            book('a-5', 'Emma', 'Jane Austen'),
            book('a-6', 'Letters of Jane Austen', 'Cassandra'),
        ]
        const order = mergeResults(results, 'Jane Austen').map(b => b.title)
        expect(order).toEqual(['Jane Austen', 'Jane Austen: a life', 'Letters of Jane Austen', 'Emma', 'Pride and Prejudice', 'Lady Susan'])
        expect(mergeResults([...results].reverse(), 'Jane Austen').map(b => b.title)).toEqual(order)
    })

    it('ranks titles before authors, ignoring case and accents', () => {
        expect(relevance(book('a', 'Les Misérables', 'Victor Hugo'), 'les miserables')).toBe(0)
        expect(relevance(book('a', 'Notre-Dame de Paris', 'Victor Hugo'), 'notre dame')).toBe(1)
        expect(relevance(book('a', 'Le Comte de Monte-Cristo', 'Dumas'), 'monte cristo')).toBe(2)
        expect(relevance(book('a', 'Les Misérables', 'Victor Hugo'), 'hugo')).toBe(3)
        expect(relevance(book('a', 'Dune', 'Frank Herbert'), 'hugo')).toBe(4)
    })
})
