import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@/utils/visibility', () => ({ watchVisibility: () => () => {} }))
import BatchOrderList from './BatchOrderList.vue'
import { moved } from './collection'
import { cleanup, mountUi } from '@/test/mountUi'
import type { Book } from '@/types'

afterEach(cleanup)

const book = (id: string): Book => ({
    id, path: `/lib/${id}.epub`, filename: `${id}.epub`, format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01',
    metadata: { title: `Book ${id}`, author: 'Author', categories: [] },
})

describe('moved', () => {
    it('moves one item, keeping the others in order', () => {
        expect(moved(['a', 'b', 'c', 'd'], 0, 2)).toEqual(['b', 'c', 'a', 'd'])
        expect(moved(['a', 'b', 'c', 'd'], 3, 0)).toEqual(['d', 'a', 'b', 'c'])
        expect(moved(['a', 'b'], 0, 9)).toEqual(['b', 'a'])
    })
})

describe('BatchOrderList', () => {
    it('is dragged by a row\'s handle: the book goes where the pointer is', async () => {
        const books = ['a', 'b', 'c'].map(book)
        const w = mountUi(BatchOrderList, { props: { modelValue: books } })
        // Rows of 40 px, one under the other (happy-dom has no layout).
        const rows = Array.from(document.querySelectorAll<HTMLElement>('.batch-order__item'))
        rows.forEach((row, i) => {
            row.getBoundingClientRect = () => ({ top: i * 40, height: 40, bottom: i * 40 + 40, left: 0, right: 200, width: 200, x: 0, y: i * 40, toJSON: () => ({}) })
        })
        const handle = rows[0].querySelector('.batch-order__handle')!
        handle.dispatchEvent(new PointerEvent('pointerdown', { button: 0, bubbles: true }))
        await w.vm.$nextTick()
        expect(rows[0].classList.contains('-dragging')).toBe(true)
        window.dispatchEvent(new PointerEvent('pointermove', { clientY: 105 }))
        window.dispatchEvent(new PointerEvent('pointerup'))
        await w.vm.$nextTick()
        const emitted = w.emitted('update:modelValue') as Book[][][]
        expect(emitted[emitted.length - 1][0].map(b => b.id)).toEqual(['b', 'c', 'a'])
        expect(document.querySelector('.batch-order__item.-dragging')).toBeNull()
    })

    it('shows places, or with numbering the volumes from the first one', async () => {
        const w = mountUi(BatchOrderList, { props: { modelValue: ['a', 'b'].map(book) } })
        const numbers = () => Array.from(document.querySelectorAll('.batch-order__number')).map(n => n.textContent?.trim())
        expect(numbers()).toEqual(['1', '2'])
        await w.setProps({ numbering: true, start: 4.5 })
        expect(numbers()).toEqual(['Vol. 4.5', 'Vol. 5.5'])
    })

    it('names each handle with the book and its place', () => {
        mountUi(BatchOrderList, { props: { modelValue: ['a', 'b'].map(book) } })
        const labels = Array.from(document.querySelectorAll('.batch-order__handle')).map(h => h.getAttribute('aria-label'))
        expect(labels).toEqual(['Move “Book a”, number 1 of 2', 'Move “Book b”, number 2 of 2'])
    })
})
