import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia, type Pinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { backendError, convertFileSrc, mockInvoke } from '@/test/tauri'
import ReaderModal from './ReaderModal.vue'
import { i18nPlugin } from '@/i18n'
import { useLibraryStore } from '@/stores/library'
import { useModalsStore } from '@/stores/modals'
import type { Book, BookFormat, ReaderSource } from '@/types'

function book(format: BookFormat): Book {
    return {
        id: 'b1', path: `/lib/Book.${format}`, filename: `Book.${format}`, format, sizeBytes: 10, dateAdded: '2026-01-01T00:00:00Z',
        metadata: { title: '', author: 'A', categories: [] },
    }
}

describe('ReaderModal', () => {
    let wrapper: VueWrapper | null = null
    let pinia: Pinia

    beforeEach(async () => {
        pinia = createPinia()
        setActivePinia(pinia)
        mockInvoke.reset()
        mockInvoke.on('scan_library', () => [])
        await useLibraryStore().openLibrary('/lib')
    })

    afterEach(() => {
        wrapper?.unmount()
        wrapper = null
        document.body.innerHTML = ''
    })

    /** `attach`: in the document, so key events can bubble up to `document`. */
    function open(b: Book, attach = false) {
        wrapper = mount(ReaderModal, {
            props: { book: b },
            global: { plugins: [pinia, i18nPlugin], stubs: { 'vue-feather': true, teleport: true } },
            attachTo: attach ? document.body : undefined,
        })
        return wrapper
    }

    it('shows a PDF from its asset URL: no bytes over IPC', async () => {
        const source: ReaderSource = { format: 'pdf', path: '/lib/Book.pdf', sizeBytes: 10, converted: false }
        mockInvoke.on('open_for_reading', () => source)
        const w = open(book('pdf'))
        await flushPromises()
        expect(w.find('iframe.reader-pdf').attributes('src')).toBe(convertFileSrc('/lib/Book.pdf'))
        expect(mockInvoke.callsTo('open_for_reading')).toEqual([
            { cmd: 'open_for_reading', args: { libraryPath: '/lib', bookId: 'b1' } },
        ])
        expect(w.find('.reader-title').text()).toBe('Unknown title')
    })

    it('says a Kindle book is being converted while it loads', async () => {
        mockInvoke.on('open_for_reading', () => new Promise(() => {}))
        const w = open(book('mobi'))
        await flushPromises()
        expect(w.find('.reader-state').text()).toBe('Converting to EPUB…')
    })

    it('shows the backend’s message when the book can’t be opened', async () => {
        mockInvoke.on('open_for_reading', () => backendError({ code: 'CONVERSION', message: 'Book.azw3 is DRM-protected, so Librarium can’t convert it.' }))
        const w = open(book('azw3'))
        await flushPromises()
        const state = w.find('.reader-state')
        expect(state.attributes('role')).toBe('alert')
        expect(state.text()).toContain('This book can’t be opened')
        expect(state.text()).toContain('Book.azw3 is DRM-protected, so Librarium can’t convert it.')
    })

    it('is a full-screen dialog named after the book, on top of the modal stack', async () => {
        mockInvoke.on('open_for_reading', () => new Promise(() => {}))
        const w = open({ ...book('epub'), metadata: { title: 'Dune', author: 'Frank Herbert', categories: [] } })
        await flushPromises()
        const dialog = w.find('dialog')
        expect(dialog.classes()).toContain('-screen')
        expect(w.find(`#${dialog.attributes('aria-labelledby')}`).text()).toBe('Dune')
        expect(useModalsStore().count).toBe(1)
        expect(w.find('.reader-close').attributes('aria-label')).toBe('Close the reader')
    })

    it('keeps its keys: arrows and Escape never reach the views behind', async () => {
        mockInvoke.on('open_for_reading', () => new Promise(() => {}))
        const behind = vi.fn()
        document.addEventListener('keydown', behind)
        const w = open(book('epub'), true)
        await flushPromises()
        await w.find('dialog').trigger('keydown', { key: 'ArrowRight' })
        await w.find('dialog').trigger('keydown', { key: 'ArrowLeft' })
        expect(behind).not.toHaveBeenCalled()
        await w.find('dialog').trigger('keydown', { key: 'Escape' })
        expect(w.emitted('close')).toHaveLength(1)
        expect(behind).not.toHaveBeenCalled()
        document.removeEventListener('keydown', behind)
    })
})
