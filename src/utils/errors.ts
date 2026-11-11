/**
 * Reading errors, whatever produced them.
 *
 * Tauri commands reject with the backend's `AppError`, serialised as
 * `{ code, message }` (see docs/IPC.md, "Errors"): `code` is stable and
 * machine-readable (`LIBRARY_UNAVAILABLE`, `NOT_FOUND`…), `message` is for
 * people. Older backends rejected with a plain string, sometimes prefixed by
 * a code (`LIBRARY_UNAVAILABLE: …`); browser APIs reject with `Error`
 * objects; some libraries throw `{ message }` objects. `String(e)` would
 * print "[object Object]" for the objects and "Error: …" for errors, so every
 * place that shows or tests an error goes through `errorMessage` /
 * `errorCode` instead.
 */

/** Codes of the backend's `AppError` (src-tauri/src/error.rs). */
export const ERROR_CODES = [
    'LIBRARY_UNAVAILABLE',
    'PERMISSION_DENIED',
    'INDEX_DAMAGED',
    'INDEX_TOO_NEW',
    'NOT_FOUND',
    'INVALID_INPUT',
    'IO',
    'NETWORK',
    'CONVERSION',
    'DEVICE',
    'CONFLICT',
    'INTERNAL',
    'OTHER',
] as const

/** A code the backend may send. New codes may appear: compare, don't switch exhaustively. */
export type ErrorCode = typeof ERROR_CODES[number]

/** The backend's "library folder not reachable" code (share not mounted, drive unplugged). */
export const LIBRARY_UNAVAILABLE: ErrorCode = 'LIBRARY_UNAVAILABLE'

/**
 * The backend's "not allowed to read the library folder" code. On macOS:
 * Librarium hasn't been allowed to read network (or removable) volumes yet,
 * in System Settings › Privacy & Security › Files and Folders; the message
 * says so. The folder is there: this is not a missing NAS.
 */
export const PERMISSION_DENIED: ErrorCode = 'PERMISSION_DENIED'

/** What a Tauri command rejects with. */
export interface AppError {
    code: string
    message: string
}

/** Legacy string errors: a code, a colon, the message. */
const LEGACY_PREFIX = /^([A-Z][A-Z_]+): /

/** Whether `e` is a `{ code, message }` error object from the backend. */
export function isAppError(e: unknown): e is AppError {
    return !!e && typeof e === 'object'
        && typeof (e as { code?: unknown }).code === 'string'
        && typeof (e as { message?: unknown }).message === 'string'
}

/** The message carried by `e`, exactly as received. */
function rawMessage(e: unknown): string {
    if (typeof e === 'string') { return e }
    if (e instanceof Error) { return e.message }
    if (e && typeof e === 'object' && 'message' in e && typeof e.message === 'string') {
        return e.message
    }
    if (e === undefined || e === null) { return '' }
    // Anything else (a number, a plain object without a message): JSON is
    // more useful in a toast than "[object Object]".
    if (typeof e === 'object') {
        try { return JSON.stringify(e) } catch { return 'Unknown error' }
    }
    if (typeof e === 'number' || typeof e === 'boolean' || typeof e === 'bigint') { return e.toString() }
    if (typeof e === 'symbol') { return e.description ?? 'Unknown error' }
    return 'Unknown error'
}

/** A legacy `CODE: message` string split in two, when the prefix is a known code. */
function splitLegacy(message: string): { code: string, rest: string } | null {
    const match = LEGACY_PREFIX.exec(message)
    if (!match || !(ERROR_CODES as readonly string[]).includes(match[1])) { return null }
    return { code: match[1], rest: message.slice(match[0].length).trim() }
}

/**
 * The machine-readable code of `e`: the backend's `code`, or the prefix of a
 * legacy `CODE: message` string (or `Error`). `null` for anything else.
 */
export function errorCode(e: unknown): string | null {
    if (isAppError(e)) { return e.code }
    if (typeof e === 'string' || e instanceof Error) {
        return splitLegacy(rawMessage(e))?.code ?? null
    }
    return null
}

/**
 * Turns anything a promise can reject with into a message a person can read.
 *
 * For backend errors that is their `message`; a legacy code prefix
 * (`LIBRARY_UNAVAILABLE: …`) is removed — test for the code with
 * `errorCode` (or helpers such as `isLibraryUnavailable`), show the rest.
 */
export function errorMessage(e: unknown): string {
    if (isAppError(e)) { return e.message }
    const message = rawMessage(e)
    return splitLegacy(message)?.rest ?? message
}

/** Whether `e` is the backend's "library folder not reachable" error. */
export function isLibraryUnavailable(e: unknown): boolean {
    return errorCode(e) === LIBRARY_UNAVAILABLE
}

/** Whether `e` is the backend's "not allowed to read the library folder" error. */
export function isPermissionDenied(e: unknown): boolean {
    return errorCode(e) === PERMISSION_DENIED
}
