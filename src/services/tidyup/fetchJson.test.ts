import { afterEach, describe, expect, it, vi } from 'vitest'
import { fetchJson, parseRetryAfter, ProviderError, DEFAULT_TIMEOUT_MS } from './fetchJson'

function json(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
    return new Response(JSON.stringify(body), { status, headers })
}

/** A `sleep` that records the waits instead of waiting. */
function recordingSleep() {
    const waits: number[] = []
    return { waits, sleep: (ms: number) => { waits.push(ms); return Promise.resolve() } }
}

afterEach(() => {
    vi.unstubAllGlobals()
    vi.useRealTimers()
})

describe('fetchJson', () => {
    it('returns the parsed body of a 2xx answer', async () => {
        vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(json({ ok: 1 }))))
        await expect(fetchJson('https://x.test/', { label: 'X' })).resolves.toEqual({ ok: 1 })
    })

    it('retries a 429 after the Retry-After the server asked for, then succeeds', async () => {
        const fetchMock = vi.fn()
            .mockResolvedValueOnce(json({}, 429, { 'Retry-After': '3' }))
            .mockResolvedValueOnce(json({ ok: 1 }))
        vi.stubGlobal('fetch', fetchMock)
        const { waits, sleep } = recordingSleep()
        await expect(fetchJson('https://x.test/', { label: 'X', sleep })).resolves.toEqual({ ok: 1 })
        expect(fetchMock).toHaveBeenCalledTimes(2)
        expect(waits).toEqual([3000])
    })

    it('backs off 1 s then 2 s on 503 without Retry-After, and gives up after 2 retries', async () => {
        const fetchMock = vi.fn(() => Promise.resolve(json({}, 503)))
        vi.stubGlobal('fetch', fetchMock)
        const { waits, sleep } = recordingSleep()
        const failure = fetchJson('https://x.test/', { label: 'Open Library', sleep })
        await expect(failure).rejects.toBeInstanceOf(ProviderError)
        await expect(failure).rejects.toMatchObject({ kind: 'http', status: 503, message: 'Open Library answered HTTP 503' })
        expect(fetchMock).toHaveBeenCalledTimes(3)
        expect(waits).toEqual([1000, 2000])
    })

    it('does not wait for a Retry-After longer than 10 s', async () => {
        const fetchMock = vi.fn(() => Promise.resolve(json({}, 429, { 'Retry-After': '120' })))
        vi.stubGlobal('fetch', fetchMock)
        const { waits, sleep } = recordingSleep()
        await expect(fetchJson('https://x.test/', { label: 'X', sleep })).rejects.toMatchObject({ status: 429 })
        expect(fetchMock).toHaveBeenCalledTimes(1)
        expect(waits).toEqual([])
    })

    it('does not retry other errors', async () => {
        const fetchMock = vi.fn(() => Promise.resolve(json({}, 500)))
        vi.stubGlobal('fetch', fetchMock)
        await expect(fetchJson('https://x.test/', { label: 'X', sleep: () => Promise.resolve() })).rejects.toMatchObject({ status: 500 })
        expect(fetchMock).toHaveBeenCalledTimes(1)
    })

    it('reads a 404 as "nothing" only when asked to', async () => {
        vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(json({}, 404))))
        await expect(fetchJson('https://x.test/', { label: 'X', notFoundIsEmpty: true })).resolves.toBeNull()
        await expect(fetchJson('https://x.test/', { label: 'X' })).rejects.toMatchObject({ status: 404 })
    })

    it('reports a body that is not JSON', async () => {
        vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(new Response('<html>'))))
        await expect(fetchJson('https://x.test/', { label: 'X' })).rejects.toMatchObject({ kind: 'format' })
    })

    it('reports a network failure', async () => {
        vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new TypeError('Load failed'))))
        await expect(fetchJson('https://x.test/', { label: 'X' })).rejects.toMatchObject({
            kind: 'network', message: 'X could not be reached: Load failed',
        })
    })

    it('aborts a request that hangs, after 10 s', async () => {
        vi.useFakeTimers()
        let signal: AbortSignal | undefined
        vi.stubGlobal('fetch', vi.fn((_url: string, init?: RequestInit) => new Promise<Response>((_resolve, reject) => {
            signal = init?.signal ?? undefined
            signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')))
        })))
        const pending = fetchJson('https://x.test/', { label: 'Google Books' })
        const assertion = expect(pending).rejects.toMatchObject({ kind: 'timeout', message: 'Google Books did not answer within 10 s' })
        await vi.advanceTimersByTimeAsync(DEFAULT_TIMEOUT_MS - 1)
        expect(signal?.aborted).toBe(false)
        await vi.advanceTimersByTimeAsync(1)
        expect(signal?.aborted).toBe(true)
        await assertion
    })
})

describe('parseRetryAfter', () => {
    it('reads seconds and HTTP dates', () => {
        expect(parseRetryAfter('2')).toBe(2000)
        expect(parseRetryAfter('Wed, 21 Oct 2026 07:28:05 GMT', Date.parse('Wed, 21 Oct 2026 07:28:00 GMT'))).toBe(5000)
        expect(parseRetryAfter(null)).toBeNull()
        expect(parseRetryAfter('soon')).toBeNull()
    })
})
