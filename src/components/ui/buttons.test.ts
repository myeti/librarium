import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import BaseButton from './BaseButton.vue'
import IconButton from './IconButton.vue'
import { cleanup, mountUi } from '@/test/mountUi'

afterEach(cleanup)

describe('BaseButton', () => {
    it('is a plain button (never submits a form by accident) with its variant and size', () => {
        const w = mountUi(BaseButton, { props: { variant: 'primary', size: 'sm' }, slots: { default: 'Save' } })
        const button = w.get('button')
        expect(button.attributes('type')).toBe('button')
        expect(button.classes()).toEqual(expect.arrayContaining(['ui-button', '-primary', '-sm']))
        expect(button.text()).toBe('Save')
    })

    it('can be a submit button', () => {
        const w = mountUi(BaseButton, { props: { type: 'submit' }, slots: { default: 'Send' } })
        expect(w.get('button').attributes('type')).toBe('submit')
    })

    it('emits click when usable', async () => {
        const w = mountUi(BaseButton, { slots: { default: 'Go' } })
        await w.get('button').trigger('click')
        expect(w.emitted('click')).toHaveLength(1)
    })

    it('while loading: shows a spinner, says busy, ignores clicks but stays focusable', async () => {
        const w = mountUi(BaseButton, { props: { loading: true }, slots: { default: 'Deleting' } })
        const button = w.get('button')
        expect(button.attributes('aria-busy')).toBe('true')
        expect(button.attributes('aria-disabled')).toBe('true')
        expect(button.attributes('disabled')).toBeUndefined()
        expect(w.findComponent({ name: 'Spinner' }).exists()).toBe(true)
        await button.trigger('click')
        expect(w.emitted('click')).toBeUndefined()
        ;(button.element as HTMLButtonElement).focus()
        expect(document.activeElement).toBe(button.element)
    })

    it('when disabled: native disabled attribute and no click', async () => {
        const w = mountUi(BaseButton, { props: { disabled: true }, slots: { default: 'Go' } })
        expect(w.get('button').attributes('disabled')).toBeDefined()
        await w.get('button').trigger('click')
        expect(w.emitted('click')).toBeUndefined()
    })

    it('passes attributes through to the <button>', () => {
        const w = mountUi(BaseButton, { attrs: { 'data-autofocus': '', title: 'Hint' }, slots: { default: 'Go' } })
        expect(w.get('button').attributes('data-autofocus')).toBe('')
        expect(w.get('button').attributes('title')).toBe('Hint')
    })
})

describe('IconButton', () => {
    it('is named by its required label (aria-label and tooltip)', () => {
        const w = mountUi(IconButton, { props: { label: 'Close', icon: 'x' } })
        const button = w.get('button')
        expect(button.attributes('aria-label')).toBe('Close')
        expect(button.attributes('title')).toBe('Close')
        expect(button.attributes('type')).toBe('button')
    })

    it('declares label and icon as required props', () => {
        const props = (IconButton as unknown as { props: Record<string, { required?: boolean }> }).props
        expect(props.label.required).toBe(true)
        expect(props.icon.required).toBe(true)
    })

    it('can have a longer tooltip than its name', () => {
        const w = mountUi(IconButton, { props: { label: 'Import', icon: 'plus', title: 'Import (⌘O)' } })
        expect(w.get('button').attributes('aria-label')).toBe('Import')
        expect(w.get('button').attributes('title')).toBe('Import (⌘O)')
    })

    it('is a toggle only when `pressed` is given', async () => {
        const plain = mountUi(IconButton, { props: { label: 'Refresh', icon: 'refresh-cw' } })
        expect(plain.get('button').attributes('aria-pressed')).toBeUndefined()

        const toggle = mountUi(IconButton, { props: { label: 'Grid view', icon: 'grid', pressed: false } })
        expect(toggle.get('button').attributes('aria-pressed')).toBe('false')
        await toggle.setProps({ pressed: true })
        expect(toggle.get('button').attributes('aria-pressed')).toBe('true')
        expect(toggle.get('button').classes()).toContain('-active')
    })

    it('emits click unless disabled', async () => {
        const w = mountUi(IconButton, { props: { label: 'Close', icon: 'x' } })
        await w.get('button').trigger('click')
        expect(w.emitted('click')).toHaveLength(1)
        await w.setProps({ disabled: true })
        await w.get('button').trigger('click')
        expect(w.emitted('click')).toHaveLength(1)
    })
})
