/**
 * JSON requests to the metadata services (Google Books, Open Library), with
 * the safety nets a batch tidy-up of hundreds of books needs:
 *
 * - **Timeout.** Each attempt is aborted after `timeoutMs` (10 s by default):
 *   a hung connection must not stall a batch forever.
 * - **Retry with backoff** on "slow down" answers only — HTTP 429 (too many
 *   requests) and 503 (service unavailable) — at most `retries` times (2).
 *   The wait is the server's `Retry-After` when it sends one (seconds or an
 *   HTTP date), else 1 s then 2 s. A server asking for more than
 *   MAX_RETRY_AFTER_MS is not waited for: the call fails at once.
 * - **Distinct failures.** Everything that isn't a usable answer rejects
 *   (`ProviderError`), so the caller can tell "the service is unreachable"
 *   (a rejection) from "the service doesn't know this book" (a resolved
 *   empty result).
 *
 * Other errors (network down, 4xx, 5xx other than 503, invalid JSON) are not
 * retried: trying again right away wouldn't change the answer.
 */

export const DEFAULT_TIMEOUT_MS = 10_000
export const DEFAULT_RETRIES = 2
/** Backoff before retry n (0-based) when the server doesn't say how long to wait. */
export const BACKOFF_MS = [1_000, 2_000]
/** Never wait longer than this between attempts. */
export const MAX_RETRY_AFTER_MS = 10_000

const RETRY_STATUSES = new Set([429, 503])

export type ProviderErrorKind = 'timeout' | 'network' | 'http' | 'format'

/** A request that didn't produce a usable answer (the service counts as unreachable). */
export class ProviderError extends Error {
    constructor(
        readonly kind: ProviderErrorKind,
        message: string,
        readonly status?: number,
    ) {
        super(message)
        this.name = 'ProviderError'
    }
}

export interface FetchJsonOptions {
    /** Service name for error messages ("Google Books"). Never put the URL there: it may carry an API key. */
    label: string
    timeoutMs?: number
    retries?: number
    /** How to wait between attempts (tests replace it). */
    sleep?: (ms: number) => Promise<void>
}

export function sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms))
}

/**
 * How long a `Retry-After` header asks to wait, in ms: a number of seconds
 * or an HTTP date. Null when absent or unreadable.
 */
export function parseRetryAfter(value: string | null, now = Date.now()): number | null {
    if (!value) { return null }
    const trimmed = value.trim()
    if (/^\d+$/.test(trimmed)) { return Number(trimmed) * 1000 }
    const date = Date.parse(trimmed)
    return Number.isNaN(date) ? null : Math.max(0, date - now)
}

interface Attempt {
    status: number
    retryAfter: string | null
    /** The parsed body of a 2xx answer. */
    body?: unknown
}

/**
 * One attempt. The timeout covers the whole exchange, headers and body:
 * a server can also stall half-way through the body.
 */
async function attemptOnce(url: string, timeoutMs: number, label: string): Promise<Attempt> {
    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), timeoutMs)
    const timedOut = () => new ProviderError('timeout', `${label} did not answer within ${Math.round(timeoutMs / 1000)} s`)
    try {
        let res: Response
        try {
            res = await fetch(url, { signal: controller.signal, headers: { Accept: 'application/json' } })
        } catch (e) {
            if (controller.signal.aborted) { throw timedOut() }
            throw new ProviderError('network', `${label} could not be reached: ${e instanceof Error ? e.message : String(e)}`)
        }
        if (!res.ok) {
            return { status: res.status, retryAfter: res.headers.get('Retry-After') }
        }
        try {
            return { status: res.status, retryAfter: null, body: await res.json() as unknown }
        } catch {
            if (controller.signal.aborted) { throw timedOut() }
            throw new ProviderError('format', `${label} sent an answer that isn't JSON`)
        }
    } finally {
        clearTimeout(timer)
    }
}

/**
 * GETs `url` and parses the JSON answer. Resolves with the parsed body for a
 * 2xx answer, and with `null` for a 404 when `notFoundIsEmpty` is set (some
 * APIs answer "no such book" that way); rejects with a ProviderError
 * otherwise (see the module comment for timeouts and retries).
 */
export async function fetchJson<T>(url: string, options: FetchJsonOptions & { notFoundIsEmpty: true }): Promise<T | null>
export async function fetchJson<T>(url: string, options: FetchJsonOptions & { notFoundIsEmpty?: false }): Promise<T>
export async function fetchJson<T>(url: string, options: FetchJsonOptions & { notFoundIsEmpty?: boolean }): Promise<T | null> {
    const { label, timeoutMs = DEFAULT_TIMEOUT_MS, retries = DEFAULT_RETRIES } = options
    const wait = options.sleep ?? sleep

    for (let attempt = 0; ; attempt++) {
        const res = await attemptOnce(url, timeoutMs, label)
        if (res.status >= 200 && res.status < 300) { return res.body as T }

        if (RETRY_STATUSES.has(res.status) && attempt < retries) {
            const asked = parseRetryAfter(res.retryAfter)
            const delay = asked ?? BACKOFF_MS[Math.min(attempt, BACKOFF_MS.length - 1)]
            if (delay <= MAX_RETRY_AFTER_MS) {
                await wait(delay)
                continue
            }
        }
        if (res.status === 404 && options.notFoundIsEmpty) { return null }
        throw new ProviderError('http', `${label} answered HTTP ${res.status}`, res.status)
    }
}
