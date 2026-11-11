/**
 * Helping the user choose which copy of a duplicate to keep (the Duplicates
 * review, components/DuplicatesModal.vue). Groups themselves come from
 * services/duplicates.ts.
 *
 * Each copy is described by what tells copies apart: format, file size,
 * date added, cover, and how complete its metadata is. One copy per group is
 * *suggested*: the one a careful user would most likely keep. Ranking, best
 * first:
 *
 * 1. **More complete metadata** (more of COMPLETENESS_FIELDS filled in, the
 *    cover included): edits made in Librarium, or a richer file.
 * 2. **A better format** for an ebook library: EPUB (reflowable, editable
 *    metadata, what Kobos read), then AZW3, MOBI, PDF, CBZ.
 * 3. **Has a cover.**
 * 4. **Added first**: the original, likely the one already sent to an
 *    e-reader.
 * 5. Library order (stable: the same input always suggests the same copy).
 *
 * The suggestion is only preselected; the user can pick another copy or
 * keep them all.
 */
import type { Book, BookFormat, BookMetadata } from '@/types'
import { isBlank } from '@/utils/bookDisplay'
import type { DuplicateGroup } from './duplicates'

/** The details counted for completeness, in the order they are listed as missing. */
export const COMPLETENESS_FIELDS = ['author', 'series', 'language', 'isbn', 'description', 'categories', 'cover'] as const
export type CompletenessField = typeof COMPLETENESS_FIELDS[number]

/** Best first. Formats not listed rank last. */
const FORMAT_RANK: readonly BookFormat[] = ['epub', 'azw3', 'mobi', 'pdf', 'cbz']

export interface CopyFacts {
    /** Details filled in, out of COMPLETENESS_FIELDS.length. */
    filled: number
    total: number
    /** The details this copy lacks, in COMPLETENESS_FIELDS order. */
    missing: CompletenessField[]
    hasCover: boolean
}

function filled(metadata: BookMetadata, field: Exclude<CompletenessField, 'cover'>): boolean {
    if (field === 'categories') { return (metadata.categories?.length ?? 0) > 0 }
    return !isBlank(metadata[field])
}

/** What a copy has and lacks. `hasCover` comes from the library store (covers live apart from books). */
export function copyFacts(book: Book, hasCover: boolean): CopyFacts {
    const missing = COMPLETENESS_FIELDS.filter(field =>
        field === 'cover' ? !hasCover : !filled(book.metadata, field))
    return {
        filled: COMPLETENESS_FIELDS.length - missing.length,
        total: COMPLETENESS_FIELDS.length,
        missing,
        hasCover,
    }
}

function formatRank(format: BookFormat): number {
    const i = FORMAT_RANK.indexOf(format)
    return i === -1 ? FORMAT_RANK.length : i
}

function addedAt(book: Book): number {
    const t = Date.parse(book.dateAdded)
    return Number.isNaN(t) ? Infinity : t
}

/** Why a copy is suggested: the first criterion that set it apart from the runner-up. */
export type SuggestionReason = 'complete' | 'format' | 'cover' | 'oldest' | 'first'

export interface Suggestion {
    bookId: string
    reason: SuggestionReason
}

/**
 * The copy to suggest keeping in `group`, and why. `hasCover(id)` says
 * whether a book has a cover (unknown counts as none: a cover not loaded
 * yet can't win a tie it might not deserve).
 */
export function suggestKeeper(group: DuplicateGroup, hasCover: (bookId: string) => boolean): Suggestion | null {
    if (group.books.length === 0) { return null }
    const ranked = group.books.map((book, index) => {
        const facts = copyFacts(book, hasCover(book.id))
        return {
            book,
            keys: [-facts.filled, formatRank(book.format), facts.hasCover ? 0 : 1, addedAt(book), index] as const,
        }
    })
    ranked.sort((a, b) => {
        for (let i = 0; i < a.keys.length; i++) {
            if (a.keys[i] !== b.keys[i]) { return a.keys[i] - b.keys[i] }
        }
        return 0
    })
    const [best, runnerUp] = ranked
    const reasons: SuggestionReason[] = ['complete', 'format', 'cover', 'oldest', 'first']
    let reason: SuggestionReason = 'first'
    if (runnerUp) {
        const differs = best.keys.findIndex((key, i) => key !== runnerUp.keys[i])
        reason = reasons[differs] ?? 'first'
    }
    return { bookId: best.book.id, reason }
}

/**
 * The books to act on: in every group where a copy is kept, all the other
 * copies. Groups without a kept copy are skipped (nothing happens to them).
 * `keptIds` holds at most one book per group; extra ids are ignored.
 */
export function copiesToRemove(groups: readonly DuplicateGroup[], keptIds: ReadonlySet<string>): string[] {
    return groups.flatMap(group => {
        const kept = group.books.filter(b => keptIds.has(b.id))
        if (kept.length !== 1) { return [] }
        return group.books.filter(b => b.id !== kept[0].id).map(b => b.id)
    })
}
