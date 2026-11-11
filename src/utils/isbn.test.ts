import { describe, expect, it } from 'vitest'
import { cleanIsbn, isbn10To13, isbn13To10, isbnKey, isbnLookupForms, isValidIsbn10, isValidIsbn13, toIsbn13 } from './isbn'

// Dune (Ace, 2005): ISBN-10 0441013597, ISBN-13 9780441013593.
// An ISBN-10 with an X check digit: 080442957X → 9780804429573.

describe('ISBN helpers', () => {
    it('keeps only digits and a final X', () => {
        expect(cleanIsbn('ISBN 978-0-441-01359-3')).toBe('9780441013593')
        expect(cleanIsbn('0-8044-2957-x')).toBe('080442957X')
        expect(cleanIsbn('X123')).toBe('123')
        expect(cleanIsbn(undefined)).toBe('')
    })

    it('checks the check digits', () => {
        expect(isValidIsbn13('9780441013593')).toBe(true)
        expect(isValidIsbn13('9780441013594')).toBe(false)
        expect(isValidIsbn10('0441013597')).toBe(true)
        expect(isValidIsbn10('080442957X')).toBe(true)
        expect(isValidIsbn10('0441013596')).toBe(false)
    })

    it('converts between ISBN-10 and ISBN-13', () => {
        expect(isbn10To13('0441013597')).toBe('9780441013593')
        expect(isbn10To13('080442957X')).toBe('9780804429573')
        expect(isbn13To10('9780441013593')).toBe('0441013597')
        expect(isbn13To10('9780804429573')).toBe('080442957X')
        // 979 ISBNs have no ISBN-10.
        expect(isbn13To10('9791032305690')).toBeNull()
    })

    it('gives the ISBN-13 of any valid spelling', () => {
        expect(toIsbn13('0-441-01359-7')).toBe('9780441013593')
        expect(toIsbn13('978-0-441-01359-3')).toBe('9780441013593')
        expect(toIsbn13('12345')).toBeNull()
    })

    it('lists the forms to look a book up by', () => {
        expect(isbnLookupForms('0-441-01359-7')).toEqual(['9780441013593', '0441013597'])
        expect(isbnLookupForms('9791032305690')).toEqual(['9791032305690'])
        // Right length, wrong check digit: tried as written, not converted.
        expect(isbnLookupForms('9780441013594')).toEqual(['9780441013594'])
        expect(isbnLookupForms('not an isbn')).toEqual([])
        expect(isbnLookupForms('')).toEqual([])
    })

    it('keys an ISBN-10 and its ISBN-13 the same way', () => {
        expect(isbnKey('0441013597')).toBe(isbnKey('978-0-441-01359-3'))
        expect(isbnKey('odd-123')).toBe('123')
        expect(isbnKey('')).toBe('')
    })
})
