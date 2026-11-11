/**
 * Random passwords for the OPDS server's LAN mode.
 *
 * The password is typed by hand into an e-reader app (KOReader, Moon+…), so
 * the alphabet leaves out characters that are easy to confuse (0/O, 1/l/I)
 * and anything an on-screen keyboard hides behind a second page. 20 symbols
 * from 56 give about 116 bits of entropy, far more than an attacker on the
 * same Wi-Fi can brute-force over HTTP.
 */
export const PASSWORD_ALPHABET = 'abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789'
export const DEFAULT_PASSWORD_LENGTH = 20

/**
 * Generates a password with `crypto.getRandomValues` (never `Math.random`,
 * which is predictable). Rejection sampling avoids modulo bias: a random byte
 * is only used if it falls below the largest multiple of the alphabet size,
 * so every symbol is exactly equally likely.
 */
export function generatePassword(
    length = DEFAULT_PASSWORD_LENGTH,
    alphabet = PASSWORD_ALPHABET,
): string {
    if (length <= 0) { return '' }
    if (alphabet.length < 2 || alphabet.length > 256) {
        throw new Error('generatePassword: the alphabet must have 2 to 256 symbols')
    }
    const limit = 256 - (256 % alphabet.length)
    let out = ''
    const buf = new Uint8Array(length * 2)
    while (out.length < length) {
        crypto.getRandomValues(buf)
        for (const byte of buf) {
            if (byte < limit) { out += alphabet[byte % alphabet.length] }
            if (out.length === length) { break }
        }
    }
    return out
}
