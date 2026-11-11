/**
 * Formatting for display, in the current UI language (`currentLang`): dates,
 * times and language names come from `Intl`, so they follow the language the
 * user chose ("12 mars 2026", "anglais") instead of always being US English.
 *
 * Formatters are cached per language: building an `Intl` formatter is far
 * more expensive than using one, and a list of a thousand books formats a
 * thousand dates.
 */
import { currentLang, type Lang } from '@/i18n'

function cached<T>(cache: Map<string, T>, key: string, make: () => T): T {
    let value = cache.get(key)
    if (value === undefined) {
        value = make()
        cache.set(key, value)
    }
    return value
}

const dateFormats = new Map<string, Intl.DateTimeFormat>()
const languageNames = new Map<string, Intl.DisplayNames | null>()

/**
 * A language code from a book ("en", "fr-CA", "fre"…) as a language name in
 * the UI language ("English", or "Anglais" in French). "und" (undetermined)
 * and empty values read as a dash; a code `Intl` doesn't know is shown in
 * capitals.
 */
export function fmtLanguage(code?: string, lang: Lang = currentLang.value): string {
    const c = code?.trim()
    if (!c || c.toLowerCase() === 'und') { return '—' }
    const names = cached(languageNames, lang, () => {
        try { return new Intl.DisplayNames([lang], { type: 'language', fallback: 'none' }) } catch { return null }
    })
    let name: string | undefined
    try {
        name = names?.of(c)
    } catch {
        // Not a well-formed language tag (RangeError).
        name = undefined
    }
    if (!name) { return c.toUpperCase() }
    // Shown on its own (a table cell, a label's value): capitalised, which
    // French language names aren't by default ("anglais").
    return name.charAt(0).toLocaleUpperCase(lang) + name.slice(1)
}

export function fmtLanguageShort(code?: string): string {
    if (!code || code.toLowerCase() === 'und') {return '—'}
    return code.toUpperCase()
}

/** A date ("Mar 12, 2026", "12 mars 2026"); a dash when it can't be read. */
export function fmtDate(iso: string, lang: Lang = currentLang.value): string {
    const date = new Date(iso)
    if (Number.isNaN(date.getTime())) { return '—' }
    return cached(dateFormats, `date:${lang}`, () => new Intl.DateTimeFormat(lang, { dateStyle: 'medium' })).format(date)
}

/** A time of day, hours and minutes ("3:05 PM", "15:05"). */
export function fmtTime(iso: string, lang: Lang = currentLang.value): string {
    const date = new Date(iso)
    if (Number.isNaN(date.getTime())) { return '—' }
    return cached(dateFormats, `time:${lang}`, () => new Intl.DateTimeFormat(lang, { hour: '2-digit', minute: '2-digit' })).format(date)
}

const KB = 1024
const MB = KB * 1024
const GB = MB * 1024
const TB = GB * 1024

/**
 * Human-readable size in binary units (1 KB = 1024 bytes, as Finder and
 * Explorer show for files). Devices and libraries reach gigabytes: "2.9 GB",
 * never "2956.4 MB".
 */
export function fmtSize(bytes: number): string {
    if (!Number.isFinite(bytes) || bytes < 0) {return '—'}
    if (bytes < KB) {return `${bytes} B`}
    if (bytes < MB) {return `${(bytes / KB).toFixed(0)} KB`}
    if (bytes < GB) {return `${(bytes / MB).toFixed(1)} MB`}
    if (bytes < TB) {return `${(bytes / GB).toFixed(1)} GB`}
    return `${(bytes / TB).toFixed(1)} TB`
}
