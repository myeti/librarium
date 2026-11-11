/**
 * Finding books that are probably the same work twice in the library.
 *
 * Two passes:
 * 1. **Same ISBN.** ISBNs are compared as ISBN-13 (an ISBN-10 and its
 *    ISBN-13 twin are the same book), ignoring hyphens and spaces.
 * 2. **Same title and author**, among the books the first pass didn't group.
 *    Titles and authors are normalised in a Unicode-aware way
 *    (`normalizeForMatch`: case, accents, punctuation and spacing ignored,
 *    letters of every script kept), and author names compare word by word in
 *    any order, so "Herbert, Frank" equals "Frank Herbert". A book whose
 *    title is unknown or reduces to nothing is never grouped: unknown isn't
 *    "the same".
 *
 * The old normalisation kept only `\w` (ASCII letters and digits), so every
 * Cyrillic, Greek, Arabic or CJK title collapsed to the same empty key and
 * all those books were offered for deletion as "duplicates" of each other.
 */
import type { Book } from '@/types'
import { isbnKey } from '@/utils/isbn'
import { matchTokens, normalizeForMatch } from '@/utils/text'

export interface DuplicateGroup {
    /** Stable for the same books: `isbn:<isbn13>` or `title:<title>::<author>`. */
    key: string
    reason: 'isbn' | 'titleAuthor'
    /** The shared ISBN (as the first book writes it), for `reason: 'isbn'`. */
    isbn?: string
    /** Two or more books, in library order. */
    books: Book[]
}

/** The title-and-author key of a book, or null when its title is unknown. */
export function titleAuthorKey(title: string | null | undefined, author: string | null | undefined): string | null {
    const t = normalizeForMatch(title)
    if (!t) { return null }
    const a = matchTokens(author).sort().join(' ')
    return `${t}::${a}`
}

function groupBy(books: Iterable<Book>, keyOf: (book: Book) => string | null): Map<string, Book[]> {
    const groups = new Map<string, Book[]>()
    for (const book of books) {
        const key = keyOf(book)
        if (!key) { continue }
        const group = groups.get(key)
        if (group) { group.push(book) } else { groups.set(key, [book]) }
    }
    return groups
}

export function findDuplicateGroups(books: readonly Book[]): DuplicateGroup[] {
    const result: DuplicateGroup[] = []
    const grouped = new Set<string>()

    for (const [isbn, group] of groupBy(books, b => isbnKey(b.metadata.isbn) || null)) {
        if (group.length < 2) { continue }
        result.push({ key: `isbn:${isbn}`, reason: 'isbn', isbn: group[0].metadata.isbn, books: group })
        group.forEach(b => grouped.add(b.id))
    }

    const rest = books.filter(b => !grouped.has(b.id))
    for (const [key, group] of groupBy(rest, b => titleAuthorKey(b.metadata.title, b.metadata.author))) {
        if (group.length < 2) { continue }
        result.push({ key: `title:${key}`, reason: 'titleAuthor', books: group })
    }
    return result
}
