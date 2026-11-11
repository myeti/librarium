import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createMemoryHistory, createRouter } from 'vue-router'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
// Two catalogs: one answers (twice the same book, one without an author), one is down.
vi.mock('@/services/store', async () => {
    const { HttpStatusError } = await import('@/services/http')
    return {
        STORE_PROVIDERS: [
            {
                id: 'gutendex', label: 'Project Gutenberg', isAvailable: () => true,
                search: () => Promise.resolve([
                    { id: 'g-1', source: 'gutendex', title: 'Emma', author: 'Jane Austen', downloadUrl: 'https://x/1.pdf', format: 'pdf' },
                    { id: 'g-2', source: 'gutendex', title: 'Emma', author: 'Jane Austen', downloadUrl: 'https://x/2.epub', format: 'epub' },
                    { id: 'g-3', source: 'gutendex', title: 'Lady Susan', author: '', downloadUrl: 'https://x/3.epub', format: 'epub' },
                ]),
            },
            {
                id: 'openlibrary', label: 'Open Library', isAvailable: () => true,
                search: () => Promise.reject(new HttpStatusError(503, 'https://openlibrary.org/search.json')),
            },
        ],
    }
})
import DiscoverView from './DiscoverView.vue'
import { currentLang, i18nPlugin } from '@/i18n'

async function searchFor(text: string) {
    const pinia = createPinia()
    setActivePinia(pinia)
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/:p(.*)*', component: DiscoverView }] })
    await router.push('/discover')
    const wrapper = mount(DiscoverView, {
        attachTo: document.body,
        global: { plugins: [pinia, i18nPlugin, router], stubs: { 'vue-feather': true } },
    })
    const input = wrapper.find('input[type="search"]')
    await input.setValue(text)
    await input.trigger('keydown', { key: 'Enter' })
    await flushPromises()
    return wrapper
}

enableAutoUnmount(afterEach)
beforeEach(() => { vi.spyOn(console, 'error').mockImplementation(() => {}) })
afterEach(() => { currentLang.value = 'en' })

describe('Discover', () => {
    it('merges the catalogs’ answers: one card per book', async () => {
        const wrapper = await searchFor('emma')
        expect(wrapper.findAll('.discover-card__title').map(t => t.text())).toEqual(['Emma', 'Lady Susan'])
        expect(wrapper.find('.discover-count').text()).toBe('2 books')
        // The EPUB copy was kept.
        expect(wrapper.find('.discover-card .ui-badge').text()).toBe('EPUB')
    })

    it('says "Unknown author" in the UI’s language, and why a catalog failed', async () => {
        currentLang.value = 'fr'
        const wrapper = await searchFor('emma')
        expect(wrapper.findAll('.discover-card__author').map(a => a.text())).toEqual(['Jane Austen', 'Auteur inconnu'])
        expect(wrapper.find('.discover-errors').text()).toContain('le catalogue n’a pas répondu (HTTP 503)')
    })
})
