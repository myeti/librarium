/**
 * Batch tidy-up: looks up several books online and applies the results
 * **without review**, so it is conservative:
 * - only results whose title closely matches the book's are used
 *   (`requireConfidentMatch`); the others count as "no confident match";
 * - only empty fields are filled, existing values are never replaced;
 * - a cover is only added to a book that has none (when in doubt, it is
 *   assumed to have one: a real cover is never replaced);
 * - a book counts as updated only once a save succeeded.
 *
 * The library and the lookup are passed in (`BatchTidyupDeps`), so the
 * rules above are tested without a backend (batch.test.ts).
 */
import type { Book, BookMetadata } from '@/types'
import type { TidyupOptions, TidyupResult } from './types'
import { fillEmptyFields } from './merge'
import { t } from '@/i18n'

export interface BatchCounts {
    updated: number
    unchanged: number
    noMatch: number
    unreachable: number
    saveFailed: number
    coverFailed: number
}

export function emptyCounts(): BatchCounts {
    return { updated: 0, unchanged: 0, noMatch: 0, unreachable: 0, saveFailed: 0, coverFailed: 0 }
}

export interface BatchTidyupDeps {
    lookup: (query: { title: string, author: string, isbn?: string }, options: TidyupOptions) => Promise<TidyupResult>
    /** The book as it is now (an earlier save may have changed it), or undefined if it left the library. */
    current: (bookId: string) => Book | undefined
    /** Whether the book has a cover; true when that can't be told. */
    hasCover: (bookId: string) => Promise<boolean>
    saveMetadata: (bookId: string, patch: Partial<BookMetadata>) => Promise<unknown>
    saveCover: (bookId: string, url: string) => Promise<unknown>
    /** Logs a failure that is only counted (console). */
    log?: (message: string, error: unknown) => void
}

/**
 * Tidies `books` one after the other, counting what happened into `counts`
 * (passed in so a caller still knows how far it got if a lookup throws).
 * Rejects only on an unexpected failure of the lookup itself.
 */
export async function tidyupBatch(books: readonly Book[], deps: BatchTidyupDeps, counts: BatchCounts = emptyCounts()): Promise<BatchCounts> {
    for (const book of books) {
        const result = await deps.lookup({
            title: book.metadata.title,
            author: book.metadata.author,
            isbn: book.metadata.isbn,
        }, { requireConfidentMatch: true })

        if (result.outcome === 'unreachable') { counts.unreachable++; continue }
        if (!result.found) { counts.noMatch++; continue }

        const current = deps.current(book.id)
        if (!current) { continue }
        const patch = fillEmptyFields(current.metadata, result.metadata)
        const addCover = !!result.coverUrl && !(await deps.hasCover(book.id).catch(() => true))

        if (Object.keys(patch).length === 0 && !addCover) { counts.unchanged++; continue }

        let saved = false
        if (Object.keys(patch).length > 0) {
            try {
                await deps.saveMetadata(book.id, patch)
                saved = true
            } catch (e) {
                deps.log?.('[tidy-up] save failed', e)
                counts.saveFailed++
                continue
            }
        }
        if (addCover && result.coverUrl) {
            try {
                await deps.saveCover(book.id, result.coverUrl)
                saved = true
            } catch (e) {
                deps.log?.('[tidy-up] cover download failed', e)
                counts.coverFailed++
            }
        }
        if (saved) { counts.updated++ }
    }
    return counts
}

/** One toast to show. */
export interface BatchMessage {
    type: 'success' | 'info' | 'warning' | 'error'
    message: string
    /** How long to show it, ms (the notifications store's default when absent). */
    duration?: number
}

/**
 * What to tell the user after a batch of `total` books: one summary toast
 * ("12 books: 8 updated, 3 unchanged, 1 without a confident match"), plus a
 * warning when covers couldn't be saved; a single error when no service
 * could be reached at all.
 */
export function batchMessages(counts: BatchCounts, total: number): BatchMessage[] {
    if (counts.unreachable === total) {
        return [{ type: 'error', message: t('tidyup.unreachable') }]
    }
    const parts: string[] = []
    if (counts.updated)     { parts.push(t('tidyup.batchUpdated', { n: counts.updated })) }
    if (counts.unchanged)   { parts.push(t('tidyup.batchUnchanged', { n: counts.unchanged })) }
    if (counts.noMatch)     { parts.push(t('tidyup.batchNoConfident', { n: counts.noMatch })) }
    if (counts.unreachable) { parts.push(t('tidyup.batchUnreachable', { n: counts.unreachable })) }
    if (counts.saveFailed)  { parts.push(t('tidyup.batchSaveFailed', { n: counts.saveFailed })) }
    const type = counts.saveFailed > 0 ? 'warning' : counts.updated > 0 ? 'success' : 'info'
    const messages: BatchMessage[] = [
        { type, message: t('tidyup.batchResult', { n: total, details: parts.join(', ') }), duration: 8000 },
    ]
    if (counts.coverFailed > 0) {
        messages.push({ type: 'warning', message: t('tidyup.batchCoverFailed', { n: counts.coverFailed }) })
    }
    return messages
}
