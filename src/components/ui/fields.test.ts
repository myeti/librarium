import { afterEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import SearchInput from './SearchInput.vue'
import TextField from './TextField.vue'
import SelectField from './SelectField.vue'
import Checkbox from './Checkbox.vue'
import { cleanup, keydown, mountUi } from '@/test/mountUi'

afterEach(() => {
    cleanup()
    vi.useRealTimers()
})

/** The element an input's `aria-describedby` points at, joined. */
function description(input: Element): string {
    return (input.getAttribute('aria-describedby') ?? '').split(' ').filter(Boolean)
        .map(id => document.getElementById(id)?.textContent?.trim()).join(' | ')
}

describe('SearchInput', () => {
    it('is labelled for screen readers', () => {
        const w = mountUi(SearchInput, { props: { label: 'Search books' } })
        const input = w.get('input').element
        const label = document.querySelector(`label[for="${input.id}"]`)
        expect(label?.textContent).toBe('Search books')
        expect(input.getAttribute('placeholder')).toBe('Search books')
    })

    it('updates v-model after a pause in typing, not on every key', async () => {
        vi.useFakeTimers()
        const w = mountUi(SearchInput, { props: { label: 'Search', debounce: 200 } })
        await w.get('input').setValue('dun')
        expect(w.emitted('update:modelValue')).toBeUndefined()
        vi.advanceTimersByTime(199)
        expect(w.emitted('update:modelValue')).toBeUndefined()
        vi.advanceTimersByTime(1)
        expect(w.emitted('update:modelValue')).toEqual([['dun']])
    })

    it('Enter commits at once and emits search', async () => {
        vi.useFakeTimers()
        const w = mountUi(SearchInput, { props: { label: 'Search' } })
        await w.get('input').setValue('dune')
        keydown(w.get('input').element, 'Enter')
        expect(w.emitted('update:modelValue')).toEqual([['dune']])
        expect(w.emitted('search')).toEqual([['dune']])
        vi.runAllTimers()
        expect(w.emitted('update:modelValue')).toHaveLength(1)
    })

    it('the × button clears and refocuses the field', async () => {
        const w = mountUi(SearchInput, { props: { label: 'Search', modelValue: 'dune' } })
        const clear = w.get('button')
        expect(clear.attributes('aria-label')).toBe('Clear')
        await clear.trigger('click')
        expect(w.emitted('update:modelValue')).toEqual([['']])
        expect(document.activeElement).toBe(w.get('input').element)
        await nextTick()
        expect(w.find('button').exists()).toBe(false)
    })

    it('Escape clears a filled field but leaves an empty one alone (for the dialog/panel)', async () => {
        const w = mountUi(SearchInput, { props: { label: 'Search', modelValue: 'dune' } })
        const filled = keydown(w.get('input').element, 'Escape')
        expect(filled.defaultPrevented).toBe(true)
        expect(w.emitted('update:modelValue')).toEqual([['']])
        await w.setProps({ modelValue: '' })
        const empty = keydown(w.get('input').element, 'Escape')
        expect(empty.defaultPrevented).toBe(false)
    })

    it('shows a value set from outside', async () => {
        const w = mountUi(SearchInput, { props: { label: 'Search', modelValue: 'a' } })
        await w.setProps({ modelValue: 'tolkien' })
        expect((w.get('input').element as HTMLInputElement).value).toBe('tolkien')
    })

    it('emits the pending text when unmounted mid-typing', async () => {
        vi.useFakeTimers()
        const w = mountUi(SearchInput, { props: { label: 'Search' } })
        await w.get('input').setValue('half')
        w.unmount()
        expect(w.emitted('update:modelValue')).toEqual([['half']])
    })
})

describe('TextField', () => {
    it('links its label, hint and error to the input', () => {
        const w = mountUi(TextField, { props: { label: 'Title', hint: 'As on the cover', error: 'Required' } })
        const input = w.get('input').element
        expect(document.querySelector(`label[for="${input.id}"]`)?.textContent).toContain('Title')
        expect(description(input)).toBe('As on the cover | Required')
        expect(input.getAttribute('aria-invalid')).toBe('true')
        expect(w.get('[role="alert"]').text()).toContain('Required')
    })

    it('has no aria-describedby or aria-invalid when there is nothing to say', () => {
        const w = mountUi(TextField, { props: { label: 'Title' } })
        expect(w.get('input').attributes('aria-describedby')).toBeUndefined()
        expect(w.get('input').attributes('aria-invalid')).toBeUndefined()
    })

    it('supports v-model', async () => {
        const w = mountUi(TextField, { props: { label: 'Title', modelValue: 'Dune' } })
        expect((w.get('input').element as HTMLInputElement).value).toBe('Dune')
        await w.get('input').setValue('Dune Messiah')
        expect(w.emitted('update:modelValue')).toEqual([['Dune Messiah']])
    })

    it('renders a textarea when multiline', () => {
        const w = mountUi(TextField, { props: { label: 'Description', multiline: true, rows: 6 } })
        expect(w.get('textarea').attributes('rows')).toBe('6')
    })

    it('uses the given id, and sends other attributes to the input (class stays on the frame)', () => {
        const w = mountUi(TextField, { props: { label: 'ISBN', id: 'isbn' }, attrs: { maxlength: '17', class: 'wide' } })
        expect(w.get('input').attributes('id')).toBe('isbn')
        expect(w.get('input').attributes('maxlength')).toBe('17')
        expect(w.get('input').classes()).not.toContain('wide')
        expect(w.classes()).toContain('wide')
    })

    it('marks required fields for assistive tech and visually', () => {
        const w = mountUi(TextField, { props: { label: 'Title', required: true } })
        expect(w.get('input').attributes('required')).toBeDefined()
        expect(w.get('label').text()).toContain('*')
    })
})

describe('SelectField', () => {
    const options = [
        { value: 1, label: 'One' },
        { value: 2, label: 'Two' },
        { value: 3, label: 'Three', disabled: true },
    ]

    it('is a labelled native select', () => {
        const w = mountUi(SelectField, { props: { label: 'Rating', options, modelValue: 2 } })
        const select = w.get('select').element as HTMLSelectElement
        expect(document.querySelector(`label[for="${select.id}"]`)?.textContent).toContain('Rating')
        expect(select.options[select.selectedIndex].text).toBe('Two')
        expect(select.options[2].disabled).toBe(true)
    })

    it('emits the option value with its own type', async () => {
        const w = mountUi(SelectField, { props: { label: 'Rating', options, modelValue: 2 } })
        await w.get('select').setValue('0')
        expect(w.emitted('update:modelValue')).toEqual([[1]])
    })

    it('shows the placeholder while no option matches', () => {
        const w = mountUi(SelectField, { props: { label: 'Rating', options, modelValue: null, placeholder: 'Choose…' } })
        const select = w.get('select').element as HTMLSelectElement
        expect(select.options[select.selectedIndex].text).toBe('Choose…')
        expect(select.options[0].disabled).toBe(true)
    })

    it('links hint and error', () => {
        const w = mountUi(SelectField, { props: { label: 'Rating', options, hint: 'Stars', error: 'Pick one' } })
        expect(description(w.get('select').element)).toBe('Stars | Pick one')
        expect(w.get('select').attributes('aria-invalid')).toBe('true')
    })
})

describe('Checkbox', () => {
    it('is a real checkbox inside its label', async () => {
        const w = mountUi(Checkbox, { props: { label: 'Show covers', modelValue: false } })
        const input = w.get('input[type="checkbox"]')
        expect(input.element.closest('label')?.textContent).toContain('Show covers')
        await input.setValue(true)
        expect(w.emitted('update:modelValue')).toEqual([[true]])
    })

    it('reflects v-model', async () => {
        const w = mountUi(Checkbox, { props: { label: 'A', modelValue: true } })
        expect((w.get('input').element as HTMLInputElement).checked).toBe(true)
        await w.setProps({ modelValue: false })
        expect((w.get('input').element as HTMLInputElement).checked).toBe(false)
    })

    it('can be indeterminate', async () => {
        const w = mountUi(Checkbox, { props: { label: 'All', indeterminate: true } })
        expect((w.get('input').element as HTMLInputElement).indeterminate).toBe(true)
        await w.setProps({ indeterminate: false })
        expect((w.get('input').element as HTMLInputElement).indeterminate).toBe(false)
    })

    it('reads its hint with it', () => {
        const w = mountUi(Checkbox, { props: { label: 'Sound', hint: 'Plays at launch' } })
        expect(description(w.get('input').element)).toBe('Plays at launch')
    })

    it('takes rich label content from the slot', () => {
        const w = mountUi(Checkbox, { slots: { default: '<strong>Bold</strong> label' } })
        expect(w.get('label').html()).toContain('<strong>Bold</strong>')
    })
})
