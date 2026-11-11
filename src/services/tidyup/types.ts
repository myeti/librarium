import type { BookMetadata } from '@/types'

export interface QueryParams {
    isbn?:   string
    title?:  string
    author?: string
    lang?:   string
}

export interface TidyupQueryResult {
    metadata: Partial<BookMetadata>
    coverUrl?: string | null
}

/** Why a tidy-up returned nothing, so the UI can say which it was. */
export type TidyupOutcome =
    /** A provider answered with usable metadata. */
    | 'found'
    /** Providers answered, but none knew this book. */
    | 'no-match'
    /** Every provider that ran failed — offline, API down, blocked. */
    | 'unreachable'
    /**
     * Providers answered, but no result's title was close enough to the
     * book's (see merge.ts `MIN_TITLE_SIMILARITY`). Only returned when the
     * caller asked for confident matches only (batch tidy-up).
     */
    | 'low-confidence'

export interface TidyupResult {
    found:    boolean
    outcome:  TidyupOutcome
    source:   string | null
    metadata: Partial<BookMetadata>
    coverUrl: string | null
    /** Title similarity between the book and the result, 0–1. */
    similarity: number
    /** Whether `similarity` reaches the threshold for unreviewed use. */
    confident: boolean
}

export interface TidyupOptions {
    /**
     * Only accept results whose title closely matches the book's. Batch
     * tidy-up sets this, because nobody reviews its results; single-book
     * tidy-up shows a review dialog (with a warning for doubtful matches).
     */
    requireConfidentMatch?: boolean
}

export interface TidyupProvider {
    readonly id: string
    /** Return false to skip this provider (e.g. missing API key). */
    isAvailable(): boolean
    query(params: QueryParams): Promise<TidyupQueryResult | null>
}
