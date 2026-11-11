import { createApp } from 'vue'
import { createPinia } from 'pinia'
import VueFeather from 'vue-feather'
import router from './router'
import App from './App.vue'
import { i18nPlugin } from './i18n'
import { shortcutsPlugin } from './stores/shortcuts'
// Bundled fonts: no network request, nothing for the CSP to block. Inter
// ('Inter Variable') is the interface; the others are the themes' cover
// lettering (themes.css): Cinzel (Saphir), Poiret One (Nocturne), Fraunces
// with optical sizes (Folklore), Italiana (Cerulean), and Cormorant Garamond
// for book titles. The browser only loads a face when a theme uses it.
import '@fontsource-variable/inter'
import '@fontsource-variable/cinzel'
import '@fontsource/poiret-one'
import '@fontsource-variable/fraunces/opsz.css'
import '@fontsource-variable/fraunces/opsz-italic.css'
import '@fontsource/italiana'
import '@fontsource-variable/cormorant-garamond'
import '@fontsource-variable/cormorant-garamond/wght-italic.css'
import './assets/main.css'

function mount(): void {
    const app = createApp(App)
    app.use(createPinia())
    // The single global keydown listener for keyboard shortcuts (needs Pinia).
    app.use(shortcutsPlugin)
    app.use(i18nPlugin)
    app.use(router)
    app.component(VueFeather.name!, VueFeather)
    app.mount('#app')
}

// `npm run vue:dev:mock`: a fake backend for working on the UI in a plain
// browser (src/dev/mockBackend.ts). The condition is a build-time constant,
// so production builds drop the import entirely.
if (import.meta.env.VITE_MOCK_BACKEND === '1') {
    void import('./dev/mockBackend').then(m => { m.installMockBackend(); mount(); m.seedMockTransfers() })
} else {
    mount()
}
