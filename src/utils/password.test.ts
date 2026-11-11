import { describe, expect, it, vi } from 'vitest'
import { generatePassword, PASSWORD_ALPHABET, DEFAULT_PASSWORD_LENGTH } from './password'

describe('generatePassword', () => {
    it('has the default length and only uses the alphabet', () => {
        const pw = generatePassword()
        expect(pw).toHaveLength(DEFAULT_PASSWORD_LENGTH)
        for (const ch of pw) { expect(PASSWORD_ALPHABET).toContain(ch) }
    })

    it('leaves out characters that are easy to confuse', () => {
        for (const ch of '0O1lI') { expect(PASSWORD_ALPHABET).not.toContain(ch) }
    })

    it('uses crypto.getRandomValues', () => {
        const spy = vi.spyOn(crypto, 'getRandomValues')
        generatePassword(8)
        expect(spy).toHaveBeenCalled()
    })

    it('gives different passwords on each call', () => {
        const seen = new Set(Array.from({ length: 50 }, () => generatePassword()))
        expect(seen.size).toBe(50)
    })

    it('discards bytes that would bias the result (rejection sampling)', () => {
        // Alphabet of 3: 256 % 3 = 1, so byte 255 must be rejected.
        let call = 0
        vi.spyOn(crypto, 'getRandomValues').mockImplementation(<T extends ArrayBufferView | null>(arr: T): T => {
            const bytes = arr as unknown as Uint8Array
            bytes.fill(call++ === 0 ? 255 : 4) // first batch all rejected, then 4 % 3 = 1
            return arr
        })
        expect(generatePassword(4, 'abc')).toBe('bbbb')
    })

    it('rejects alphabets it cannot sample from', () => {
        expect(() => generatePassword(4, 'a')).toThrow()
        expect(generatePassword(0)).toBe('')
    })
})
