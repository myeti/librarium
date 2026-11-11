import { describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { batchMessages, emptyCounts, tidyupBatch, type BatchTidyupDeps } from './batch'
import type { TidyupResult } from './types'
import type { Book, BookMetadata } from '@/types'

function book(id: string, metadata: Partial<BookMetadata> = {}): Book {
    return {
        id, path: `/lib/${id}.epub`, filename: `${id}.epub`, format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01',
        metadata: { title: `Title ${id}`, author: 'Author', categories: [], ...metadata },
    }
}

function found(metadata: Partial<BookMetadata>, coverUrl: string | null = null): TidyupResult {
    return { found: true, outcome: 'found', source: 'googlebooks', metadata, coverUrl, similarity: 1, confident: true }
}

const NOT_FOUND: TidyupResult = { found: false, outcome: 'no-match', source: null, metadata: {}, coverUrl: null, similarity: 0, confident: false }
const UNREACHABLE: TidyupResult = { ...NOT_FOUND, outcome: 'unreachable' }

/** A library of `books` with fakes for every dependency; `results` by book id. */
function setup(books: Book[], results: Record<string, TidyupResult>, covers: Record<string, boolean> = {}) {
    const library = new Map(books.map(b => [b.id, b]))
    const deps = {
        lookup: vi.fn((q: { title: string }) => Promise.resolve(results[books.find(b => b.metadata.title === q.title)!.id])),
        current: (id: string) => library.get(id),
        hasCover: vi.fn((id: string) => Promise.resolve(covers[id] ?? false)),
        saveMetadata: vi.fn((id: string, patch: Partial<BookMetadata>) => {
            const b = library.get(id)!
            library.set(id, { ...b, metadata: { ...b.metadata, ...patch } })
            return Promise.resolve()
        }),
        saveCover: vi.fn(() => Promise.resolve()),
    } satisfies BatchTidyupDeps
    return { deps, library }
}

describe('tidyupBatch', () => {
    it('only fills empty fields and asks for confident matches', async () => {
        const b = book('a', { isbn: '111', description: '' })
        const { deps } = setup([b], { a: found({ title: 'Other title', isbn: '999', description: 'New', language: 'fr' }) })
        const counts = await tidyupBatch([b], deps)
        expect(deps.lookup).toHaveBeenCalledWith(expect.anything(), { requireConfidentMatch: true })
        expect(deps.saveMetadata).toHaveBeenCalledWith('a', { description: 'New', language: 'fr' })
        expect(counts).toMatchObject({ updated: 1, unchanged: 0 })
    })

    it('adds a cover only to a book without one', async () => {
        const withCover = book('a')
        const without = book('b')
        const { deps } = setup([withCover, without], {
            a: found({}, 'https://covers/a.jpg'),
            b: found({}, 'https://covers/b.jpg'),
        }, { a: true })
        const counts = await tidyupBatch([withCover, without], deps)
        expect(deps.saveCover).toHaveBeenCalledTimes(1)
        expect(deps.saveCover).toHaveBeenCalledWith('b', 'https://covers/b.jpg')
        expect(counts).toMatchObject({ updated: 1, unchanged: 1 })
    })

    it('assumes a cover exists when that cannot be told (never replaces a real one)', async () => {
        const b = book('a')
        const { deps } = setup([b], { a: found({}, 'https://covers/a.jpg') })
        deps.hasCover.mockRejectedValueOnce(new Error('unreadable'))
        const counts = await tidyupBatch([b], deps)
        expect(deps.saveCover).not.toHaveBeenCalled()
        expect(counts.unchanged).toBe(1)
    })

    it('counts no match, unreachable services, failed saves and failed covers apart', async () => {
        const books = ['a', 'b', 'c', 'd'].map(id => book(id))
        const { deps } = setup(books, {
            a: NOT_FOUND,
            b: UNREACHABLE,
            c: found({ description: 'x' }),
            d: found({ description: 'y' }, 'https://covers/d.jpg'),
        })
        deps.saveMetadata.mockImplementation((id: string) => id === 'c' ? Promise.reject(new Error('locked')) : Promise.resolve())
        deps.saveCover.mockRejectedValueOnce(new Error('404'))
        const counts = await tidyupBatch(books, deps)
        // d: metadata saved (updated), cover failed.
        expect(counts).toEqual({ ...emptyCounts(), noMatch: 1, unreachable: 1, saveFailed: 1, updated: 1, coverFailed: 1 })
    })

    it('reads each book again before patching it (an earlier save may have changed it)', async () => {
        const b = book('a')
        const { deps, library } = setup([b], { a: found({ description: 'From the web' }) })
        library.set('a', { ...b, metadata: { ...b.metadata, description: 'Typed meanwhile' } })
        const counts = await tidyupBatch([b], deps)
        expect(deps.saveMetadata).not.toHaveBeenCalled()
        expect(counts.unchanged).toBe(1)
    })

    it('keeps the counts so far when a lookup throws', async () => {
        const books = [book('a'), book('b')]
        const { deps } = setup(books, { a: found({ description: 'x' }) })
        deps.lookup.mockResolvedValueOnce(found({ description: 'x' })).mockRejectedValueOnce(new Error('boom'))
        const counts = emptyCounts()
        await expect(tidyupBatch(books, deps, counts)).rejects.toThrow('boom')
        expect(counts.updated).toBe(1)
    })
})

describe('batchMessages', () => {
    it('sums the batch up in one toast, with a separate warning for covers', () => {
        const messages = batchMessages({ ...emptyCounts(), updated: 2, noMatch: 1, coverFailed: 1 }, 3)
        expect(messages.map(m => m.type)).toEqual(['success', 'warning'])
        expect(messages[0].message).toBe('Metadata for 3 books: 2 updated, 1 without a confident match.')
    })

    it('warns when saves failed, informs when nothing changed, errors when nothing was reachable', () => {
        expect(batchMessages({ ...emptyCounts(), updated: 1, saveFailed: 1 }, 2)[0].type).toBe('warning')
        expect(batchMessages({ ...emptyCounts(), unchanged: 2 }, 2)[0].type).toBe('info')
        const unreachable = batchMessages({ ...emptyCounts(), unreachable: 2 }, 2)
        expect(unreachable).toHaveLength(1)
        expect(unreachable[0].type).toBe('error')
    })
})
