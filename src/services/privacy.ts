/**
 * Helpers that keep personal data out of what Librarium sends to its servers
 * (error reports, analytics, feedback). PRIVACY.md, the consent dialog and
 * Settings › Privacy & analytics promise "no file names, no paths, no error
 * messages, counts and durations rounded into ranges": everything outgoing
 * goes through here.
 */

/**
 * Error reports carry no free text (decision D2, PRIVACY.md): only an error
 * code, the place where it happened and, for backend errors, the backend's
 * error category. Free text — even "scrubbed" — can hold a file name
 * ("Dune.epub can't be added to the library") or a path outside the folders
 * a scrubber knows. So the two strings a report carries must look like
 * identifiers written in the code; anything else is replaced.
 */
const ERROR_CODE_RE = /^[A-Z][A-Z0-9_]{0,63}$/
const CONTEXT_KEY_RE = /^[a-z][A-Za-z0-9]*(?:\.[a-z][A-Za-z0-9]*){0,3}$/

export const UNKNOWN_ERROR_CODE = 'UNKNOWN'
export const UNKNOWN_CONTEXT = 'unknown'

/** `IMPORT_FAILED` as is; anything that isn't an upper-case code becomes `UNKNOWN`. */
export function safeErrorCode(code: string | null | undefined): string {
    return code && ERROR_CODE_RE.test(code) ? code : UNKNOWN_ERROR_CODE
}

/** `library.trash` as is; anything that isn't a dotted key becomes `unknown`. */
export function safeContextKey(key: string | null | undefined): string {
    return key && key.length <= 64 && CONTEXT_KEY_RE.test(key) ? key : UNKNOWN_CONTEXT
}

/**
 * A device model is sent with some analytics events and with feedback
 * ("Kindle Paperwhite 5"). Older code paths could put a device serial number
 * there instead (Kobo's `.kobo/version` starts with the serial), and a serial
 * identifies a person's device. Any word that looks like a serial (8+
 * letters and digits mixed) makes the whole model 'unknown'.
 */
export function safeDeviceModel(model: string | null | undefined): string {
    if (!model) { return 'unknown' }
    const serialLike = /^(?=.*\d)(?=.*[A-Za-z])[A-Za-z0-9]{8,}$/
    if (model.length > 40 || model.split(/\s+/).some(word => serialLike.test(word))) {
        return 'unknown'
    }
    return model
}

// ── Rounding (counts and durations are never sent exactly) ────────────────

/**
 * A count, as a range: `0`, `1`, `2-5`, `6-20`, `21-100` or `100+` (more
 * than 100). Used for the number of books converted or imported, of fields
 * applied, of catalogs searched…
 */
export function bucketCount(n: number): string {
    if (!Number.isFinite(n) || n <= 0) { return '0' }
    if (n <= 1)   { return '1' }
    if (n <= 5)   { return '2-5' }
    if (n <= 20)  { return '6-20' }
    if (n <= 100) { return '21-100' }
    return '100+'
}

/** A duration in minutes, as a range: `<5`, `5-15`, `15-60` or `60+`. */
export function bucketMinutes(minutes: number): string {
    if (!Number.isFinite(minutes) || minutes < 5) { return '<5' }
    if (minutes < 15) { return '5-15' }
    if (minutes < 60) { return '15-60' }
    return '60+'
}
