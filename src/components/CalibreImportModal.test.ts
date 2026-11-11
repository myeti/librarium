import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia, type Pinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
type Handler = (event: { payload: unknown }) => void
const listeners = new Map<string, Handler>()
vi.mock('@tauri-apps/api/event', () => ({
    listen: vi.fn((name: string, handler: Handler) => {
        listeners.set(name, handler)
        return Promise.resolve(() => { listeners.delete(name) })
    }),
}))
vi.mock('@/services/librarium-api', () => ({ reportError: vi.fn() }))
import { backendError, mockInvoke } from '@/test/tauri'
import CalibreImportModal from './CalibreImportModal.vue'
import { i18nPlugin } from '@/i18n'
import { useLibraryStore } from '@/stores/library'
import { useNotificationsStore } from '@/stores/notifications'
import type { Book, CalibreBook, CalibreImportResult } from '@/types'

const CALIBRE_BOOKS: CalibreBook[] = [
    { id: 1, title: 'Dune', author: 'Frank Herbert', series: 'Dune', seriesIndex: 0, availableFormats: ['epub'], bestFormat: 'epub', hasCover: true },
    { id: 2, title: '', author: '', series: null, seriesIndex: null, availableFormats: ['txt'], bestFormat: null, hasCover: false },
]

function book(id: string): Book {
    return {
        id, path: `/lib/${id}.epub`, filename: `${id}.epub`, format: 'epub', sizeBytes: 1, dateAdded: '2026-01-01T00:00:00Z',
        metadata: { title: id, author: 'A', categories: [] },
    }
}

function deferred<T>() {
    let resolve!: (v: T) => void
    let reject!: (e: unknown) => void
    const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej })
    return { promise, resolve, reject }
}

describe('CalibreImportModal', () => {
    let wrapper: VueWrapper | null = null
    let pinia: Pinia
    let running: ReturnType<typeof deferred<CalibreImportResult>>

    beforeEach(async () => {
        pinia = createPinia()
        setActivePinia(pinia)
        mockInvoke.reset()
        listeners.clear()
        mockInvoke.on('scan_library', () => [])
        mockInvoke.on('ensure_cover', () => null)
        mockInvoke.on('detect_calibre_library', () => '/calibre')
        mockInvoke.on('list_calibre_books', () => CALIBRE_BOOKS)
        running = deferred<CalibreImportResult>()
        mockInvoke.on('import_from_calibre', () => running.promise)
        mockInvoke.on('cancel_calibre_import', () => true)
        await useLibraryStore().openLibrary('/lib')
    })

    afterEach(() => {
        wrapper?.unmount()
        wrapper = null
    })

    /** Mounted, with the detected Calibre library listed. */
    async function browsing() {
        wrapper = mount(CalibreImportModal, { global: { plugins: [pinia, i18nPlugin], stubs: { 'vue-feather': true, teleport: true } } })
        await flushPromises()
        await wrapper.find('.auto-detect-row .use-detected').trigger('click')
        await flushPromises()
        return wrapper
    }

    async function startImport(w: VueWrapper) {
        await w.find('.start-import').trigger('click')
        await flushPromises()
    }

    it('is a labelled dialog that starts on the detected library', async () => {
        wrapper = mount(CalibreImportModal, { global: { plugins: [pinia, i18nPlugin], stubs: { 'vue-feather': true, teleport: true } } })
        await flushPromises()
        const dialog = wrapper.find('dialog')
        expect(wrapper.find(`#${dialog.attributes('aria-labelledby')}`).text()).toBe('Import from Calibre')
        expect(wrapper.find('.auto-detect-path').text()).toBe('/calibre')
    })

    it('lists Calibre books with placeholders for unknown values, and a series position of 0', async () => {
        const w = await browsing()
        expect(w.findAll('.book-title').map(t => t.text())).toEqual(['Dune', 'Unknown title'])
        expect(w.findAll('.book-author').map(t => t.text())).toEqual(['Frank Herbert', 'Unknown author'])
        expect(w.find('.book-series').text()).toBe('Dune #0')
    })

    it('imports without sending the library, shows progress and reports a cancelled import', async () => {
        const w = await browsing()
        await startImport(w)
        expect(mockInvoke.callsTo('import_from_calibre')[0].args).toEqual({ calibrePath: '/calibre', bookIds: [1, 2], libraryPath: '/lib' })

        listeners.get('calibre-import-progress')!({ payload: { done: 1, total: 2, currentTitle: 'Dune' } })
        await flushPromises()
        expect(w.find('.progress-count').text()).toBe('1 of 2 books')
        expect(w.find('.progress-title').text()).toBe('Dune')
        const bar = w.find('[role="progressbar"]')
        expect(bar.attributes('aria-valuenow')).toBe('1')
        expect(bar.attributes('aria-valuemax')).toBe('2')
        expect(bar.attributes('aria-valuetext')).toBe('1 of 2 books')

        await w.find('.cancel-import').trigger('click')
        await flushPromises()
        expect(mockInvoke.callsTo('cancel_calibre_import')).toHaveLength(1)
        expect(w.find('.cancel-import').attributes('disabled')).toBeDefined()
        expect(w.find('.cancel-import').text()).toBe('Cancelling after the current book…')

        running.resolve({
            imported: [book('dune')],
            skipped: [
                { title: '', author: '', reason: 'noFile', existingFilename: null },
                { title: 'Emma', author: 'Jane Austen', reason: 'alreadyInLibrary', existingFilename: 'Emma.epub' },
            ],
            failed: [],
            cancelled: true,
            error: null,
            remaining: 1,
        })
        await flushPromises()

        expect(w.find('.notice.-warn').text()).toBe('Import cancelled: 1 book was not imported.')
        expect(w.findAll('.skipped-book').map(li => li.text())).toEqual([
            'Unknown title — Unknown author (no file in Calibre)',
            'Emma — Jane Austen (already in your library as Emma.epub)',
        ])
        expect(w.emitted('imported')).toEqual([[[book('dune')]]])
        expect(useLibraryStore().books.map(b => b.id)).toEqual(['dune'])
    })

    it('reports an import that stopped early, with what it had done', async () => {
        const w = await browsing()
        await startImport(w)
        running.resolve({ imported: [], skipped: [], failed: [{ title: 'Dune', reason: 'copied, not indexed' }], cancelled: false, error: 'Disk full', remaining: 4 })
        await flushPromises()
        expect(w.find('.notice.-bad').text()).toBe('The import stopped early: Disk full 4 books were not imported.')
        expect(w.find('.failed-book').text()).toBe('Dune — copied, not indexed')
    })

    it('closing during the import (×) cancels it, and closes once it has stopped', async () => {
        const w = await browsing()
        await startImport(w)
        await w.find('.ui-modal__close').trigger('click')
        await flushPromises()
        expect(mockInvoke.callsTo('cancel_calibre_import')).toHaveLength(1)
        expect(w.emitted('close')).toBeUndefined()

        running.resolve({ imported: [book('a')], skipped: [], failed: [], cancelled: true, error: null, remaining: 1 })
        await flushPromises()
        expect(w.emitted('imported')).toEqual([[[book('a')]]])
        expect(w.emitted('close')).toHaveLength(1)
    })

    it('Escape during the import does the same: cancel first, close after', async () => {
        const w = await browsing()
        await startImport(w)
        await w.find('dialog').trigger('keydown', { key: 'Escape' })
        await flushPromises()
        expect(mockInvoke.callsTo('cancel_calibre_import')).toHaveLength(1)
        expect(w.emitted('close')).toBeUndefined()
        running.resolve({ imported: [], skipped: [], failed: [], cancelled: true, error: null, remaining: 2 })
        await flushPromises()
        expect(w.emitted('close')).toHaveLength(1)
    })

    it('says so in the dialog when another import is running, and stays open on the list', async () => {
        const w = await browsing()
        await startImport(w)
        await w.find('.ui-modal__close').trigger('click')   // asked to close meanwhile
        running.reject({ code: 'CONFLICT', message: 'A Calibre import is already running' })
        await flushPromises()
        expect(w.find('[role="alert"]').text()).toBe('A Calibre import is already running.')
        expect(w.find('.browse-toolbar').exists()).toBe(true)
        expect(w.emitted('close')).toBeUndefined()
        expect(useNotificationsStore().items).toEqual([])
        expect(mockInvoke.callsTo('scan_library')).toHaveLength(1)
    })

    it('shows a read error from the backend in the dialog and stays on the first step', async () => {
        mockInvoke.on('list_calibre_books', () => backendError({ code: 'INVALID_INPUT', message: 'metadata.db in this folder is not a Calibre database' }))
        const w = await browsing()
        expect(w.find('.auto-detect-row').exists()).toBe(true)
        expect(w.find('[role="alert"]').text()).toContain('is not a Calibre database')
    })

    it('labels each book’s checkbox with the book, and says when a book has no file it can import', async () => {
        const w = await browsing()
        const rows = w.findAll('.book-row')
        expect(rows[0].find('label').text()).toContain('Dune')
        expect(rows[0].find('input[type="checkbox"]').element.closest('label')).not.toBeNull()
        expect(rows[1].text()).toContain('No supported file')
        await rows[1].find('input').setValue(false)
        expect(w.find('.start-import').text()).toBe('Import 1 book')
    })
})
