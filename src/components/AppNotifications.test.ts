import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import AppNotifications from './AppNotifications.vue'
import BaseModal from './ui/BaseModal.vue'
import { useNotificationsStore } from '@/stores/notifications'
import { cleanup, keydown, mountUi, testPinia } from '@/test/mountUi'

/** happy-dom has no Popover API: record the calls instead. */
const proto = HTMLElement.prototype as unknown as Record<'showPopover' | 'hidePopover', unknown>
let shownOn: HTMLElement[]

beforeEach(() => {
    shownOn = []
    proto.showPopover = function (this: HTMLElement) { shownOn.push(this) }
    proto.hidePopover = function () {}
})

afterEach(() => {
    cleanup()
    Reflect.deleteProperty(proto, 'showPopover')
    Reflect.deleteProperty(proto, 'hidePopover')
})

const region = () => document.querySelector<HTMLElement>('.toasts')!
const toasts = () => Array.from(document.querySelectorAll<HTMLElement>('.toast'))

async function setup() {
    mountUi(AppNotifications)
    await flushPromises()
    return useNotificationsStore(testPinia())
}

describe('AppNotifications', () => {
    it('reads errors assertively and everything else politely', async () => {
        const store = await setup()
        store.push('success', 'Saved')
        store.push('error', 'Disk full')
        await flushPromises()
        // (Test utils stub TransitionGroup: the lists are found by class, not as <ol>.)
        const [polite, assertive] = Array.from(region().querySelectorAll('.toasts__list'))
        expect(polite.getAttribute('aria-live')).toBe('polite')
        expect(polite.textContent).toContain('Saved')
        expect(polite.textContent).not.toContain('Disk full')
        expect(assertive.getAttribute('aria-live')).toBe('assertive')
        expect(assertive.textContent).toContain('Disk full')
        // The kind is said in words, not only by the icon's colour.
        expect(assertive.textContent).toContain('Error:')
    })

    it('has a labelled close button, and runs an action button’s callback', async () => {
        const store = await setup()
        const retry = vi.fn()
        store.push('error', 'Import failed', { action: { label: 'Try again', run: retry } })
        store.push('info', 'Note')
        await flushPromises()

        const failed = toasts().find(t => t.textContent?.includes('Import failed'))!
        const action = Array.from(failed.querySelectorAll('button')).find(b => b.textContent?.includes('Try again'))!
        action.click()
        await flushPromises()
        expect(retry).toHaveBeenCalledTimes(1)
        expect(store.items.map(n => n.message)).toEqual(['Note'])

        const close = toasts()[0].querySelector<HTMLButtonElement>('button[aria-label="Dismiss"]')!
        close.click()
        await flushPromises()
        expect(store.items).toEqual([])
    })

    it('is shown in the top layer (popover) when mounted', async () => {
        await setup()
        expect(region().getAttribute('popover')).toBe('manual')
        expect(shownOn).toContain(region())
    })

    it('moves into the top dialog, above it and not inert, and back to <body> when it closes', async () => {
        const store = await setup()
        store.push('error', 'Could not save')
        await flushPromises()
        expect(region().parentElement).toBe(document.body)

        const modal = mountUi(BaseModal, { props: { title: 'Edit' }, slots: { default: '<input />' } })
        await flushPromises()
        const dialog = document.querySelector('dialog')!
        expect(dialog.contains(region())).toBe(true)
        // Shown again after the move: it joins the top layer after the dialog.
        expect(shownOn.filter(el => el === region()).length).toBeGreaterThanOrEqual(2)

        // A dialog over the dialog: the toasts follow the top one.
        const confirm = mountUi(BaseModal, { props: { title: 'Confirm' }, slots: { default: '<button>OK</button>' } })
        await flushPromises()
        const dialogs = document.querySelectorAll('dialog')
        expect(dialogs[1].contains(region())).toBe(true)

        // The lower one closing first changes nothing; the last one closing sends them home.
        modal.unmount()
        await flushPromises()
        expect(dialogs[1].contains(region())).toBe(true)
        confirm.unmount()
        await flushPromises()
        expect(region().parentElement).toBe(document.body)
        expect(store.items.map(n => n.message)).toEqual(['Could not save'])
        expect(shownOn[shownOn.length - 1]).toBe(region())
    })

    it('Escape on a toast dismisses it, and doesn’t close the dialog behind', async () => {
        const store = await setup()
        const modal = mountUi(BaseModal, { props: { title: 'Edit' }, slots: { default: '<input />' } })
        store.push('error', 'Could not save')
        await flushPromises()
        const close = toasts()[0].querySelector('button')!
        keydown(close, 'Escape')
        await flushPromises()
        expect(store.items).toEqual([])
        expect(modal.emitted('close')).toBeUndefined()
    })

    it('stops the timers while the pointer or the focus is on the toasts', async () => {
        const store = await setup()
        store.push('success', 'Saved')
        await flushPromises()
        region().dispatchEvent(new MouseEvent('mouseenter'))
        await flushPromises()
        expect(store.paused).toBe(true)
        region().dispatchEvent(new MouseEvent('mouseleave'))
        await flushPromises()
        expect(store.paused).toBe(false)

        toasts()[0].querySelector('button')!.dispatchEvent(new FocusEvent('focusin', { bubbles: true }))
        await flushPromises()
        expect(store.paused).toBe(true)
        toasts()[0].querySelector('button')!.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: document.body }))
        await flushPromises()
        expect(store.paused).toBe(false)
    })
})
