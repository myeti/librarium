import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { googleBooksProvider, googleBooksQuery } from './googleBooks'
import { useSettingsStore } from '@/stores/settings'

const KEY = 'AIzaSyTEST-secret-key_123'

function answer(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
    return new Response(JSON.stringify(body), { status, headers })
}

/** The `q` parameter of the n-th request made with `fetchMock`. */
function queryOf(fetchMock: ReturnType<typeof vi.fn>, n = 0): string | null {
    return new URL(String((fetchMock.mock.calls[n] as unknown[])[0])).searchParams.get('q')
}

describe('Google Books tidy-up provider', () => {
    beforeEach(() => {
        setActivePinia(createPinia())
        useSettingsStore().googleBooksApiKey = KEY
        vi.spyOn(console, 'log').mockImplementation(() => {})
    })

    afterEach(() => {
        vi.unstubAllGlobals()
        vi.useRealTimers()
    })

    it('sends the API key but never logs it', async () => {
        const fetchMock = vi.fn(() => Promise.resolve(answer({ items: [{ volumeInfo: { title: 'Dune' } }] })))
        vi.stubGlobal('fetch', fetchMock)
        const log = vi.spyOn(console, 'log').mockImplementation(() => {})

        await googleBooksProvider.query({ title: 'Dune', author: 'Frank Herbert' })

        expect(String((fetchMock.mock.calls[0] as unknown[])[0])).toContain(`key=${KEY}`)
        const logged = log.mock.calls.flat().map(String).join('\n')
        expect(logged).toContain('googleapis.com')
        expect(logged).not.toContain(KEY)
        expect(logged).toContain('key=[redacted]')
    })

    it('quotes the title and author, so every word of them must match', async () => {
        const fetchMock = vi.fn(() => Promise.resolve(answer({ items: [] })))
        vi.stubGlobal('fetch', fetchMock)
        await googleBooksProvider.query({ title: 'The Hobbit', author: 'J. R. R. Tolkien' })
        expect(queryOf(fetchMock)).toBe('intitle:"The Hobbit" inauthor:"J. R. R. Tolkien"')
        // A quote inside a title can't end the phrase early.
        expect(googleBooksQuery({ title: 'Say "Hi"' })).toBe('intitle:"Say Hi"')
    })

    it('looks an ISBN up by its ISBN-13, cleaned', () => {
        expect(googleBooksQuery({ isbn: '0-441-01359-7' })).toBe('isbn:9780441013593')
        expect(googleBooksQuery({ isbn: 'n/a' })).toBeNull()
    })

    it('omits missing fields instead of returning undefined values', async () => {
        vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(answer({ items: [{ volumeInfo: { title: 'Dune' } }] }))))
        const res = await googleBooksProvider.query({ title: 'Dune' })
        expect(res?.metadata).toEqual({ title: 'Dune' })
    })

    it('answers "no match" (null) when Google has nothing', async () => {
        vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(answer({ totalItems: 0 }))))
        await expect(googleBooksProvider.query({ title: 'Nothing like it' })).resolves.toBeNull()
    })

    it('retries a rate-limited request, then answers', async () => {
        vi.useFakeTimers()
        const fetchMock = vi.fn()
            .mockResolvedValueOnce(answer({}, 429))
            .mockResolvedValueOnce(answer({ items: [{ volumeInfo: { title: 'Dune' } }] }))
        vi.stubGlobal('fetch', fetchMock)
        const pending = googleBooksProvider.query({ title: 'Dune' })
        await vi.runAllTimersAsync()
        await expect(pending).resolves.toMatchObject({ metadata: { title: 'Dune' } })
        expect(fetchMock).toHaveBeenCalledTimes(2)
    })

    it('treats an HTTP error as a failure, not as "no match"', async () => {
        vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(new Response('quota', { status: 403 }))))
        await expect(googleBooksProvider.query({ title: 'Dune' })).rejects.toThrow('Google Books answered HTTP 403')
    })
})
