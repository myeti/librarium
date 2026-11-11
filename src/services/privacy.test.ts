import { describe, expect, it } from 'vitest'
import {
    bucketCount, bucketMinutes, safeContextKey, safeDeviceModel, safeErrorCode,
    UNKNOWN_CONTEXT, UNKNOWN_ERROR_CODE,
} from './privacy'

describe('safeErrorCode', () => {
    it('keeps fixed upper-case codes', () => {
        expect(safeErrorCode('IMPORT_FAILED')).toBe('IMPORT_FAILED')
        expect(safeErrorCode('NOT_FOUND')).toBe('NOT_FOUND')
    })

    it.each([
        ['a message', 'Dune.epub can’t be added to the library'],
        ['a path', '/srv/books/Ann/Dune.epub'],
        ['a lower-case word', 'import_failed'],
        ['a code followed by text', 'IO: cannot read /opt/x'],
        ['an over-long string', `A${'B'.repeat(80)}`],
        ['nothing', ''],
    ])('replaces %s with UNKNOWN', (_label, input) => {
        expect(safeErrorCode(input)).toBe(UNKNOWN_ERROR_CODE)
    })

    it('handles missing input', () => {
        expect(safeErrorCode(null)).toBe(UNKNOWN_ERROR_CODE)
        expect(safeErrorCode(undefined)).toBe(UNKNOWN_ERROR_CODE)
    })
})

describe('safeContextKey', () => {
    it('keeps dotted keys written in the code', () => {
        expect(safeContextKey('library.trash')).toBe('library.trash')
        expect(safeContextKey('tidyUp.batch')).toBe('tidyUp.batch')
        expect(safeContextKey('app.startup')).toBe('app.startup')
    })

    it.each([
        ['a file name', 'Dune.epub'],
        ['a sentence', 'could not read the library'],
        ['a path', '/opt/books/x'],
        ['a Windows path', 'C:\\Users\\ann'],
        ['an e-mail address', 'ann@example.com'],
        ['an over-long key', `a${'b'.repeat(80)}`],
        ['nothing', ''],
    ])('replaces %s with unknown', (_label, input) => {
        expect(safeContextKey(input)).toBe(UNKNOWN_CONTEXT)
    })
})

describe('safeDeviceModel', () => {
    it('keeps ordinary model names', () => {
        expect(safeDeviceModel('Kindle Paperwhite 5')).toBe('Kindle Paperwhite 5')
        expect(safeDeviceModel('Kobo Clara HD')).toBe('Kobo Clara HD')
        expect(safeDeviceModel('Kobo fw 4.20.14622')).toBe('Kobo fw 4.20.14622')
    })

    it('drops anything that looks like a serial number', () => {
        expect(safeDeviceModel('Kobo N418170012345')).toBe('unknown')
        expect(safeDeviceModel('G090G1234567ABCD')).toBe('unknown')
    })

    it('reports missing models as unknown', () => {
        expect(safeDeviceModel(undefined)).toBe('unknown')
        expect(safeDeviceModel('')).toBe('unknown')
    })
})

describe('bucketCount', () => {
    it.each([
        [0, '0'],
        [1, '1'],
        [2, '2-5'],
        [5, '2-5'],
        [6, '6-20'],
        [20, '6-20'],
        [21, '21-100'],
        [100, '21-100'],
        [101, '100+'],
        [5000, '100+'],
    ])('%i → %s', (n, expected) => {
        expect(bucketCount(n)).toBe(expected)
    })

    it('never returns the number itself', () => {
        for (const n of [3, 7, 13, 42, 99, 250]) {
            expect(bucketCount(n)).not.toBe(String(n))
        }
    })

    it('reads nonsense as 0', () => {
        expect(bucketCount(-3)).toBe('0')
        expect(bucketCount(Number.NaN)).toBe('0')
    })
})

describe('bucketMinutes', () => {
    it.each([
        [0, '<5'],
        [4.9, '<5'],
        [5, '5-15'],
        [14.9, '5-15'],
        [15, '15-60'],
        [59, '15-60'],
        [60, '60+'],
        [600, '60+'],
    ])('%f min → %s', (minutes, expected) => {
        expect(bucketMinutes(minutes)).toBe(expected)
    })

    it('reads nonsense as the shortest range', () => {
        expect(bucketMinutes(Number.NaN)).toBe('<5')
        expect(bucketMinutes(-1)).toBe('<5')
    })
})
