import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { backendError, mockInvoke } from '@/test/tauri'
import EpubConvertModal from './EpubConvertModal.vue'
import { useLibraryStore } from '@/stores/library'
import { useNotificationsStore } from '@/stores/notifications'
import { cleanup, keydown, mountUi, testPinia } from '@/test/mountUi'
import type { Book } from '@/types'

function book(id: string): Book {
    return {
        id, path: `/lib/${id}.mobi`, filename: `${id}.mobi`, format: 'mobi', sizeBytes: 1, dateAdded: '2026-01-01T00:00:00Z',
        metadata: { title: id, author: 'A', categories: [] },
    }
}

const BOOKS = [book('dune'), book('emma')]
const asEpub = (b: Book): Book => ({ ...b, path: b.path.replace('.mobi', '.epub'), filename: b.filename.replace('.mobi', '.epub'), format: 'epub' })

const top = (): HTMLDialogElement => {
    const all = document.querySelectorAll('dialog')
    return all[all.length - 1]
}
const buttonNamed = (text: string) => Array.from(top().querySelectorAll('button')).find(b => b.textContent?.trim() === text)

describe('EpubConvertModal', () => {
    let failing: Set<string>
    let pending: Promise<void> | null

    beforeEach(async () => {
        failing = new Set()
        pending = null
        mockInvoke.reset()
        mockInvoke.on('scan_library', () => BOOKS.map(b => ({ ...b })))
        mockInvoke.on('invalidate_opds_cache', () => null)
        mockInvoke.on('convert_to_epub', async ({ bookId }) => {
            if (pending) { await pending }
            const b = BOOKS.find(x => x.id === bookId)!
            return failing.has(b.id) ? backendError({ code: 'CONVERSION', message: `${b.id} is DRM-protected` }) : asEpub(b)
        })
        await useLibraryStore(testPinia()).openLibrary('/lib')
    })

    afterEach(cleanup)

    it('is a labelled dialog listing the books, with focus inside', () => {
        mountUi(EpubConvertModal, { props: { books: BOOKS } })
        expect(document.getElementById(top().getAttribute('aria-labelledby')!)?.textContent).toContain('Convert to EPUB?')
        expect(top().querySelectorAll('.book-item')).toHaveLength(2)
        expect(top().contains(document.activeElement)).toBe(true)
    })

    it('reports the converted books at once when all succeeded, with a toast', async () => {
        const w = mountUi(EpubConvertModal, { props: { books: BOOKS } })
        buttonNamed('Convert')!.click()
        await flushPromises()
        expect(w.emitted('converted')).toEqual([[BOOKS.map(asEpub)]])
        expect(useNotificationsStore().items.map(n => n.message)).toEqual(['Converted 2 books to EPUB.'])
        expect(useLibraryStore().books.map(b => b.format)).toEqual(['epub', 'epub'])
    })

    it('can’t be closed while converting', async () => {
        let release!: () => void
        pending = new Promise(res => { release = res })
        const w = mountUi(EpubConvertModal, { props: { books: BOOKS } })
        buttonNamed('Convert')!.click()
        await flushPromises()
        keydown(top(), 'Escape')
        expect(w.emitted('close')).toBeUndefined()
        expect(top().querySelector('.ui-modal__close')).toBeNull()
        release()
        await flushPromises()
    })

    it('says so plainly when nothing could be converted, and reports nothing', async () => {
        failing.add('dune')
        failing.add('emma')
        const w = mountUi(EpubConvertModal, { props: { books: BOOKS } })
        buttonNamed('Convert')!.click()
        await flushPromises()
        expect(top().querySelector('[role="alert"]')!.textContent).toContain('None of the 2 books could be converted')
        buttonNamed('Close')!.click()
        expect(w.emitted('converted')).toBeUndefined()
        expect(w.emitted('close')).toHaveLength(1)
        expect(useNotificationsStore().items).toEqual([])
    })

    it('stays open on failures, shows them, retries only those, and reports the converted ones on close', async () => {
        failing.add('emma')
        const w = mountUi(EpubConvertModal, { props: { books: BOOKS } })
        buttonNamed('Convert')!.click()
        await flushPromises()

        expect(w.emitted('converted')).toBeUndefined()
        expect(top().querySelector('[role="alert"]')!.textContent).toContain('1 of 2 books could not be converted')
        const emma = Array.from(top().querySelectorAll('.book-item')).find(li => li.textContent?.includes('emma'))!
        expect(emma.textContent).toContain('Failed: emma is DRM-protected')
        expect(emma.querySelector('.sr-only')!.textContent).toBe('Failed')

        // Retry: only the failed book goes again.
        mockInvoke.calls.length = 0
        buttonNamed('Try again')!.click()
        await flushPromises()
        expect(mockInvoke.callsTo('convert_to_epub').map(c => c.args.bookId)).toEqual(['emma'])

        keydown(top(), 'Escape')
        expect(w.emitted('converted')).toEqual([[[asEpub(BOOKS[0])]]])
        expect(w.emitted('close')).toHaveLength(1)
    })
})
