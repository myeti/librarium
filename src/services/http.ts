import * as api from '@/services/api'
import type { HttpGetResponse } from '@/types'

/**
 * GET requests through the Rust backend (`http_get`, see docs/IPC.md).
 *
 * The webview's Content-Security-Policy only lets `fetch` reach a short list
 * of hosts. That's deliberate (a compromised page can't send data anywhere
 * else), but it also blocks hosts we can't list in advance (the OPDS
 * catalogues the user adds) and third-party APIs we'd rather not add to it
 * (archive.org). Rust has no CSP, so those requests go through it; it
 * enforces http(s) only, a timeout and a size cap.
 */

/** The response was 2xx but its body isn't the expected format (e.g. HTML or XML instead of JSON). */
export class HttpFormatError extends Error {
    constructor(readonly url: string, readonly contentType: string | null) {
        super(`Unexpected response format${contentType ? ` (${contentType})` : ''}`)
        this.name = 'HttpFormatError'
    }
}

/** A response arrived but its status wasn't 2xx. */
export class HttpStatusError extends Error {
    constructor(readonly status: number, readonly url: string) {
        super(`HTTP ${status}`)
        this.name = 'HttpStatusError'
    }
}

export interface HttpGetOptions {
    headers?: Record<string, string>
    maxBytes?: number
    /**
     * The Rust request can't be cancelled from here, but an aborted signal
     * makes the call reject with an `AbortError` (like `fetch`) and its
     * result is ignored, so callers can treat both the same way.
     */
    signal?: AbortSignal
}

function abortError(): DOMException {
    return new DOMException('The request was aborted', 'AbortError')
}

export async function httpGet(url: string, options: HttpGetOptions = {}): Promise<HttpGetResponse> {
    const { headers, maxBytes, signal } = options
    if (signal?.aborted) { throw abortError() }
    const res = await api.httpGet(url, headers, maxBytes)
    if (signal?.aborted) { throw abortError() }
    return res
}

/** GET a JSON document; rejects on network errors, non-2xx statuses and invalid JSON. */
export async function httpGetJson<T = unknown>(url: string, options: HttpGetOptions = {}): Promise<T> {
    return (await httpGetJsonAt<T>(url, options)).data
}

/**
 * `httpGetJson`, with the URL the document was served from — after
 * redirects, which the backend follows — so relative links inside it (an
 * OPDS feed's `/books/12.epub`) can be resolved the way a browser would.
 * Falls back to `url` when the backend doesn't say.
 */
export async function httpGetJsonAt<T = unknown>(url: string, options: HttpGetOptions = {}): Promise<{ data: T, url: string }> {
    const res = await httpGet(url, options)
    if (res.status < 200 || res.status >= 300) { throw new HttpStatusError(res.status, url) }
    let data: T
    try {
        data = JSON.parse(res.body) as T
    } catch {
        throw new HttpFormatError(url, res.contentType)
    }
    return { data, url: res.finalUrl || url }
}
