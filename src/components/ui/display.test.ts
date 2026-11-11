import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import Badge from './Badge.vue'
import EmptyState from './EmptyState.vue'
import ProgressBar from './ProgressBar.vue'
import SettingRow from './SettingRow.vue'
import Spinner from './Spinner.vue'
import { cleanup, mountUi } from '@/test/mountUi'

afterEach(cleanup)

describe('Badge', () => {
    it('shows its text with the tone as a class', () => {
        const w = mountUi(Badge, { props: { tone: 'epub' }, slots: { default: 'epub' } })
        expect(w.text()).toBe('epub')
        expect(w.classes()).toEqual(expect.arrayContaining(['ui-badge', '-epub']))
    })

    it('defaults to neutral and can carry an icon', () => {
        const w = mountUi(Badge, { props: { icon: 'check' }, slots: { default: 'Up to date' } })
        expect(w.classes()).toContain('-neutral')
        expect(w.find('vue-feather-stub').attributes('type')).toBe('check')
    })
})

describe('EmptyState', () => {
    it('shows an icon, a heading, a description and actions', () => {
        const w = mountUi(EmptyState, {
            props: { icon: 'book-open', title: 'Your library is empty', description: 'Import books to start.' },
            slots: { actions: '<button>Import</button>' },
        })
        expect(w.get('h2').text()).toBe('Your library is empty')
        expect(w.get('p').text()).toBe('Import books to start.')
        expect(w.get('.ui-empty__actions button').text()).toBe('Import')
        expect(w.attributes('role')).toBeUndefined()
    })

    it('is announced at once for failures, and shows a spinner while loading', () => {
        const failed = mountUi(EmptyState, { props: { title: 'Library not reachable', alert: true } })
        expect(failed.attributes('role')).toBe('alert')
        const loading = mountUi(EmptyState, { props: { loading: true, description: 'Loading…' } })
        expect(loading.findComponent(Spinner).exists()).toBe(true)
    })

    it('uses the heading level it is given', () => {
        const w = mountUi(EmptyState, { props: { title: 'No results', headingTag: 'h3' } })
        expect(w.find('h3').exists()).toBe(true)
    })
})

describe('Spinner', () => {
    it('is decorative without a label', () => {
        const w = mountUi(Spinner)
        expect(w.attributes('role')).toBeUndefined()
        expect(w.find('vue-feather-stub').attributes('aria-hidden')).toBe('true')
    })

    it('announces its label politely', () => {
        const w = mountUi(Spinner, { props: { label: 'Loading the library' } })
        expect(w.attributes('role')).toBe('status')
        expect(w.text()).toBe('Loading the library')
    })
})

describe('ProgressBar', () => {
    it('is a progressbar with its value and label', () => {
        const w = mountUi(ProgressBar, { props: { label: 'Sending', value: 30, max: 120, showValue: true } })
        expect(w.attributes('role')).toBe('progressbar')
        expect(w.attributes('aria-label')).toBe('Sending')
        expect(w.attributes('aria-valuemin')).toBe('0')
        expect(w.attributes('aria-valuemax')).toBe('120')
        expect(w.attributes('aria-valuenow')).toBe('30')
        expect((w.get('.ui-progress__fill').element as HTMLElement).style.width).toBe('25%')
        expect(w.get('.ui-progress__value').text()).toBe('25%')
    })

    it('clamps out-of-range values', () => {
        const over = mountUi(ProgressBar, { props: { label: 'x', value: 150 } })
        expect(over.attributes('aria-valuenow')).toBe('100')
        const under = mountUi(ProgressBar, { props: { label: 'x', value: -5 } })
        expect(under.attributes('aria-valuenow')).toBe('0')
    })

    it('is indeterminate without a value (no number announced)', () => {
        const w = mountUi(ProgressBar, { props: { label: 'Converting', value: null } })
        expect(w.attributes('aria-valuenow')).toBeUndefined()
        expect(w.classes()).toContain('-indeterminate')
    })

    it('can announce a text instead of a number', () => {
        const w = mountUi(ProgressBar, { props: { label: 'Import', value: 12, max: 40, valueText: '12 of 40 books' } })
        expect(w.attributes('aria-valuetext')).toBe('12 of 40 books')
    })
})

describe('SettingRow', () => {
    it('labels a native control by id', () => {
        const w = mountUi(SettingRow, {
            props: { label: 'Launch sound', description: 'Plays when Librarium opens', controlId: 'sound' },
            slots: { default: '<input id="sound" type="checkbox" />' },
        })
        expect(w.get('label').attributes('for')).toBe('sound')
        expect(w.get('label').text()).toBe('Launch sound')
        expect(w.get('.ui-setting__description').text()).toBe('Plays when Librarium opens')
    })

    it('gives the slot ids for aria-labelledby / aria-describedby', () => {
        const w = mountUi(SettingRow, {
            props: { label: 'Theme', description: 'Colours of the app' },
            slots: {
                default: `<template #default="{ labelId, descriptionId }">
                    <div role="radiogroup" :aria-labelledby="labelId" :aria-describedby="descriptionId"></div>
                </template>`,
            },
        })
        const group = w.get('[role="radiogroup"]').element
        expect(document.getElementById(group.getAttribute('aria-labelledby')!)?.textContent).toBe('Theme')
        expect(document.getElementById(group.getAttribute('aria-describedby')!)?.textContent?.trim()).toBe('Colours of the app')
    })
})
