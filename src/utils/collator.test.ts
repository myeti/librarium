import { describe, expect, it } from 'vitest'
import { compareText, sortedText } from './collator'

describe('compareText / sortedText', () => {
    it('sorts accented letters with their base letter', () => {
        // A plain `<` puts every "É" after "Z".
        expect(sortedText(['Zola', 'Élise', 'Emma'], 'fr')).toEqual(['Élise', 'Emma', 'Zola'])
        expect(sortedText(['Zola', 'Élise', 'Emma'], 'en')).toEqual(['Élise', 'Emma', 'Zola'])
    })

    it('compares numbers inside titles as numbers', () => {
        expect(sortedText(['Tome 10', 'Tome 2', 'Tome 1'])).toEqual(['Tome 1', 'Tome 2', 'Tome 10'])
    })

    it('ignores case', () => {
        expect(compareText('dune', 'Dune')).toBe(0)
        expect(sortedText(['beta', 'Alpha', 'alpha2'])).toEqual(['Alpha', 'alpha2', 'beta'])
    })

    it('keeps the order of values it considers equal (stable sort)', () => {
        expect(sortedText(['dune', 'Dune', 'DUNE'])).toEqual(['dune', 'Dune', 'DUNE'])
    })
})
