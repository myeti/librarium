/**
 * How a book's title and author are *shown*.
 *
 * An unknown title or author is an empty string in the library (the
 * backend never stores placeholders, and never writes them into files).
 * Wherever a title or author is displayed — cards, list rows, the detail
 * panel, dialogs, toasts — go through these helpers so an empty value reads
 * as a localised "Unknown title" / "Titre inconnu" instead of a blank.
 *
 * Don't use them for anything that is *sent* (searches, metadata saves,
 * device matching): there the empty string is the truth.
 */
import type { Book, BookMetadata } from '@/types'
import type { TextKey } from '@/i18n'

/** The translation function: `t` from `@/i18n`, or `$t` in a template. */
export type Translate = (key: TextKey) => string

/** A library book, or just its metadata. */
export type HasTitleAndAuthor = Pick<Book, 'metadata'> | Pick<BookMetadata, 'title' | 'author'>

function metadataOf(book: HasTitleAndAuthor): Partial<Pick<BookMetadata, 'title' | 'author'>> {
    return 'metadata' in book ? book.metadata : book
}

/** Whether a text value counts as unknown: absent, empty or only whitespace. */
export function isBlank(value: string | null | undefined): boolean {
    return value === null || value === undefined || value.trim() === ''
}

/** The book's title, or the localised "Unknown title". */
export function displayTitle(book: HasTitleAndAuthor, t: Translate): string {
    const title = metadataOf(book).title
    return isBlank(title) ? t('book.unknownTitle') : title!.trim()
}

/** The book's author, or the localised "Unknown author". */
export function displayAuthor(book: HasTitleAndAuthor, t: Translate): string {
    const author = metadataOf(book).author
    return isBlank(author) ? t('book.unknownAuthor') : author!.trim()
}

/**
 * Sort order for values where "unknown" must come last *whatever the
 * direction*: a blank sorts after any non-blank value. Returns `null` when
 * neither or both are blank, so the caller compares them as usual.
 */
export function blanksLast(a: unknown, b: unknown): number | null {
    const blankA = typeof a === 'string' && isBlank(a)
    const blankB = typeof b === 'string' && isBlank(b)
    if (blankA === blankB) { return null }
    return blankA ? 1 : -1
}

/**
 * "Dune #2", or just "Dune" when the volume isn't known (never "Dune #"),
 * or null when the book isn't in a series.
 */
export function seriesLabel(book: Pick<Book, 'metadata'>): string | null {
    const { series, seriesIndex } = book.metadata
    if (isBlank(series)) { return null }
    const hasIndex = typeof seriesIndex === 'number' && Number.isFinite(seriesIndex)
    return hasIndex ? `${series!.trim()} #${seriesIndex}` : series!.trim()
}
