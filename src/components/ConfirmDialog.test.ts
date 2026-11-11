import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import ConfirmDialog from './ConfirmDialog.vue'
import { i18nPlugin } from '@/i18n'
import { useModalsStore } from '@/stores/modals'

describe('ConfirmDialog', () => {
    let wrapper: VueWrapper | null = null
    let opener: HTMLButtonElement

    beforeEach(() => {
        // The element that had focus before the dialog opened.
        opener = document.createElement('button')
        opener.textContent = 'Delete'
        document.body.appendChild(opener)
        opener.focus()
    })

    afterEach(() => {
        wrapper?.unmount()
        wrapper = null
        document.body.innerHTML = ''
    })

    function open(props: Record<string, unknown> = {}) {
        const pinia = createPinia()
        setActivePinia(pinia)
        wrapper = mount(ConfirmDialog, {
            props: { title: 'Move 3 books to the Trash?', message: 'They go to the Trash.', ...props },
            global: { plugins: [pinia, i18nPlugin], stubs: { 'vue-feather': true } },
            attachTo: document.body,
        })
        const dialog = document.body.querySelector('[role="dialog"]') as HTMLElement
        const [cancel, confirm] = Array.from(dialog.querySelectorAll('button'))
        return { dialog, cancel, confirm }
    }

    function key(target: Element, k: string, init: KeyboardEventInit = {}) {
        target.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init }))
    }

    it('is an accessible modal dialog labelled by its title', () => {
        const { dialog } = open()
        expect(dialog.getAttribute('aria-modal')).toBe('true')
        const title = document.getElementById(dialog.getAttribute('aria-labelledby')!)
        expect(title?.textContent).toContain('Move 3 books to the Trash?')
        expect(document.getElementById(dialog.getAttribute('aria-describedby')!)?.textContent).toContain('They go to the Trash.')
    })

    it('focuses Cancel first for destructive actions', () => {
        const { cancel } = open({ danger: true })
        expect(document.activeElement).toBe(cancel)
    })

    it('focuses the confirm button for other actions', () => {
        const { confirm } = open()
        expect(document.activeElement).toBe(confirm)
    })

    it('cancels on Escape', () => {
        const { dialog } = open({ danger: true })
        key(dialog, 'Escape')
        expect(wrapper!.emitted('cancel')).toHaveLength(1)
        expect(wrapper!.emitted('confirm')).toBeUndefined()
    })

    it('confirms on Enter when no button has focus', () => {
        const { dialog } = open({ danger: true })
        key(dialog, 'Enter')
        expect(wrapper!.emitted('confirm')).toHaveLength(1)
    })

    it('lets Enter on the focused Cancel button cancel instead of confirming', () => {
        const { cancel } = open({ danger: true })
        key(cancel, 'Enter')
        expect(wrapper!.emitted('confirm')).toBeUndefined()
    })

    it('does not let key presses reach document-level shortcuts', () => {
        const spy = vi.fn()
        document.addEventListener('keydown', spy)
        const { dialog } = open()
        key(dialog, 'Escape')
        document.removeEventListener('keydown', spy)
        expect(spy).not.toHaveBeenCalled()
    })

    it('ignores the buttons and Escape while busy', () => {
        const { dialog, confirm } = open({ busy: true })
        confirm.click()
        key(dialog, 'Escape')
        expect(wrapper!.emitted('confirm')).toBeUndefined()
        expect(wrapper!.emitted('cancel')).toBeUndefined()
    })

    it('counts itself as an open modal and restores focus when closed', () => {
        open()
        expect(useModalsStore().count).toBe(1)
        wrapper!.unmount()
        wrapper = null
        expect(useModalsStore().count).toBe(0)
        expect(document.activeElement).toBe(opener)
    })

    it('shows an error passed by the parent', () => {
        const { dialog } = open({ error: 'Nothing was moved to the Trash: disk full' })
        expect(dialog.querySelector('[role="alert"]')?.textContent).toContain('disk full')
    })
})
