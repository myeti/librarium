import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, nextTick } from 'vue'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import BaseModal from './BaseModal.vue'
import { useModalsStore } from '@/stores/modals'
import { cleanup, keydown, mountUi, testPinia } from '@/test/mountUi'

/** All open dialogs, bottom to top (they are all teleported to <body>). */
const dialogs = () => Array.from(document.body.querySelectorAll('dialog'))
const top = () => dialogs()[dialogs().length - 1]

describe('BaseModal', () => {
    let opener: HTMLButtonElement

    beforeEach(() => {
        opener = document.createElement('button')
        opener.textContent = 'Open'
        document.body.appendChild(opener)
        opener.focus()
    })

    afterEach(cleanup)

    function open(props: Record<string, unknown> = {}, slots: Record<string, string> = {}) {
        return mountUi(BaseModal, {
            props: { title: 'Edit book', ...props },
            slots: { default: '<input id="first" /><button id="second">B</button>', ...slots },
        })
    }

    it('opens a native modal <dialog>, labelled by its title', () => {
        open()
        const dialog = top()
        expect(dialog.open).toBe(true)
        expect(dialog.getAttribute('role')).toBe('dialog')
        expect(dialog.getAttribute('aria-modal')).toBe('true')
        const title = document.getElementById(dialog.getAttribute('aria-labelledby')!)
        expect(title?.textContent).toContain('Edit book')
    })

    it('can be labelled by a heading of its own content, or by aria-label', () => {
        open({ title: '', labelledby: 'my-heading', bare: true }, { default: '<h2 id="my-heading">Mine</h2>' })
        expect(top().getAttribute('aria-labelledby')).toBe('my-heading')
        cleanup()
        open({ title: '', ariaLabel: 'Reader' })
        expect(top().getAttribute('aria-label')).toBe('Reader')
        expect(top().hasAttribute('aria-labelledby')).toBe(false)
    })

    it('focuses the first field of its body, or the element marked data-autofocus', () => {
        open()
        expect(document.activeElement?.id).toBe('first')
        cleanup()
        open({}, { default: '<input id="a" /><button id="b" data-autofocus>B</button>' })
        expect(document.activeElement?.id).toBe('b')
    })

    it('focuses the panel when the content has nothing focusable', () => {
        open({}, { default: '<p>Just text</p>' })
        expect((document.activeElement as HTMLElement).classList.contains('ui-modal__panel')).toBe(true)
    })

    it('registers in the modals stack while open', () => {
        const w = open()
        const modals = useModalsStore(testPinia())
        expect(modals.count).toBe(1)
        w.unmount()
        expect(modals.count).toBe(0)
    })

    it('gives focus back to the opener when it closes', () => {
        const w = open()
        expect(document.activeElement).not.toBe(opener)
        w.unmount()
        expect(document.activeElement).toBe(opener)
    })

    it('asks to close on Escape and on the × button', async () => {
        const w = open()
        keydown(top(), 'Escape')
        expect(w.emitted('close')).toEqual([['escape']])
        const close = top().querySelector<HTMLButtonElement>('.ui-modal__close')!
        expect(close.getAttribute('aria-label')).toBe('Close')
        close.click()
        await nextTick()
        expect(w.emitted('close')).toEqual([['escape'], ['button']])
    })

    it('ignores Escape and hides the × while not dismissible', () => {
        const w = open({ dismissible: false })
        keydown(top(), 'Escape')
        expect(w.emitted('close')).toBeUndefined()
        expect(top().querySelector('.ui-modal__close')).toBeNull()
    })

    it('handles the browser cancel event (its own Escape) once', () => {
        const w = open()
        const cancel = new Event('cancel', { cancelable: true })
        top().dispatchEvent(cancel)
        expect(cancel.defaultPrevented).toBe(true)
        expect(w.emitted('close')).toEqual([['escape']])
        // The same key press seen as keydown + cancel is one request.
        keydown(top(), 'Escape')
        top().dispatchEvent(new Event('cancel', { cancelable: true }))
        expect(w.emitted('close')).toHaveLength(2)
    })

    it('keeps key presses inside the dialog', () => {
        const spy = vi.fn()
        document.addEventListener('keydown', spy)
        open()
        keydown(top(), 'Escape')
        keydown(top(), 'a')
        document.removeEventListener('keydown', spy)
        expect(spy).not.toHaveBeenCalled()
    })

    it('passes other keys on as a keydown event', () => {
        const w = open()
        keydown(top(), 'Enter')
        expect(w.emitted('keydown')).toHaveLength(1)
    })

    it('only the top dialog reacts to Escape', () => {
        const below = open({ title: 'Below' })
        const above = open({ title: 'Above' })
        const [first, second] = dialogs()
        keydown(first, 'Escape')
        expect(below.emitted('close')).toBeUndefined()
        keydown(second, 'Escape')
        expect(above.emitted('close')).toHaveLength(1)
        expect(below.emitted('close')).toBeUndefined()
    })

    it('wraps Tab and Shift+Tab inside the panel', () => {
        open()
        const first = document.getElementById('first')!
        const close = top().querySelector<HTMLElement>('.ui-modal__close')!
        // Order in the panel: × (header), #first, #second.
        document.getElementById('second')!.focus()
        const forward = keydown(document.getElementById('second')!, 'Tab')
        expect(forward.defaultPrevented).toBe(true)
        expect(document.activeElement).toBe(close)
        const back = keydown(close, 'Tab', { shiftKey: true })
        expect(back.defaultPrevented).toBe(true)
        expect(document.activeElement?.id).toBe('second')
        // In the middle, Tab is left to the browser.
        first.focus()
        expect(keydown(first, 'Tab').defaultPrevented).toBe(false)
    })

    describe('backdrop', () => {
        function clickBackdrop() {
            const dialog = top()
            dialog.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
            dialog.dispatchEvent(new MouseEvent('click', { bubbles: true }))
        }

        it('does nothing by default', () => {
            const w = open()
            clickBackdrop()
            expect(w.emitted('close')).toBeUndefined()
        })

        it('asks to close with closeOnBackdrop', () => {
            const w = open({ closeOnBackdrop: true })
            clickBackdrop()
            expect(w.emitted('close')).toEqual([['backdrop']])
        })

        it('ignores a press that started inside the panel (text selection)', () => {
            const w = open({ closeOnBackdrop: true })
            document.getElementById('first')!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
            top().dispatchEvent(new MouseEvent('click', { bubbles: true }))
            expect(w.emitted('close')).toBeUndefined()
        })

        it('ignores clicks inside the panel', () => {
            const w = open({ closeOnBackdrop: true })
            const panel = top().querySelector('.ui-modal__panel')!
            panel.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
            panel.dispatchEvent(new MouseEvent('click', { bubbles: true }))
            expect(w.emitted('close')).toBeUndefined()
        })
    })

    describe('unsaved edits (dirty)', () => {
        it('asks "Discard your changes?" instead of closing, on Escape, × and backdrop', async () => {
            const w = open({ dirty: true, closeOnBackdrop: true })
            keydown(top(), 'Escape')
            await nextTick()
            expect(w.emitted('close')).toBeUndefined()
            expect(dialogs()).toHaveLength(2)
            const guard = top()
            expect(guard.getAttribute('role')).toBe('alertdialog')
            expect(document.getElementById(guard.getAttribute('aria-labelledby')!)?.textContent).toBe('Discard your changes?')
            // Safe default: focus on "Keep editing".
            expect(document.activeElement?.textContent).toContain('Keep editing')
        })

        it('"Keep editing" (or Escape) keeps the dialog and the edits', async () => {
            const w = open({ dirty: true })
            keydown(top(), 'Escape')
            await nextTick()
            keydown(top(), 'Escape')
            await nextTick()
            expect(dialogs()).toHaveLength(1)
            expect(w.emitted('close')).toBeUndefined()
        })

        it('"Discard" closes with the original reason', async () => {
            const w = open({ dirty: true })
            top().querySelector<HTMLButtonElement>('.ui-modal__close')!.click()
            await nextTick()
            const discard = Array.from(top().querySelectorAll('button')).find(b => b.textContent?.includes('Discard'))!
            discard.click()
            await nextTick()
            expect(w.emitted('close')).toEqual([['button']])
        })
    })

    it('re-opens if the browser closes it on its own, and asks to close', () => {
        const w = open()
        const dialog = top()
        dialog.close()                        // as Chromium does after repeated Escapes
        expect(dialog.open).toBe(true)
        expect(w.emitted('close')).toEqual([['escape']])
    })

    it('renders header, body and footer slots; bare renders the content alone', () => {
        open({}, { footer: '<button id="save">Save</button>' })
        expect(top().querySelector('.ui-modal__header')).not.toBeNull()
        expect(top().querySelector('.ui-modal__body #first')).not.toBeNull()
        expect(top().querySelector('.ui-modal__footer #save')).not.toBeNull()
        cleanup()
        open({ bare: true, title: '' }, { default: '<p id="content">Hi</p>' })
        expect(top().querySelector('.ui-modal__header')).toBeNull()
        expect(top().querySelector('.ui-modal__panel > #content')).not.toBeNull()
    })

    it('puts attributes (class, data-*) on the panel', () => {
        mountUi(BaseModal, { props: { title: 'T' }, attrs: { class: 'my-dialog', 'data-test': 'x' } })
        const panel = top().querySelector('.ui-modal__panel')!
        expect(panel.classList.contains('my-dialog')).toBe(true)
        expect(panel.getAttribute('data-test')).toBe('x')
    })

    it('is unmounted by its parent through v-if, the usual pattern', async () => {
        const Parent = defineComponent({
            components: { BaseModal },
            data: () => ({ show: true }),
            render() {
                return this.show ? h(BaseModal, { title: 'T', onClose: () => { this.show = false } }) : null
            },
        })
        mountUi(Parent)
        expect(dialogs()).toHaveLength(1)
        keydown(top(), 'Escape')
        await nextTick()
        expect(dialogs()).toHaveLength(0)
        expect(useModalsStore(testPinia()).count).toBe(0)
        expect(document.activeElement).toBe(opener)
    })
})
