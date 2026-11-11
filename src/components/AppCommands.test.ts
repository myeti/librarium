import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createMemoryHistory, createRouter, type Router } from 'vue-router'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
// The menu bar's events: the test plays the backend and emits them.
const menuListeners: ((event: { payload: string }) => void)[] = []
vi.mock('@tauri-apps/api/event', () => ({
    listen: vi.fn((name: string, handler: (event: { payload: string }) => void) => {
        if (name === 'menu-command') { menuListeners.push(handler) }
        return Promise.resolve(() => {})
    }),
}))
import { backendError, mockInvoke } from '@/test/tauri'
import AppCommands from './AppCommands.vue'
import { useShortcutsStore } from '@/stores/shortcuts'
import { useImportsStore } from '@/stores/imports'
import { useLibraryStore } from '@/stores/library'
import { useModalsStore } from '@/stores/modals'
import { useNotificationsStore } from '@/stores/notifications'
import { useBookActionsStore } from '@/stores/bookActions'
import { i18nPlugin } from '@/i18n'

/** A menu click, as the backend reports it (src-tauri/src/menu.rs). */
function menu(id: string) {
    for (const listener of menuListeners) { listener({ payload: id }) }
}

let wrapper: VueWrapper | null = null
let router: Router

async function mountAt(path: string) {
    const pinia = createPinia()
    setActivePinia(pinia)
    useShortcutsStore().install(window)
    router = createRouter({
        history: createMemoryHistory(),
        routes: [
            { path: '/library', name: 'library', component: { template: '<div />' } },
            { path: '/:other(.*)*', name: 'other', component: { template: '<div />' } },
        ],
    })
    await router.push(path)
    wrapper = mount(AppCommands, {
        attachTo: document.body,
        global: { plugins: [pinia, i18nPlugin, router], stubs: { 'vue-feather': true } },
    })
    await flushPromises()
}

beforeEach(() => {
    mockInvoke.reset()
    menuListeners.length = 0
})

afterEach(() => {
    wrapper?.unmount()
    wrapper = null
    useShortcutsStore().reset()
    document.body.innerHTML = ''
    vi.useRealTimers()
})

describe('menu bar commands', () => {
    it('opens Settings, and Settings › Privacy', async () => {
        await mountAt('/library')
        menu('app.settings')
        await flushPromises()
        expect(router.currentRoute.value.fullPath).toBe('/settings')
        menu('help.privacy')
        await flushPromises()
        expect(router.currentRoute.value.fullPath).toBe('/settings#privacy')
    })

    it('opens the Keyboard shortcuts dialog (same as ⌘/)', async () => {
        await mountAt('/library')
        menu('help.shortcuts')
        await flushPromises()
        expect(document.querySelector('[role="dialog"]')?.textContent).toContain('Keyboard shortcuts')
    })

    it('runs the same import as ⌘O, once even when the menu echoes the key', async () => {
        vi.useFakeTimers()
        await mountAt('/settings')
        const pick = vi.spyOn(useImportsStore(), 'pickAndImport').mockResolvedValue(undefined)
        menu('library.import')
        expect(pick).toHaveBeenCalledTimes(1)
        vi.advanceTimersByTime(1000)
        window.dispatchEvent(new KeyboardEvent('keydown', { key: 'o', metaKey: true, ctrlKey: !useShortcutsStore().isMac, bubbles: true, cancelable: true }))
        menu('library.import')
        expect(pick).toHaveBeenCalledTimes(2)
    })

    it('switches the library to a list from another screen, and shows it', async () => {
        await mountAt('/settings')
        menu('view.list')
        await flushPromises()
        expect(useLibraryStore().viewMode).toBe('list')
        expect(router.currentRoute.value.path).toBe('/library')
    })

    it('opens the library for Search, then searches once the view is there', async () => {
        await mountAt('/discover')
        menu('library.search')
        await flushPromises()
        expect(router.currentRoute.value.path).toBe('/library')
        // The library view registers its handler when it mounts.
        const focusSearch = vi.fn()
        useShortcutsStore().register('library.search', focusSearch)
        expect(focusSearch).toHaveBeenCalledTimes(1)
    })

    it('on the library, leaves Search to the view’s own handler', async () => {
        await mountAt('/library')
        const focusSearch = vi.fn()
        useShortcutsStore().register('library.search', focusSearch)
        menu('library.search')
        expect(focusSearch).toHaveBeenCalledTimes(1)
        expect(router.currentRoute.value.path).toBe('/library')
    })

    it('opens the Duplicates review in the library', async () => {
        await mountAt('/settings')
        menu('library.findDuplicates')
        await flushPromises()
        expect(useBookActionsStore().duplicatesOpen).toBe(true)
        expect(router.currentRoute.value.path).toBe('/library')
    })

    it('shows the log file, and says so when it can’t', async () => {
        await mountAt('/library')
        mockInvoke.on('open_log_folder', () => undefined)
        menu('help.logs')
        await flushPromises()
        expect(mockInvoke.callsTo('open_log_folder')).toHaveLength(1)
        mockInvoke.on('open_log_folder', () => backendError({ code: 'IO', message: 'no file manager' }))
        menu('help.logs')
        await flushPromises()
        const items = useNotificationsStore().items
        expect(items[items.length - 1].message).toBe('Could not show the log file: no file manager')
    })

    it('does nothing behind a dialog, and ignores unknown ids', async () => {
        await mountAt('/library')
        useModalsStore().open('ConfirmDialog')
        menu('app.settings')
        menu('rm -rf /')
        await flushPromises()
        expect(router.currentRoute.value.path).toBe('/library')
    })
})
