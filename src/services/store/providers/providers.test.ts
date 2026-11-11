import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { backendError, mockInvoke } from '@/test/tauri'
import { createOpdsCatalogProvider, resolveHref } from './opdsCatalog'
import { openLibraryStoreProvider } from './openLibrary'
import { HttpStatusError } from '@/services/http'

const json = (body: unknown, status = 200) => ({
    status, contentType: 'application/json', body: JSON.stringify(body), finalUrl: 'https://x.test/',
})

beforeEach(() => mockInvoke.reset())
afterEach(() => vi.unstubAllGlobals())

describe('OPDS catalog provider', () => {
    const source = { id: 'c1', url: 'https://books.example.org/opds/search', name: 'Example', apiKey: 'tok' }

    it('fetches through the backend (http_get), with the API key as a bearer token', async () => {
        mockInvoke.on('http_get', () => json({
            publications: [{
                metadata: { title: 'Dune', author: { name: 'Frank Herbert' }, identifier: 'urn:1' },
                links: [{ rel: 'http://opds-spec.org/acquisition', type: 'application/epub+zip', href: 'https://books.example.org/dune.epub' }],
            }],
        }))
        const books = await createOpdsCatalogProvider(source).search('dune')

        const args = mockInvoke.callsTo('http_get')[0].args
        expect(args.url).toBe('https://books.example.org/opds/search?q=dune')
        expect(args.headers).toMatchObject({ Authorization: 'Bearer tok' })
        expect(books).toHaveLength(1)
        expect(books[0]).toMatchObject({ title: 'Dune', author: 'Frank Herbert', format: 'epub' })
    })

    it('resolves relative download and cover links against the feed (after redirects)', async () => {
        mockInvoke.on('http_get', () => ({
            ...json({
                publications: [{
                    metadata: { title: 'Dune' },
                    links: [{ rel: 'http://opds-spec.org/acquisition/open-access', type: 'application/epub+zip', href: '/books/12.epub' }],
                    images: [{ href: 'covers/12.jpg', type: 'image/jpeg' }],
                }, {
                    metadata: { title: 'Messiah' },
                    links: [
                        { rel: 'http://opds-spec.org/acquisition', type: 'application/epub+zip', href: '../13.epub' },
                        { rel: 'http://opds-spec.org/image/thumbnail', type: 'image/png', href: '//cdn.example.org/13.png' },
                    ],
                }],
            }),
            // The search was redirected: links are relative to where the feed was served from.
            finalUrl: 'https://books.example.org/v2/opds/search?q=dune',
        }))
        const books = await createOpdsCatalogProvider(source).search('dune')

        expect(books.map(b => b.downloadUrl)).toEqual([
            'https://books.example.org/books/12.epub',
            'https://books.example.org/v2/13.epub',
        ])
        expect(books.map(b => b.coverUrl)).toEqual([
            'https://books.example.org/v2/opds/covers/12.jpg',
            'https://cdn.example.org/13.png',
        ])
    })

    it('names each download after its real format, and skips what the library can’t import', async () => {
        const acquisition = 'http://opds-spec.org/acquisition'
        mockInvoke.on('http_get', () => json({
            publications: [
                { metadata: { title: 'Kindle only' }, links: [{ rel: acquisition, type: 'application/x-mobipocket-ebook', href: 'https://x.test/a.mobi' }] },
                { metadata: { title: 'KF8' }, links: [{ rel: acquisition, type: 'application/vnd.amazon.ebook', href: 'https://x.test/b' }] },
                { metadata: { title: 'Comic' }, links: [{ rel: acquisition, type: 'application/vnd.comicbook+zip', href: 'https://x.test/c' }] },
                { metadata: { title: 'Paper' }, links: [{ rel: acquisition, type: 'application/pdf; charset=binary', href: 'https://x.test/d' }] },
                // No type: the extension says it.
                { metadata: { title: 'Untyped' }, links: [{ rel: acquisition, href: 'https://x.test/e.azw3?token=1' }] },
                // Formats Librarium can't import are not offered (never passed off as .pdf).
                { metadata: { title: 'FB2' }, links: [{ rel: acquisition, type: 'application/x-fictionbook+xml', href: 'https://x.test/f.fb2' }] },
                { metadata: { title: 'Web page' }, links: [{ rel: acquisition, type: 'text/html', href: 'https://x.test/g' }] },
                // Several copies: the EPUB is downloaded.
                {
                    metadata: { title: 'Both' },
                    links: [
                        { rel: acquisition, type: 'application/pdf', href: 'https://x.test/h.pdf' },
                        { rel: acquisition, type: 'application/epub+zip', href: 'https://x.test/h.epub' },
                    ],
                },
            ],
        }))
        const books = await createOpdsCatalogProvider(source).search('x')

        expect(books.map(b => [b.title, b.format, b.downloadUrl])).toEqual([
            ['Kindle only', 'mobi', 'https://x.test/a.mobi'],
            ['KF8', 'azw3', 'https://x.test/b'],
            ['Comic', 'cbz', 'https://x.test/c'],
            ['Paper', 'pdf', 'https://x.test/d'],
            ['Untyped', 'azw3', 'https://x.test/e.azw3?token=1'],
            ['Both', 'epub', 'https://x.test/h.epub'],
        ])
    })

    it('never offers a link that isn’t http(s)', () => {
        expect(resolveHref('javascript:alert(1)', 'https://x.test/feed')).toBeUndefined()
        expect(resolveHref('file:///etc/passwd', 'https://x.test/feed')).toBeUndefined()
        expect(resolveHref(undefined, 'https://x.test/feed')).toBeUndefined()
        expect(resolveHref('a b.epub', 'https://x.test/feed/')).toBe('https://x.test/feed/a%20b.epub')
    })

    it('reports an unreachable catalog as an error, not as "no results"', async () => {
        mockInvoke.on('http_get', () => json({}, 502))
        await expect(createOpdsCatalogProvider(source).search('dune')).rejects.toBeInstanceOf(HttpStatusError)

        mockInvoke.on('http_get', () => backendError('dns error'))
        await expect(createOpdsCatalogProvider(source).search('dune')).rejects.toBe('dns error')
    })
})

describe('Open Library store provider', () => {
    it('reads archive.org file lists through the backend (blocked by the CSP otherwise)', async () => {
        vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(new Response(JSON.stringify({
            docs: [{ key: '/works/OL1W', title: 'Dune', author_name: ['Frank Herbert'], ia: ['dune00herb'], cover_i: 42 }],
        })))))
        mockInvoke.on('http_get', () => json({ files: [{ name: 'dune.epub', format: 'EPUB' }], metadata: {} }))

        const books = await openLibraryStoreProvider.search('dune')

        expect(mockInvoke.callsTo('http_get')[0].args.url).toBe('https://archive.org/metadata/dune00herb')
        expect(books).toHaveLength(1)
        expect(books[0].downloadUrl).toBe('https://archive.org/download/dune00herb/dune.epub')
        expect(books[0].coverUrl).toBe('https://covers.openlibrary.org/b/id/42-M.jpg?default=false')
    })

    it('fails loudly when every archive.org lookup failed', async () => {
        vi.stubGlobal('fetch', vi.fn(() => Promise.resolve(new Response(JSON.stringify({
            docs: [{ key: '/works/OL1W', title: 'Dune', ia: ['dune00herb'] }],
        })))))
        mockInvoke.on('http_get', () => backendError('connection reset'))
        await expect(openLibraryStoreProvider.search('dune')).rejects.toBe('connection reset')
    })
})
