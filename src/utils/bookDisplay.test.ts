import { describe, expect, it } from 'vitest'
import { blanksLast, displayAuthor, displayTitle, isBlank, seriesLabel } from './bookDisplay'
import { currentLang, t } from '@/i18n'
import type { Book } from '@/types'

function book(title: string, author: string): Book {
    return {
        id: 'b', path: '/lib/b.epub', filename: 'b.epub', format: 'epub', sizeBytes: 1,
        dateAdded: '2026-01-01T00:00:00Z', metadata: { title, author, categories: [] },
    }
}

describe('bookDisplay', () => {
    it('shows known values as they are', () => {
        expect(displayTitle(book('Dune', 'Frank Herbert'), t)).toBe('Dune')
        expect(displayAuthor(book('Dune', 'Frank Herbert'), t)).toBe('Frank Herbert')
        expect(displayTitle({ title: ' Dune ', author: '' }, t)).toBe('Dune')
    })

    it('shows a localised placeholder for empty values', () => {
        currentLang.value = 'en'
        expect(displayTitle(book('', ''), t)).toBe('Unknown title')
        expect(displayAuthor(book('', '   '), t)).toBe('Unknown author')
        currentLang.value = 'fr'
        expect(displayTitle(book('', ''), t)).toBe('Titre inconnu')
        expect(displayAuthor(book('', ''), t)).toBe('Auteur inconnu')
        currentLang.value = 'en'
    })

    it('treats absent, empty and whitespace values as unknown', () => {
        expect(isBlank(undefined)).toBe(true)
        expect(isBlank(null)).toBe(true)
        expect(isBlank(' \t')).toBe(true)
        expect(isBlank('x')).toBe(false)
    })

    it('sorts blanks last in both directions', () => {
        expect(blanksLast('', 'a')).toBe(1)
        expect(blanksLast('a', ' ')).toBe(-1)
        expect(blanksLast('a', 'b')).toBeNull()
        expect(blanksLast('', '')).toBeNull()
        expect(blanksLast(3, 4)).toBeNull()
    })
})

describe('seriesLabel', () => {
    function inSeries(series?: string, seriesIndex?: number): Book {
        return { ...book('T', 'A'), metadata: { title: 'T', author: 'A', categories: [], series, seriesIndex } }
    }

    it('shows the series with its volume', () => {
        expect(seriesLabel(inSeries('Dune', 2))).toBe('Dune #2')
        expect(seriesLabel(inSeries('Saga', 0))).toBe('Saga #0')
        expect(seriesLabel(inSeries('Discworld', 1.5))).toBe('Discworld #1.5')
    })

    it('never shows "Saga #" without a number', () => {
        expect(seriesLabel(inSeries('Saga'))).toBe('Saga')
        expect(seriesLabel(inSeries(' Saga ', Number.NaN))).toBe('Saga')
    })

    it('is null outside a series', () => {
        expect(seriesLabel(inSeries())).toBeNull()
        expect(seriesLabel(inSeries('   ', 3))).toBeNull()
    })
})
