import { beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@/utils/sounds', () => ({ playNewUser: vi.fn(), playIntro: vi.fn(), playTransferComplete: vi.fn() }))
import { mockInvoke } from '@/test/tauri'
import UserPicker from './UserPicker.vue'
import { useUsersStore } from '@/stores/users'
import { currentLang, i18nPlugin, t } from '@/i18n'

/** The picker, with the users store as `state` leaves it. */
function mountPicker(state: Partial<ReturnType<typeof useUsersStore>['$state']>) {
    const pinia = createPinia()
    setActivePinia(pinia)
    const store = useUsersStore()
    store.$patch({ libraryRoot: '/Volumes/books', ...state })
    const wrapper = mount(UserPicker, {
        global: { plugins: [pinia, i18nPlugin], stubs: { 'vue-feather': true } },
    })
    return { store, wrapper }
}

beforeEach(() => {
    mockInvoke.reset()
    currentLang.value = 'en'
})

describe('UserPicker when the users can’t be listed', () => {
    it('explains a permission refusal with the backend’s message, and offers Retry — not "Add user"', async () => {
        const message = 'Librarium isn’t allowed to open the library folder "books". In System Settings › Privacy & Security › Files and Folders, allow Librarium to access Network Volumes…'
        const { wrapper, store } = mountPicker({ unavailable: true, errorCode: 'PERMISSION_DENIED', error: message, currentUser: 'ann' })

        const alert = wrapper.find('[role="alert"]')
        expect(alert.find('h2').text()).toBe(t('library.permissionDeniedTitle'))
        expect(alert.find('p').text()).toBe(message)
        expect(wrapper.find('.user-card--add').exists()).toBe(false)

        mockInvoke.on('list_users', () => ['ann'])
        mockInvoke.on('scan_library', () => [])
        mockInvoke.on('library_lock_status', () => ({ otherInstance: null }))
        await alert.find('button').trigger('click')
        await flushPromises()
        expect(mockInvoke.callsTo('list_users')).toHaveLength(1)
        expect(store.unavailable).toBe(false)
    })

    it('keeps the "not reachable" wording for a missing folder', () => {
        const { wrapper } = mountPicker({ unavailable: true, errorCode: 'LIBRARY_UNAVAILABLE', error: 'The library folder "books" can’t be reached.' })
        expect(wrapper.find('[role="alert"] h2').text()).toBe(t('library.unavailableTitle'))
        expect(wrapper.find('[role="alert"] p').text()).toBe(t('library.unavailableBody', { path: '/Volumes/books' }))
    })

    it('shows any other failure as it is', () => {
        const { wrapper } = mountPicker({ unavailable: true, errorCode: 'IO', error: 'Could not read the folder books: Input/output error' })
        expect(wrapper.find('[role="alert"] h2').text()).toBe(t('user.listFailedTitle'))
        expect(wrapper.find('[role="alert"] p').text()).toBe('Could not read the folder books: Input/output error')
        expect(wrapper.find('[role="alert"] button').exists()).toBe(true)
    })
})
