/**
 * Text normalisation for *comparing* titles and author names (never for
 * display). Two strings that a person would call "the same" should normalise
 * to the same value: case, accents, punctuation and spacing are ignored, and
 * non-Latin scripts are kept (a Latin-only `[^a-z]` filter would turn every
 * Cyrillic or CJK title into the same empty string).
 */
export function normalizeForMatch(s: string | null | undefined): string {
    if (!s) { return '' }
    return s
        // NFKD splits "é" into "e" + a combining accent, which the next step drops.
        .normalize('NFKD')
        .replace(/\p{M}+/gu, '')
        .toLowerCase()
        // Anything that isn't a letter or a digit (in any script) separates words.
        .replace(/[^\p{L}\p{N}]+/gu, ' ')
        .trim()
}

/** The words of a string after {@link normalizeForMatch}. */
export function matchTokens(s: string | null | undefined): string[] {
    const n = normalizeForMatch(s)
    return n ? n.split(' ') : []
}
