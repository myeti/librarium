import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
// The window's drag-and-drop listener, driven by the test.
const dragDrop = vi.hoisted(() => ({
    handler: null as ((e: { payload: unknown }) => void) | null,
    unlisten: vi.fn(),
}))
vi.mock('@tauri-apps/api/window', () => ({
    getCurrentWindow: () => ({
        onDragDropEvent: (handler: (e: { payload: unknown }) => void) => {
            dragDrop.handler = handler
            return Promise.resolve(dragDrop.unlisten)
        },
    }),
}))
import { mockInvoke } from '@/test/tauri'
import { i18nPlugin } from '@/i18n'
import { useLibraryStore } from '@/stores/library'
import FileDropTarget from './FileDropTarget.vue'
import type { Book } from '@/types'

let wrapper: VueWrapper | null = null

function drop(payload: unknown) {
    dragDrop.handler!({ payload })
    return flushPromises()
}

beforeEach(async () => {
    const pinia = createPinia()
    setActivePinia(pinia)
    mockInvoke.reset()
    dragDrop.handler = null
    dragDrop.unlisten.mockReset()
    mockInvoke.on('scan_library', () => [])
    await useLibraryStore().openLibrary('/lib')
    wrapper = mount(FileDropTarget, {
        attachTo: document.body,
        global: {
            plugins: [pinia, i18nPlugin],
            stubs: { 'vue-feather': true, EpubConvertModal: { props: ['books'], template: '<div class="convert-offer">{{ books.length }}</div>' } },
        },
    })
    await flushPromises()
})

afterEach(() => {
    wrapper?.unmount()
    wrapper = null
    document.body.innerHTML = ''
})

describe('FileDropTarget', () => {
    it('shows the drop overlay while files are dragged over the window', async () => {
        expect(wrapper!.find('.dropzone-overlay').exists()).toBe(false)
        await drop({ type: 'enter', paths: ['/a.epub'], position: { x: 0, y: 0 } })
        expect(wrapper!.find('.dropzone-overlay').exists()).toBe(true)
        await drop({ type: 'leave' })
        expect(wrapper!.find('.dropzone-overlay').exists()).toBe(false)
    })

    it('imports dropped ebooks and offers to convert the MOBI ones', async () => {
        const mobi: Book = {
            id: 'm', path: '/lib/m.mobi', filename: 'm.mobi', format: 'mobi', sizeBytes: 1, dateAdded: '2026-01-01',
            metadata: { title: 'M', author: 'A', categories: [] },
        }
        mockInvoke.on('import_books', () => [mobi])
        await drop({ type: 'drop', paths: ['/x/m.mobi', '/x/readme.txt'], position: { x: 0, y: 0 } })
        expect(mockInvoke.callsTo('import_books')[0].args.paths).toEqual(['/x/m.mobi'])
        expect(wrapper!.find('.dropzone-overlay').exists()).toBe(false)
        expect(wrapper!.find('.convert-offer').text()).toBe('1')
    })

    it('stops listening when unmounted', () => {
        wrapper!.unmount()
        wrapper = null
        expect(dragDrop.unlisten).toHaveBeenCalledTimes(1)
    })
})
