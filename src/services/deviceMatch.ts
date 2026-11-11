/**
 * Matching library books against the books found on an e-reader, for the
 * "already on device" marks.
 *
 * The books Librarium sends carry their library id on the device
 * (`DeviceBook.libraryId`, from the device manifest or the file itself), so
 * those are matched by id: certain, and never confused with another book
 * that has the same title. Only device books without a known id (copied
 * by hand, by another tool, or from another user's library) fall back
 * on a strict match: the same title AND author, or the same file name.
 *
 * The backend doesn't need any of this to place or replace a re-sent book:
 * it finds the previous copy through its manifest (docs/IPC.md).
 */
import type { Book, DeviceBook } from '@/types'
import { matchTokens, normalizeForMatch } from '@/utils/text'

/** Same title, ignoring case, accents, punctuation and spacing. */
export function sameTitle(a: string | null | undefined, b: string | null | undefined): boolean {
    const na = normalizeForMatch(a)
    return na !== '' && na === normalizeForMatch(b)
}

/**
 * Same author, ignoring case, accents and word order, so "Herbert, Frank"
 * (the sort form many devices and Calibre use) equals "Frank Herbert".
 */
export function sameAuthor(a: string | null | undefined, b: string | null | undefined): boolean {
    const ta = matchTokens(a).sort().join(' ')
    return ta !== '' && ta === matchTokens(b).sort().join(' ')
}

/** Whether a device book is the same work as a library book (title and author). */
export function isSameBook(book: Book, deviceBook: DeviceBook): boolean {
    return sameTitle(book.metadata.title, deviceBook.title)
        && sameAuthor(book.metadata.author, deviceBook.author)
}

/**
 * The key under which `sameTitle` and `sameAuthor` both hold: the normalised
 * title and the sorted author words. Null when either is unknown (an empty
 * title or author never matches anything).
 */
export function bookMatchKey(title: string | null | undefined, author: string | null | undefined): string | null {
    const t = normalizeForMatch(title)
    const a = matchTokens(author).sort().join(' ')
    return t && a ? `${t}\u0000${a}` : null
}

/** A file name without its last extension, lower-cased: how the device's files are compared. */
export function fileStem(filename: string): string {
    return filename.replace(/\.[^.]+$/, '').toLowerCase()
}

/**
 * What is on the device, indexed so "is this library book already there?"
 * is two lookups instead of a scan of the device list per book (the library
 * list asks it for every row it shows).
 */
export class DeviceIndex {
    private readonly ids = new Set<string>()
    private readonly keys = new Set<string>()
    private readonly stems = new Set<string>()

    /**
     * @param deviceBooks what `list_device_books` returned.
     * @param libraryIds the ids of the open library's books. A device book
     *   whose `libraryId` is one of them is that book and nothing else; one
     *   whose id isn't (another user's library) is matched like a book
     *   without an id. Without this set, every device book with an id is
     *   matched by id only.
     */
    constructor(deviceBooks: readonly DeviceBook[], libraryIds?: ReadonlySet<string>) {
        for (const db of deviceBooks) {
            if (db.libraryId) {
                this.ids.add(db.libraryId)
                if (!libraryIds || libraryIds.has(db.libraryId)) { continue }
            }
            const key = bookMatchKey(db.title, db.author)
            if (key) { this.keys.add(key) }
            this.stems.add(fileStem(db.filename))
        }
    }

    /**
     * "Already on device": a device book carries this book's id; else a
     * device book without a known id has the same title AND author (see
     * `isSameBook`) or the same file name. Title alone matched different
     * books that happen to share a title.
     */
    has(book: Book): boolean {
        if (this.ids.has(book.id)) { return true }
        const key = bookMatchKey(book.metadata.title, book.metadata.author)
        return (key !== null && this.keys.has(key)) || this.stems.has(fileStem(book.filename))
    }
}
