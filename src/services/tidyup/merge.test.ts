import { describe, expect, it } from 'vitest'
import {
    stripUndefined, isEmptyValue, fillEmptyFields, titleSimilarity, isConfidentMatch,
    withoutPlaceholderCover, MIN_TITLE_SIMILARITY,
} from './merge'
import type { BookMetadata } from '@/types'

const book = (m: Partial<BookMetadata>): BookMetadata => ({ title: 'Dune', author: 'Frank Herbert', ...m })

describe('stripUndefined', () => {
    it('drops undefined keys and keeps every other value', () => {
        expect(stripUndefined({ a: 1, b: undefined, c: '', d: null, e: [] })).toEqual({ a: 1, c: '', d: null, e: [] })
    })

    it('stops a provider result from erasing fields when spread', () => {
        const current = book({ description: 'A desert planet.' })
        const incoming = { title: 'Dune', description: undefined }
        expect({ ...current, ...stripUndefined(incoming) }.description).toBe('A desert planet.')
    })
})

describe('isEmptyValue', () => {
    it.each([undefined, null, '', '   ', [], 'und', 'Unknown', 'Unknown author', 'unknown title'])(
        'treats %j as empty', v => expect(isEmptyValue(v)).toBe(true),
    )
    it.each(['fr', 'Dune', ['Fiction'], 0, 3])('treats %j as filled in', v => expect(isEmptyValue(v)).toBe(false))
})

describe('fillEmptyFields (batch tidy-up)', () => {
    it('only fills fields that are empty on the book', () => {
        const current = book({ language: 'fr', description: '', isbn: undefined, categories: [] })
        const incoming = {
            title: 'Dune (Deluxe Edition)', author: 'F. Herbert', language: 'en',
            description: 'Melange…', isbn: '9780441013593', categories: ['Science Fiction'],
        }
        expect(fillEmptyFields(current, incoming)).toEqual({
            description: 'Melange…', isbn: '9780441013593', categories: ['Science Fiction'],
        })
    })

    it('replaces placeholders such as "und" and "Unknown author"', () => {
        const current = book({ author: 'Unknown author', language: 'und' })
        expect(fillEmptyFields(current, { author: 'Frank Herbert', language: 'en' }))
            .toEqual({ author: 'Frank Herbert', language: 'en' })
    })

    it('ignores undefined and empty incoming values', () => {
        const current = book({})
        expect(fillEmptyFields(current, { description: undefined, isbn: '', categories: [] })).toEqual({})
    })
})

describe('titleSimilarity', () => {
    it('is 1 for the same title written differently', () => {
        expect(titleSimilarity('L’Étranger', "l'etranger")).toBe(1)
        expect(titleSimilarity('The Hobbit', 'Hobbit')).toBe(1)
        expect(titleSimilarity('LES MISÉRABLES', 'Les Misérables')).toBe(1)
    })

    it('accepts an added or dropped subtitle', () => {
        expect(titleSimilarity('Sapiens', 'Sapiens: A Brief History of Humankind')).toBe(1)
        expect(titleSimilarity('The Hobbit', 'The Hobbit, or There and Back Again')).toBe(1)
    })

    it('accepts small wording differences', () => {
        expect(titleSimilarity(
            "Harry Potter and the Philosopher's Stone",
            "Harry Potter and the Sorcerer's Stone",
        )).toBeGreaterThanOrEqual(MIN_TITLE_SIMILARITY)
    })

    it('rejects a different book that shares a word', () => {
        expect(titleSimilarity('Dune', 'Dune Messiah')).toBeLessThan(MIN_TITLE_SIMILARITY)
        expect(titleSimilarity('Love, Rosie', 'Love, Actually')).toBeLessThan(MIN_TITLE_SIMILARITY)
        expect(titleSimilarity('Le Petit Prince', 'Le Prince')).toBeLessThan(MIN_TITLE_SIMILARITY)
    })

    it('is 0 for unrelated or missing titles', () => {
        expect(titleSimilarity('Dune', 'Pride and Prejudice')).toBe(0)
        expect(titleSimilarity('', 'Dune')).toBe(0)
        expect(titleSimilarity('Dune', undefined)).toBe(0)
    })

    it('compares non-Latin titles instead of reducing them to nothing', () => {
        expect(titleSimilarity('Война и мир', 'война и мир')).toBe(1)
        expect(titleSimilarity('Война и мир', 'Анна Каренина')).toBe(0)
    })
})

describe('isConfidentMatch', () => {
    it('uses the minimum similarity threshold', () => {
        expect(isConfidentMatch('Dune', 'Dune')).toBe(true)
        expect(isConfidentMatch('Dune', 'Dune Messiah')).toBe(false)
        expect(isConfidentMatch('Dune', undefined)).toBe(false)
    })
})

describe('withoutPlaceholderCover', () => {
    it('adds default=false to Open Library cover URLs', () => {
        expect(withoutPlaceholderCover('https://covers.openlibrary.org/b/isbn/9780441013593-L.jpg'))
            .toBe('https://covers.openlibrary.org/b/isbn/9780441013593-L.jpg?default=false')
    })

    it('leaves other URLs alone', () => {
        const url = 'https://books.google.com/books/content?id=x&zoom=0'
        expect(withoutPlaceholderCover(url)).toBe(url)
    })
})
