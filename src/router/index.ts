import { createRouter, createWebHistory, type RouteRecordRaw } from 'vue-router'
import LibraryView from '@/views/LibraryView.vue'
import SettingsView from '@/views/SettingsView.vue'
import DiscoverView from '@/views/DiscoverView.vue'

/**
 * The app's screens. Old addresses still lead somewhere: `/options` is now
 * Settings (its `#section` kept, e.g. `#mtp-setting`), `/store` is Discover,
 * the Transfers page became the sidebar's Activity popover, and the Device
 * view is the library showing the device's books.
 */
export const routes: RouteRecordRaw[] = [
    { path: '/', redirect: '/library' },
    { path: '/library', name: 'library', component: LibraryView },
    { path: '/discover', name: 'discover', component: DiscoverView },
    { path: '/settings', name: 'settings', component: SettingsView },
    // Former names.
    { path: '/options', redirect: to => ({ path: '/settings', hash: to.hash }) },
    { path: '/store', redirect: '/discover' },
    { path: '/transfers', redirect: '/library' },
    { path: '/device', redirect: { path: '/library', query: { scope: 'device' } } },
]

const router = createRouter({
    history: createWebHistory(),
    routes,
})

export default router
