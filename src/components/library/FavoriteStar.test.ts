import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { cleanup, mountUi } from '@/test/mountUi'
import { useBookActionsStore } from '@/stores/bookActions'
import type { Book } from '@/types'
import FavoriteStar from './FavoriteStar.vue'

function book(favorite?: boolean): Book {
    return {
        id: 'a', path: '/lib/a.epub', filename: 'a.epub', format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01',
        metadata: { title: 'Dune', author: 'Frank Herbert', categories: [] }, ...(favorite ? { favorite } : {}),
    }
}

afterEach(cleanup)

describe('FavoriteStar', () => {
    it('says what a click does, and is filled on a favorite', () => {
        const off = mountUi(FavoriteStar, { props: { book: book() } })
        expect(off.attributes('aria-label')).toBe('Add “Dune” to favorites')
        expect(off.classes()).not.toContain('-on')
        const on = mountUi(FavoriteStar, { props: { book: book(true) } })
        expect(on.attributes('aria-label')).toBe('Remove “Dune” from favorites')
        expect(on.classes()).toContain('-on')
        // One Tab stop per collection: the star isn't one.
        expect(on.attributes('tabindex')).toBe('-1')
    })

    it('toggles the book, and the click stops there (it would open the book)', async () => {
        const wrapper = mountUi(FavoriteStar, { props: { book: book() } })
        const toggle = vi.spyOn(useBookActionsStore(), 'toggleFavorite').mockResolvedValue()
        const parentClick = vi.fn()
        const star = wrapper.element as HTMLElement
        star.parentElement!.addEventListener('click', parentClick)
        await wrapper.trigger('click')
        expect(toggle).toHaveBeenCalledWith([book()])
        expect(parentClick).not.toHaveBeenCalled()
    })
})
