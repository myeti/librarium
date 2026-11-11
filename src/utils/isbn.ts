/**
 * ISBNs as people and files write them ("978-2-07-036024-5", "2070360245",
 * "ISBN 0-441-01359-7") turned into forms that can be compared and looked up.
 *
 * Every ISBN-10 has an ISBN-13 twin ("978" + the first nine digits + a new
 * check digit), and the two name the same book: duplicate detection compares
 * the ISBN-13, and metadata lookups ask for both.
 */

/** The digits of an ISBN, plus a final "X" (an ISBN-10 check digit worth 10). */
export function cleanIsbn(raw: string | null | undefined): string {
    if (!raw) { return '' }
    const s = raw.toUpperCase().replace(/[^0-9X]/g, '')
    // An X is only meaningful as the last character of an ISBN-10.
    return s.replace(/X(?!$)/g, '')
}

export function isValidIsbn10(s: string): boolean {
    if (!/^\d{9}[\dX]$/.test(s)) { return false }
    let sum = 0
    for (let i = 0; i < 10; i++) {
        const digit = s[i] === 'X' ? 10 : Number(s[i])
        sum += digit * (10 - i)
    }
    return sum % 11 === 0
}

export function isValidIsbn13(s: string): boolean {
    if (!/^\d{13}$/.test(s)) { return false }
    let sum = 0
    for (let i = 0; i < 13; i++) {
        sum += Number(s[i]) * (i % 2 === 0 ? 1 : 3)
    }
    return sum % 10 === 0
}

/** The ISBN-13 of a valid ISBN-10. */
export function isbn10To13(isbn10: string): string {
    const body = '978' + isbn10.slice(0, 9)
    let sum = 0
    for (let i = 0; i < 12; i++) {
        sum += Number(body[i]) * (i % 2 === 0 ? 1 : 3)
    }
    return body + String((10 - (sum % 10)) % 10)
}

/** The ISBN-10 of a valid ISBN-13, or null when it has none (979- prefixes). */
export function isbn13To10(isbn13: string): string | null {
    if (!isbn13.startsWith('978')) { return null }
    const body = isbn13.slice(3, 12)
    let sum = 0
    for (let i = 0; i < 9; i++) {
        sum += Number(body[i]) * (10 - i)
    }
    const check = (11 - (sum % 11)) % 11
    return body + (check === 10 ? 'X' : String(check))
}

/** The ISBN-13 of `raw` when it is a valid ISBN-10 or ISBN-13, else null. */
export function toIsbn13(raw: string | null | undefined): string | null {
    const s = cleanIsbn(raw)
    if (isValidIsbn13(s)) { return s }
    if (isValidIsbn10(s)) { return isbn10To13(s) }
    return null
}

/**
 * The forms to look a book up by, best first:
 * - a valid ISBN gives its ISBN-13 and, when it has one, its ISBN-10
 *   (catalogues index some books under one form only);
 * - a value of the right length with a wrong check digit is tried as written
 *   (publishers do print those), without converting it;
 * - anything else gives nothing: it isn't an ISBN, so don't ask for it.
 */
export function isbnLookupForms(raw: string | null | undefined): string[] {
    const s = cleanIsbn(raw)
    const isbn13 = toIsbn13(s)
    if (isbn13) {
        const isbn10 = isbn13To10(isbn13)
        return isbn10 ? [isbn13, isbn10] : [isbn13]
    }
    if (/^\d{13}$/.test(s) || /^\d{9}[\dX]$/.test(s)) { return [s] }
    return []
}

/**
 * A key under which two ISBNs of the same book are equal: the ISBN-13 when
 * valid, else the cleaned value (so two books carrying the same odd value
 * still match), or '' when there is nothing.
 */
export function isbnKey(raw: string | null | undefined): string {
    return toIsbn13(raw) ?? cleanIsbn(raw)
}
