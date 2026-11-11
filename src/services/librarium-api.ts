import { analytics } from './analytics'
import { getInstallationId } from './installationId'
import { safeContextKey, safeDeviceModel, safeErrorCode } from './privacy'
import { ERROR_CODES, errorCode } from '@/utils/errors'

const API_URL = import.meta.env.VITE_API_URL ?? ''
const API_KEY = import.meta.env.VITE_API_KEY ?? ''

function apiHeaders(extra: Record<string, string> = {}): Record<string, string> {
    const h: Record<string, string> = { ...extra }
    if (API_KEY) {h['X-API-Key'] = API_KEY}
    return h
}

let _cachedVersion = 'unknown'
/** Call once on app startup so reportError can include the version without async lookups. */
export function cacheAppVersion(v: string) { _cachedVersion = v }

export interface ApiMessage {
    id: number
    translations: Record<string, { title: string; content: string }>
    constraintOs: string | null
    constraintVersion: string | null
    createdAt: string
}

/** Returns messages with id > lastId (or all if null). Returns [] on any error. */
export async function fetchMessages(lastId: number | null): Promise<ApiMessage[]> {
    if (!API_URL) {return []}
    try {
        const params = lastId != null ? `?from=${lastId}` : ''
        const res = await fetch(`${API_URL}/api/app/messages${params}`, {
            headers: apiHeaders(),
            signal: AbortSignal.timeout(5000),
        })
        if (!res.ok) {return []}
        return await res.json() as ApiMessage[]
    } catch {
        return []
    }
}

function currentOs(): string {
    const ua = navigator.userAgent
    return ua.includes('Mac') ? 'macos' : ua.includes('Win') ? 'windows' : 'linux'
}

/** The OS version as the web view's user agent states it (macOS: often frozen at 10.15.7). */
function getOsVersion(): string | null {
    const ua = navigator.userAgent
    const mac = ua.match(/Mac OS X ([\d_]+)/)
    if (mac) {return mac[1].replace(/_/g, '.')}
    const win = ua.match(/Windows NT ([\d.]+)/)
    if (win) {return win[1]}
    return null
}

/**
 * Where an error report comes from: one fixed key per place that reports
 * (PRIVACY.md lists them in words). A report carries this key, never the
 * error's message.
 */
export type ErrorReportContext =
    | 'app.startup'
    | 'users.init'
    | 'import.files'
    | 'library.trash'
    | 'library.forget'
    | 'duplicates.trash'
    | 'duplicates.forget'
    | 'details.rename'
    | 'tidyUp.book'
    | 'tidyUp.batch'
    | 'device.view'
    | 'device.list'
    | 'device.delete'
    | 'calibre.detect'

/** The backend's error category (`IO`, `NOT_FOUND`…: a fixed list), or null. */
function backendErrorCategory(cause: unknown): string | null {
    const code = errorCode(cause)
    return code && (ERROR_CODES as readonly string[]).includes(code) ? code : null
}

/**
 * The body of an error report: nothing but fixed codes, the app version, the
 * OS and the installation ID. Exported for the tests.
 */
export function errorReportBody(code: string, context: ErrorReportContext, cause?: unknown): Record<string, string | null> {
    const where = safeContextKey(context)
    const category = backendErrorCategory(cause)
    return {
        appUid:       getInstallationId(),
        version:      _cachedVersion,
        os:           currentOs(),
        osVersion:    getOsVersion(),
        errorCode:    safeErrorCode(code),
        // The server's free-text field: here only the place it happened and
        // the backend's category, e.g. "library.trash (IO)". Never the message.
        errorContent: category ? `${where} (${category})` : where,
    }
}

/**
 * Sends an error report. Fire-and-forget: never throws, never rejects.
 *
 * Error reports follow the analytics consent (decision D2): nothing is sent
 * unless the user allowed it. A report carries **no free text**: `code` (a
 * fixed code such as `IMPORT_FAILED`), `context` (a fixed key saying where
 * it happened), the backend's error category when `cause` is a backend error
 * (`IO`, `NOT_FOUND`…), the app version, the OS and its version, and the
 * installation ID. The error's message never leaves the computer: messages
 * name files ("Dune.epub can't be added to the library") and hold paths.
 */
export function reportError(code: string, context: ErrorReportContext, cause?: unknown): void {
    if (!API_URL || !analytics.hasConsent) {return}
    fetch(`${API_URL}/api/app/report/error`, {
        method: 'POST',
        headers: apiHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify(errorReportBody(code, context, cause)),
        signal: AbortSignal.timeout(5000),
    }).catch(() => {})
}

/** The e-reader feedback mentions: its brand (`kindle`, `kobo`) and model. */
export interface FeedbackDevice {
    brand: string
    model?: string | null
}

/** What goes with a feedback message, besides the message and the installation ID. */
export interface FeedbackAttachments {
    version: string
    os: string
    osVersion: string | null
    deviceBrand: string | null
    deviceModel: string | null
}

/**
 * What a feedback message is sent with (the dialog shows these values before
 * sending): the app version, the OS and its version, and the connected
 * e-reader's brand and model (through `safeDeviceModel`, so never a serial
 * number). The installation ID is added when sending.
 */
export function feedbackAttachments(device?: FeedbackDevice | null): FeedbackAttachments {
    return {
        version:     _cachedVersion,
        os:          currentOs(),
        osVersion:   getOsVersion(),
        deviceBrand: device ? (/^[a-z]{1,20}$/.test(device.brand) ? device.brand : 'unknown') : null,
        deviceModel: device?.model ? safeDeviceModel(device.model) : null,
    }
}

/**
 * Sends a user feedback message. Throws on network/server error so the
 * caller can inform the user. Feedback is the user's own act, so it doesn't
 * depend on the statistics consent; it may create the installation ID.
 */
export async function sendFeedback(message: string, device?: FeedbackDevice | null): Promise<void> {
    if (!API_URL) {throw new Error('No API URL configured')}
    const res = await fetch(`${API_URL}/api/app/report/feedback`, {
        method: 'POST',
        headers: apiHeaders({ 'Content-Type': 'application/json' }),
        body: JSON.stringify({
            appUid: getInstallationId(),
            ...feedbackAttachments(device),
            message,
        }),
        signal: AbortSignal.timeout(8000),
    })
    if (!res.ok) {throw new Error(`Server error ${res.status}`)}
}
