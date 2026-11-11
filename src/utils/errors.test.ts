import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { ERROR_CODES, errorCode, errorMessage, isAppError, isLibraryUnavailable, isPermissionDenied } from './errors'

const MESSAGE = 'The library folder "Alice" can’t be reached. Is the drive or NAS connected?'
/** What the backend rejects with (`AppError`, serialised). */
const UNAVAILABLE = { code: 'LIBRARY_UNAVAILABLE', message: MESSAGE }
/** The same error as a plain `CODE: message` string, which is read too. */
const LEGACY_UNAVAILABLE = `LIBRARY_UNAVAILABLE: ${MESSAGE}`

describe('errorMessage', () => {
    it('reads strings, Errors and { message } objects', () => {
        expect(errorMessage('plain')).toBe('plain')
        expect(errorMessage(new Error('boom'))).toBe('boom')
        expect(errorMessage({ message: 'obj' })).toBe('obj')
        expect(errorMessage(undefined)).toBe('')
        expect(errorMessage(42)).toBe('42')
    })

    it('reads backend { code, message } errors', () => {
        expect(errorMessage(UNAVAILABLE)).toBe(MESSAGE)
        expect(errorMessage({ code: 'NOT_FOUND', message: 'Book x is not in the library' })).toBe('Book x is not in the library')
    })

    it('drops a legacy code prefix', () => {
        expect(errorMessage(LEGACY_UNAVAILABLE)).toBe(MESSAGE)
        expect(errorMessage(new Error(LEGACY_UNAVAILABLE))).toBe(MESSAGE)
        // Only known codes are prefixes: other capitalised words stay.
        expect(errorMessage('HTTP: 404')).toBe('HTTP: 404')
    })
})

describe('errorCode', () => {
    it('is the code of backend errors and legacy prefixes', () => {
        expect(errorCode(UNAVAILABLE)).toBe('LIBRARY_UNAVAILABLE')
        expect(errorCode({ code: 'CONVERSION', message: 'DRM' })).toBe('CONVERSION')
        expect(errorCode(LEGACY_UNAVAILABLE)).toBe('LIBRARY_UNAVAILABLE')
        expect(errorCode(new Error(LEGACY_UNAVAILABLE))).toBe('LIBRARY_UNAVAILABLE')
    })

    it('is null when there is none', () => {
        expect(errorCode('Book x is not in the library')).toBeNull()
        expect(errorCode('Error: LIBRARY_UNAVAILABLE: x')).toBeNull()
        expect(errorCode({ message: 'no code' })).toBeNull()
        expect(errorCode({ code: 42, message: 'not a string code' })).toBeNull()
        expect(errorCode(undefined)).toBeNull()
    })

    it('recognises AppError objects', () => {
        expect(isAppError(UNAVAILABLE)).toBe(true)
        expect(isAppError({ message: 'x' })).toBe(false)
        expect(isAppError(null)).toBe(false)
    })
})

describe('isLibraryUnavailable', () => {
    it('detects the code in every shape', () => {
        expect(isLibraryUnavailable(UNAVAILABLE)).toBe(true)
        expect(isLibraryUnavailable(LEGACY_UNAVAILABLE)).toBe(true)
        expect(isLibraryUnavailable(new Error(LEGACY_UNAVAILABLE))).toBe(true)
        expect(isLibraryUnavailable({ code: 'NOT_FOUND', message: MESSAGE })).toBe(false)
        expect(isLibraryUnavailable('Book x is not in the library')).toBe(false)
    })
})

describe('isPermissionDenied', () => {
    it('tells a refused folder from a missing one', () => {
        const denied = { code: 'PERMISSION_DENIED', message: 'Librarium isn’t allowed to open the library folder "Alice".' }
        expect(isPermissionDenied(denied)).toBe(true)
        expect(isLibraryUnavailable(denied)).toBe(false)
        expect(isPermissionDenied(UNAVAILABLE)).toBe(false)
    })
})

describe('ERROR_CODES', () => {
    it('lists every code the backend sends (src-tauri/src/error.rs)', () => {
        const source = readFileSync(join(process.cwd(), 'src-tauri/src/error.rs'), 'utf8')
        const backend = [...source.matchAll(/Self::\w+(?:\s*\{\s*\.\.\s*\}|\(_\))?\s*=>\s*"([A-Z_]+)"/g)].map(m => m[1])
        expect(backend.length).toBeGreaterThan(10)
        expect([...ERROR_CODES].sort()).toEqual([...new Set(backend)].sort())
    })
})
