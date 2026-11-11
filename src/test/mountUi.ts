/**
 * Test helper for UI components: mounts with a fresh Pinia, the i18n plugin
 * and vue-feather stubbed, attached to the document (focus, Teleport and
 * `document.activeElement` need a real tree).
 *
 * Test files that use it must mock Tauri first (i18n saves the language
 * through the backend):
 *
 *     vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
 *
 * Call `cleanup()` in afterEach: it unmounts everything mounted through
 * here and empties <body>.
 */
import { mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia, type Pinia } from 'pinia'
import type { Component, DefineComponent } from 'vue'
import { i18nPlugin } from '@/i18n'

/** What the kit tests pass to mount (a subset of VTU's mounting options). */
export interface UiMountOptions {
    props?: Record<string, unknown>
    attrs?: Record<string, unknown>
    /** Slot content: HTML strings (compiled by VTU) or render functions. */
    slots?: Record<string, unknown>
    /** Globals to provide, e.g. `{ $route, $router }` for a view that reads the route. */
    mocks?: Record<string, unknown>
}

const mounted: VueWrapper[] = []
let pinia: Pinia | null = null

/** The Pinia of the current test (created on first use, reset by cleanup()). */
export function testPinia(): Pinia {
    if (!pinia) {
        pinia = createPinia()
        setActivePinia(pinia)
    }
    return pinia
}

export function mountUi(component: Component, options: UiMountOptions = {}): VueWrapper {
    const wrapper: VueWrapper = mount(component as DefineComponent, {
        attachTo: document.body,
        props: options.props,
        attrs: options.attrs,
        slots: options.slots as Record<string, string>,
        global: {
            plugins: [testPinia(), i18nPlugin],
            stubs: { 'vue-feather': true },
            mocks: options.mocks,
        },
    })
    mounted.push(wrapper)
    return wrapper
}

export function cleanup() {
    while (mounted.length) {
        const w = mounted.pop()!
        try { w.unmount() } catch { /* already unmounted by the test */ }
    }
    pinia = null
    document.body.innerHTML = ''
}

/** Dispatches a keydown on `target` (bubbling, cancelable) and returns the event. */
export function keydown(target: EventTarget, key: string, init: KeyboardEventInit = {}): KeyboardEvent {
    const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init })
    target.dispatchEvent(event)
    return event
}
