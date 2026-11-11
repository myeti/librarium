import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))

// Two fake providers in the real priority order (Google Books, then Open Library).
// vi.hoisted: vi.mock factories run before the module body.
const { google, openLibrary } = vi.hoisted(() => ({
    google:      { id: 'googlebooks', isAvailable: vi.fn(() => true), query: vi.fn() },
    openLibrary: { id: 'openlibrary', isAvailable: vi.fn(() => true), query: vi.fn() },
}))
vi.mock('./providers/googleBooks', () => ({ googleBooksProvider: google }))
vi.mock('./providers/openLibrary', () => ({ openLibraryProvider: openLibrary }))

import { mockInvoke } from '@/test/tauri'
import { tidyupBook, availableTidyupSources } from './index'

const dune = { title: 'Dune', author: 'Frank Herbert' }

beforeEach(() => {
    mockInvoke.reset()
    mockInvoke.on('detect_language', () => 'en')
    google.query.mockReset()
    openLibrary.query.mockReset()
    google.isAvailable.mockReturnValue(true)
})

describe('tidyupBook', () => {
    it('returns the first confident match and strips undefined fields', async () => {
        google.query.mockResolvedValue({ metadata: { title: 'Dune', description: undefined, isbn: '9780441013593' }, coverUrl: null })
        const res = await tidyupBook(dune)
        expect(res).toMatchObject({ found: true, confident: true, source: 'googlebooks' })
        expect(res.metadata).toEqual({ title: 'Dune', isbn: '9780441013593' })
        expect(openLibrary.query).not.toHaveBeenCalled()
    })

    it('tries the next provider when the first result is a different book', async () => {
        google.query.mockResolvedValue({ metadata: { title: 'Dune Messiah' } })
        openLibrary.query.mockResolvedValue({ metadata: { title: 'Dune' } })
        const res = await tidyupBook(dune)
        expect(res).toMatchObject({ found: true, confident: true, source: 'openlibrary' })
    })

    it('in batch mode, reports a doubtful match as "low-confidence" and applies nothing', async () => {
        google.query.mockResolvedValue({ metadata: { title: 'Dune Messiah', author: 'Frank Herbert' } })
        openLibrary.query.mockResolvedValue(null)
        const res = await tidyupBook(dune, { requireConfidentMatch: true })
        expect(res).toMatchObject({ found: false, outcome: 'low-confidence', metadata: {} })
    })

    it('in single-book mode, returns the doubtful match flagged for review', async () => {
        google.query.mockResolvedValue({ metadata: { title: 'Dune Messiah' } })
        openLibrary.query.mockResolvedValue(null)
        const res = await tidyupBook(dune)
        expect(res).toMatchObject({ found: true, confident: false, source: 'googlebooks' })
    })

    it('says "unreachable" only when every provider failed', async () => {
        google.query.mockRejectedValue(new Error('HTTP 429'))
        openLibrary.query.mockRejectedValue(new Error('offline'))
        expect((await tidyupBook(dune)).outcome).toBe('unreachable')

        openLibrary.query.mockResolvedValue(null)
        expect((await tidyupBook(dune)).outcome).toBe('no-match')
    })

    it('looks an ISBN up first, and falls back to the title when it finds nothing', async () => {
        google.query.mockImplementation((params: { isbn?: string }) =>
            Promise.resolve(params.isbn ? null : { metadata: { title: 'Dune' } }))
        const res = await tidyupBook({ ...dune, isbn: '978-0-441-01359-3' })
        expect(google.query.mock.calls).toEqual([
            [{ isbn: '978-0-441-01359-3' }],
            [{ title: 'Dune', author: 'Frank Herbert', lang: 'en' }],
        ])
        expect(res).toMatchObject({ found: true, source: 'googlebooks' })
    })

    it('uses the ISBN result as is when there is one, without detecting the language', async () => {
        google.query.mockResolvedValue({ metadata: { title: 'Dune' } })
        await tidyupBook({ ...dune, isbn: '9780441013593' })
        expect(google.query.mock.calls).toEqual([[{ isbn: '9780441013593' }]])
        expect(mockInvoke.callsTo('detect_language')).toHaveLength(0)
    })

    it('skips the ISBN lookup for something that isn’t an ISBN', async () => {
        google.query.mockResolvedValue({ metadata: { title: 'Dune' } })
        await tidyupBook({ ...dune, isbn: 'n/a' })
        expect(google.query.mock.calls).toEqual([[{ title: 'Dune', author: 'Frank Herbert', lang: 'en' }]])
    })

    it('detects the language once, even when several providers search by title', async () => {
        google.query.mockResolvedValue(null)
        openLibrary.query.mockResolvedValue(null)
        await tidyupBook(dune)
        expect(mockInvoke.callsTo('detect_language')).toHaveLength(1)
    })

    it('searches without a language when it can’t be detected', async () => {
        mockInvoke.on('detect_language', () => null)
        google.query.mockResolvedValue({ metadata: { title: 'Dune' } })
        await tidyupBook(dune)
        expect(google.query.mock.calls[0]).toEqual([{ title: 'Dune', author: 'Frank Herbert', lang: undefined }])
    })

    it('counts a provider whose title fallback failed as unreachable, not as "no match"', async () => {
        google.query.mockImplementation((params: { isbn?: string }) =>
            params.isbn ? Promise.resolve(null) : Promise.reject(new Error('HTTP 429')))
        openLibrary.query.mockRejectedValue(new Error('timeout'))
        const res = await tidyupBook({ ...dune, isbn: '9780441013593' })
        expect(res.outcome).toBe('unreachable')
    })

    it('lists the providers that will be queried', () => {
        expect(availableTidyupSources()).toEqual(['googlebooks', 'openlibrary'])
        google.isAvailable.mockReturnValue(false)
        expect(availableTidyupSources()).toEqual(['openlibrary'])
    })
})
