import { describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { coverUrl } from './covers'

describe('coverUrl', () => {
    it('is the asset URL of the cover file, versioned by its mtime', () => {
        expect(coverUrl({ coverPath: '/lib/.covers/thumbs/a b.jpg', coverVersion: 1730000000123 }))
            .toBe(`asset://localhost/${encodeURIComponent('/lib/.covers/thumbs/a b.jpg')}?v=1730000000123`)
    })

    it('changes when the cover is replaced at the same path', () => {
        const before = coverUrl({ coverPath: '/lib/.covers/thumbs/a.jpg', coverVersion: 1 })
        const after = coverUrl({ coverPath: '/lib/.covers/thumbs/a.jpg', coverVersion: 2 })
        expect(after).not.toBe(before)
    })

    it('is null without a cover, and never a data URL', () => {
        expect(coverUrl({})).toBeNull()
        expect(coverUrl({ coverPath: '' })).toBeNull()
        expect(coverUrl({ coverPath: '/x.jpg' })).toMatch(/^asset:.*\?v=0$/)
    })
})
