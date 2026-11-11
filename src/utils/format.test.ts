import { afterEach, describe, expect, it } from 'vitest'
import { fmtDate, fmtLanguage, fmtSize, fmtTime } from './format'
import { currentLang } from '@/i18n'

afterEach(() => { currentLang.value = 'en' })

describe('fmtDate / fmtTime', () => {
    it('follows the UI language', () => {
        expect(fmtDate('2026-03-12T10:00:00Z', 'en')).toBe('Mar 12, 2026')
        expect(fmtDate('2026-03-12T10:00:00Z', 'fr')).toBe('12 mars 2026')
        currentLang.value = 'fr'
        expect(fmtDate('2026-03-12T10:00:00Z')).toBe('12 mars 2026')
    })

    it('shows a dash for an unreadable date', () => {
        expect(fmtDate('not a date')).toBe('—')
        expect(fmtTime('')).toBe('—')
    })

    it('formats times of day', () => {
        expect(fmtTime(new Date(2026, 0, 1, 15, 5).toISOString(), 'fr')).toBe('15:05')
    })
})

describe('fmtLanguage', () => {
    it('names the language in the UI language, capitalised', () => {
        expect(fmtLanguage('en', 'en')).toBe('English')
        expect(fmtLanguage('fr', 'en')).toBe('French')
        expect(fmtLanguage('en', 'fr')).toBe('Anglais')
        expect(fmtLanguage('pt-BR', 'en')).toBe('Brazilian Portuguese')
    })

    it('shows a dash for nothing or "und", and an unknown code in capitals', () => {
        expect(fmtLanguage(undefined)).toBe('—')
        expect(fmtLanguage('und')).toBe('—')
        expect(fmtLanguage('zz-not-a-tag!')).toBe('ZZ-NOT-A-TAG!')
    })
})

describe('fmtSize', () => {
    it.each([
        [0, '0 B'],
        [512, '512 B'],
        [1024, '1 KB'],
        [350 * 1024, '350 KB'],
        [1024 * 1024, '1.0 MB'],
        [5.25 * 1024 * 1024, '5.3 MB'],
        // The case from the review: 2956.4 MB is now shown in GB.
        [2956.4 * 1024 * 1024, '2.9 GB'],
        [32 * 1024 ** 3, '32.0 GB'],
        [2.5 * 1024 ** 4, '2.5 TB'],
    ])('formats %d bytes as %s', (bytes, expected) => {
        expect(fmtSize(bytes)).toBe(expected)
    })

    it('shows a dash for impossible sizes', () => {
        expect(fmtSize(-1)).toBe('—')
        expect(fmtSize(Number.NaN)).toBe('—')
    })
})
