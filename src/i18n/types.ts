/**
 * Types of the translation files (en.ts is the reference, fr.ts must have
 * the same shape) and of the keys `t()` accepts.
 *
 * A message is either a plain string, or — when it depends on a count — a
 * {@link PluralMessage}: one string per CLDR plural category, picked with
 * `Intl.PluralRules` from the `n` parameter. A section (nested object) must
 * therefore never contain a key named `other`: that is how a plural message
 * is told apart from a section.
 */

/** CLDR plural categories (what `Intl.PluralRules.select()` returns). */
export type PluralCategory = 'zero' | 'one' | 'two' | 'few' | 'many' | 'other'

/**
 * A message that depends on a count `{n}`. `other` is required and used for
 * any category a language doesn't spell out (French `many`, for example).
 */
export type PluralMessage = Partial<Record<PluralCategory, string>> & { other: string }

/** The shape every translation file has: sections of messages, nested. */
export interface MessageTree {
    [key: string]: string | PluralMessage | MessageTree
}

/** `prefix.key`, or `key` at the top level. */
type Join<P extends string, K extends string> = P extends '' ? K : `${P}.${K}`

/** Dotted paths of the plain-string messages of `T`. */
export type TextKeys<T, P extends string = ''> = {
    [K in keyof T & string]: T[K] extends string
        ? Join<P, K>
        : T[K] extends { other: string }
            ? never
            : TextKeys<T[K], Join<P, K>>
}[keyof T & string]

/** Dotted paths of the plural messages of `T`. */
export type PluralKeys<T, P extends string = ''> = {
    [K in keyof T & string]: T[K] extends string
        ? never
        : T[K] extends { other: string }
            ? Join<P, K>
            : PluralKeys<T[K], Join<P, K>>
}[keyof T & string]

/**
 * A translation of the reference file `T`: the same sections and keys, a
 * string for every string and a plural message (with that language's
 * categories) for every plural message.
 */
export type Translation<T> = {
    [K in keyof T]: T[K] extends string
        ? string
        : T[K] extends { other: string }
            ? PluralMessage
            : Translation<T[K]>
}

/** Values substituted for `{name}` placeholders. */
export type TranslationParams = Record<string, string | number>
