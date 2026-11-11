import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, type VueWrapper } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import FilterSortBar from './FilterSortBar.vue'
import { useLibraryStore } from '@/stores/library'
import { i18nPlugin } from '@/i18n'

describe('FilterSortBar search', () => {
    let wrapper: VueWrapper | null = null

    beforeEach(() => { vi.useFakeTimers() })

    afterEach(() => {
        wrapper?.unmount()
        wrapper = null
        vi.useRealTimers()
    })

    function mountBar() {
        const pinia = createPinia()
        setActivePinia(pinia)
        wrapper = mount(FilterSortBar, { global: { plugins: [pinia, i18nPlugin], stubs: { 'vue-feather': true } } })
        const input = wrapper.find('input[type="search"]')
        return { store: useLibraryStore(), input }
    }

    it('applies the search once typing pauses (150 ms), not on every key', async () => {
        const { store, input } = mountBar()
        const setFilters = vi.spyOn(store, 'setFilters')
        await input.setValue('d')
        await input.setValue('du')
        await input.setValue('dune')
        expect((input.element as HTMLInputElement).value).toBe('dune')
        expect(store.filters.search).toBe('')

        vi.advanceTimersByTime(149)
        expect(setFilters).not.toHaveBeenCalled()
        vi.advanceTimersByTime(1)
        expect(setFilters).toHaveBeenCalledTimes(1)
        expect(store.filters.search).toBe('dune')
    })

    it('applies at once on Enter', async () => {
        const { store, input } = mountBar()
        await input.setValue('emma')
        await input.trigger('keydown', { key: 'Enter' })
        expect(store.filters.search).toBe('emma')
    })

    it('follows the store when the search is reset elsewhere (another library opened)', async () => {
        const { store, input } = mountBar()
        await input.setValue('dune')
        vi.advanceTimersByTime(150)
        await input.setValue('dun') // pending…
        store.setFilters({ search: '' }) // …when the library resets the search
        await wrapper!.vm.$nextTick()
        expect((input.element as HTMLInputElement).value).toBe('')
        vi.runAllTimers()
        expect(store.filters.search).toBe('')
    })
})
