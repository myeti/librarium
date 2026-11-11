import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { openLibraryProvider } from './openLibrary'

function answer(body: unknown, status = 200): Response {
    return new Response(JSON.stringify(body), { status })
}

/** Routes fetched URLs to answers by path; unknown URLs fail the test. */
function stubOpenLibrary(routes: Record<string, unknown>) {
    const fetchMock = vi.fn((url: string) => {
        const { pathname } = new URL(url)
        if (!(pathname in routes)) { return Promise.reject(new Error(`unexpected fetch ${url}`)) }
        return Promise.resolve(answer(routes[pathname]))
    })
    vi.stubGlobal('fetch', fetchMock)
    return fetchMock
}

beforeEach(() => { vi.spyOn(console, 'log').mockImplementation(() => {}) })
afterEach(() => { vi.unstubAllGlobals() })

describe('Open Library tidy-up provider', () => {
    it('asks for both forms of an ISBN in one request, URL-encoded', async () => {
        const fetchMock = stubOpenLibrary({
            '/api/books': { 'ISBN:0441013597': { title: 'Dune', authors: [{ name: 'Frank Herbert' }] } },
        })
        const res = await openLibraryProvider.query({ isbn: '0-441-01359-7' })

        const url = new URL(String((fetchMock.mock.calls[0] as unknown[])[0]))
        expect(url.searchParams.get('bibkeys')).toBe('ISBN:9780441013593,ISBN:0441013597')
        expect(url.search).toContain('bibkeys=ISBN%3A9780441013593%2CISBN%3A0441013597')
        expect(res?.metadata).toMatchObject({ title: 'Dune', author: 'Frank Herbert', isbn: '0-441-01359-7' })
        expect(res?.coverUrl).toBe('https://covers.openlibrary.org/b/isbn/0441013597-L.jpg?default=false')
    })

    it('answers null (not an error) for an ISBN it doesn’t know, and doesn’t ask for junk', async () => {
        const fetchMock = stubOpenLibrary({ '/api/books': {} })
        await expect(openLibraryProvider.query({ isbn: '9780441013593' })).resolves.toBeNull()
        await expect(openLibraryProvider.query({ isbn: 'unknown' })).resolves.toBeNull()
        expect(fetchMock).toHaveBeenCalledTimes(1)
    })

    it('searches by title, then reads the work and an edition with an ISBN', async () => {
        stubOpenLibrary({
            '/search.json': { docs: [{ key: '/works/OL893415W', author_name: ['Frank Herbert'] }] },
            '/works/OL893415W.json': { title: 'Dune', description: { value: '<p>Spice</p>' }, subjects: ['Science fiction'] },
            '/works/OL893415W/editions.json': { entries: [{ isbn_13: ['9780441013593'], languages: [{ key: '/languages/eng' }] }] },
        })
        const res = await openLibraryProvider.query({ title: 'Dune', author: 'Frank Herbert' })
        expect(res?.metadata).toEqual({
            title: 'Dune', author: 'Frank Herbert', language: 'eng', isbn: '9780441013593',
            categories: ['Science Fiction'], description: 'Spice',
        })
    })

    it('still answers when the editions can’t be read', async () => {
        vi.stubGlobal('fetch', vi.fn((url: string) => {
            const { pathname } = new URL(url)
            if (pathname === '/search.json') { return Promise.resolve(answer({ docs: [{ key: '/works/OL1W' }] })) }
            if (pathname === '/works/OL1W.json') { return Promise.resolve(answer({ title: 'Dune' })) }
            return Promise.resolve(answer({}, 500))
        }))
        const res = await openLibraryProvider.query({ title: 'Dune' })
        expect(res?.metadata).toEqual({ title: 'Dune' })
        expect(res?.coverUrl).toBe('https://covers.openlibrary.org/b/olid/OL1W-L.jpg?default=false')
    })

    it('ignores a search result that isn’t a work (its key is put into URLs)', async () => {
        const fetchMock = stubOpenLibrary({ '/search.json': { docs: [{ key: '@evil.test/x' }] } })
        await expect(openLibraryProvider.query({ title: 'Dune' })).resolves.toBeNull()
        expect(fetchMock).toHaveBeenCalledTimes(1)
    })
})
