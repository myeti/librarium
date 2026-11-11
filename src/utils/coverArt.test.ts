import { describe, expect, it } from 'vitest'
import { COVER_COLOR_COUNT, coverColor } from './coverArt'

const book = (id: string, series?: string) => ({ id, metadata: { title: id, author: '', series } as never })

describe('coverColor', () => {
    it('is a cover colour token, the same for a book every time', () => {
        expect(coverColor(book('a'))).toMatch(/^var\(--cover-[1-6]\)$/)
        expect(coverColor(book('a'))).toBe(coverColor(book('a')))
    })

    it('dresses a series alike, whatever the case or spacing of its name', () => {
        expect(coverColor(book('1', 'Dune'))).toBe(coverColor(book('2', ' dune ')))
    })

    it('spreads books without a series over every colour', () => {
        const used = new Set(Array.from({ length: 60 }, (_, i) => coverColor(book(`book-${i}`))))
        expect(used.size).toBe(COVER_COLOR_COUNT)
    })
})
