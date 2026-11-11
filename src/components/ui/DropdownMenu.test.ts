/* eslint-disable vue/one-component-per-file -- small host components for the tests */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, nextTick } from 'vue'
import { flushPromises } from '@vue/test-utils'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import DropdownMenu from './DropdownMenu.vue'
import MenuItem from './MenuItem.vue'
import MenuSeparator from './MenuSeparator.vue'
import BaseModal from './BaseModal.vue'
import { computeMenuPosition, VIEWPORT_MARGIN } from './menuPosition'
import { cleanup, keydown, mountUi } from '@/test/mountUi'

afterEach(cleanup)

const onReveal = vi.fn()
const onDelete = vi.fn()

interface TriggerSlot { toggle: () => void, attrs: Record<string, unknown> }

/** The trigger slot of the tests: a plain button wired as documented. */
const trigger = ({ toggle, attrs }: TriggerSlot) => h('button', { id: 'trigger', ...attrs, onClick: toggle }, '⋮')

/** A "more actions" menu as a view would write it (render function: no template compiler in tests). */
const Host = defineComponent({
    render() {
        return h('div', [
            h('button', { id: 'outside' }, 'Outside'),
            h(DropdownMenu, { label: 'More actions' }, {
                trigger,
                default: () => [
                    h(MenuItem, { icon: 'folder', onClick: onReveal }, () => 'Reveal'),
                    h(MenuItem, { icon: 'edit', disabled: true }, () => 'Rename'),
                    h(MenuItem, { icon: 'copy' }, () => 'Duplicate'),
                    h(MenuSeparator),
                    h(MenuItem, { icon: 'trash-2', danger: true, onClick: onDelete }, () => 'Delete'),
                ],
            }),
        ])
    },
})

async function openMenu() {
    const w = mountUi(Host)
    const trigger = document.getElementById('trigger')!
    trigger.focus()
    trigger.click()
    await flushPromises()
    await nextTick()
    return { w, trigger, menu: () => document.querySelector<HTMLElement>('[role="menu"]') }
}

const items = () => Array.from(document.querySelectorAll<HTMLElement>('[role="menuitem"]'))
const labelOf = (el: Element | null) => el?.textContent?.trim()

describe('DropdownMenu', () => {
    it('opens from its trigger, announces it, and focuses the first item', async () => {
        const { trigger, menu } = await openMenu()
        expect(menu()).not.toBeNull()
        expect(menu()!.getAttribute('aria-label')).toBe('More actions')
        expect(trigger.getAttribute('aria-haspopup')).toBe('menu')
        expect(trigger.getAttribute('aria-expanded')).toBe('true')
        expect(trigger.getAttribute('aria-controls')).toBe(menu()!.id)
        expect(labelOf(document.activeElement)).toBe('Reveal')
    })

    it('moves with the arrows (wrapping), skips disabled items, jumps with Home/End', async () => {
        const { menu } = await openMenu()
        const m = menu()!
        keydown(m, 'ArrowDown')
        expect(labelOf(document.activeElement)).toBe('Duplicate')   // Rename is disabled
        keydown(m, 'ArrowDown')
        expect(labelOf(document.activeElement)).toBe('Delete')
        keydown(m, 'ArrowDown')
        expect(labelOf(document.activeElement)).toBe('Reveal')      // wrapped
        keydown(m, 'ArrowUp')
        expect(labelOf(document.activeElement)).toBe('Delete')
        keydown(m, 'Home')
        expect(labelOf(document.activeElement)).toBe('Reveal')
        keydown(m, 'End')
        expect(labelOf(document.activeElement)).toBe('Delete')
    })

    it('jumps to the next item starting with a typed letter', async () => {
        vi.useFakeTimers()
        try {
            const { menu } = await openMenu()
            keydown(menu()!, 'd')
            expect(labelOf(document.activeElement)).toBe('Duplicate')
            vi.advanceTimersByTime(600)                 // the typed prefix resets
            keydown(menu()!, 'd')
            expect(labelOf(document.activeElement)).toBe('Delete')
            vi.advanceTimersByTime(600)
            keydown(menu()!, 'd')
            expect(labelOf(document.activeElement)).toBe('Duplicate')  // wrapped
            keydown(menu()!, 'e')                       // typed quickly: the prefix is "de"
            expect(labelOf(document.activeElement)).toBe('Delete')
        } finally {
            vi.useRealTimers()
        }
    })

    it('runs the item and closes, giving focus back to the trigger', async () => {
        const { trigger, menu } = await openMenu()
        items()[0].click()
        await nextTick()
        expect(onReveal).toHaveBeenCalledOnce()
        expect(menu()).toBeNull()
        expect(document.activeElement).toBe(trigger)
        expect(trigger.getAttribute('aria-expanded')).toBe('false')
    })

    it('ignores a disabled item and stays open', async () => {
        const { menu } = await openMenu()
        const rename = items().find(i => labelOf(i) === 'Rename')!
        expect(rename.getAttribute('aria-disabled')).toBe('true')
        rename.click()
        await nextTick()
        expect(menu()).not.toBeNull()
    })

    it('closes on Escape without letting it reach a dialog or the view', async () => {
        const { trigger, menu } = await openMenu()
        const spy = vi.fn()
        document.addEventListener('keydown', spy)
        const e = keydown(menu()!, 'Escape')
        document.removeEventListener('keydown', spy)
        await nextTick()
        expect(e.defaultPrevented).toBe(true)
        expect(spy).not.toHaveBeenCalled()
        expect(menu()).toBeNull()
        expect(document.activeElement).toBe(trigger)
    })

    it('closes on Tab, back on the trigger', async () => {
        const { trigger, menu } = await openMenu()
        keydown(menu()!, 'Tab')
        await nextTick()
        expect(menu()).toBeNull()
        expect(document.activeElement).toBe(trigger)
    })

    it('closes on a click outside, without moving focus there', async () => {
        const { menu } = await openMenu()
        const outside = document.getElementById('outside')!
        outside.dispatchEvent(new Event('pointerdown', { bubbles: true }))
        await nextTick()
        expect(menu()).toBeNull()
    })

    it('a click inside or on the trigger is not "outside"', async () => {
        const { trigger, menu } = await openMenu()
        menu()!.dispatchEvent(new Event('pointerdown', { bubbles: true }))
        trigger.dispatchEvent(new Event('pointerdown', { bubbles: true }))
        await nextTick()
        expect(menu()).not.toBeNull()
    })

    it('opens from the keyboard on the trigger (ArrowDown: first item, ArrowUp: last)', async () => {
        mountUi(Host)
        const trigger = document.getElementById('trigger')!
        keydown(trigger, 'ArrowUp')
        await flushPromises()
        await nextTick()
        expect(labelOf(document.activeElement)).toBe('Delete')
    })

    it('opens as a context menu at the pointer', async () => {
        const w = mountUi(Host)
        const menuVm = w.findComponent(DropdownMenu).vm as unknown as { openFromEvent(e: MouseEvent): void }
        const event = new MouseEvent('contextmenu', { clientX: 120, clientY: 80, cancelable: true })
        Object.defineProperty(event, 'target', { value: document.getElementById('outside') })
        menuVm.openFromEvent(event)
        await flushPromises()
        await nextTick()
        expect(event.defaultPrevented).toBe(true)
        const menu = document.querySelector<HTMLElement>('[role="menu"]')!
        expect(menu).not.toBeNull()
        // happy-dom has no layout (the menu measures 0×0): the point itself.
        expect(menu.style.left).toBe('120px')
        expect(menu.style.top).toBe('80px')
        expect(labelOf(document.activeElement)).toBe('Reveal')
    })

    it('renders inside the open dialog that contains its trigger', async () => {
        const InDialog = defineComponent({
            render() {
                return h(BaseModal, { title: 'Book' }, () => h(DropdownMenu, { label: 'Actions' }, {
                    trigger,
                    default: () => h(MenuItem, null, () => 'One'),
                }))
            },
        })
        mountUi(InDialog)
        document.getElementById('trigger')!.click()
        await flushPromises()
        await nextTick()
        const menu = document.querySelector('[role="menu"]')!
        expect(menu.closest('dialog')).not.toBeNull()
    })

    it('MenuItem can be a checkbox item', () => {
        const w = mountUi(MenuItem, { props: { checked: true }, slots: { default: 'Title' } })
        expect(w.get('button').attributes('role')).toBe('menuitemcheckbox')
        expect(w.get('button').attributes('aria-checked')).toBe('true')
        expect(w.get('button').attributes('tabindex')).toBe('-1')
    })
})

describe('computeMenuPosition', () => {
    const viewport = { width: 1000, height: 800 }
    const menu = { width: 200, height: 300 }

    it('opens below the trigger, aligned to its start (or end)', () => {
        const anchor = { left: 100, top: 50, width: 40, height: 30 }
        expect(computeMenuPosition(anchor, menu, viewport, 'bottom-start', 4)).toEqual({ left: 100, top: 84 })
        expect(computeMenuPosition({ ...anchor, left: 500 }, menu, viewport, 'bottom-end', 4)).toEqual({ left: 340, top: 84 })
        // Aligned to the end it would start off-screen: clamped to the margin.
        expect(computeMenuPosition(anchor, menu, viewport, 'bottom-end', 4)).toEqual({ left: VIEWPORT_MARGIN, top: 84 })
    })

    it('flips above when there is no room below', () => {
        const anchor = { left: 100, top: 700, width: 40, height: 30 }
        expect(computeMenuPosition(anchor, menu, viewport, 'bottom-start', 4).top).toBe(700 - 4 - 300)
    })

    it('stays inside the window horizontally', () => {
        const anchor = { left: 950, top: 50, width: 40, height: 30 }
        expect(computeMenuPosition(anchor, menu, viewport).left).toBe(1000 - 200 - VIEWPORT_MARGIN)
    })

    it('at a point: flips left and up near the bottom-right corner', () => {
        const point = { left: 900, top: 700, width: 0, height: 0 }
        expect(computeMenuPosition(point, menu, viewport)).toEqual({ left: 700, top: 400 })
    })

    it('never goes above or left of the margin, even for a huge menu', () => {
        const point = { left: 10, top: 10, width: 0, height: 0 }
        expect(computeMenuPosition(point, { width: 2000, height: 2000 }, viewport)).toEqual({ left: VIEWPORT_MARGIN, top: VIEWPORT_MARGIN })
    })
})
