<template>
    <AboutModal v-if="aboutOpen" @close="aboutOpen = false" />
    <ShortcutsDialog v-if="shortcutsOpen" @close="shortcutsOpen = false" />
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import * as api from '@/services/api'
    import { useShortcutsStore, type ShortcutHandler } from '@/stores/shortcuts'
    import { useImportsStore } from '@/stores/imports'
    import { useDeviceStore } from '@/stores/device'
    import { useLibraryStore } from '@/stores/library'
    import { useBookActionsStore } from '@/stores/bookActions'
    import { useNotificationsStore } from '@/stores/notifications'
    import { errorMessage } from '@/utils/errors'
    import type { ShortcutId } from '@/constants/shortcuts'
    import AboutModal from '@/components/AboutModal.vue'
    import ShortcutsDialog from '@/components/ShortcutsDialog.vue'

    /**
     * The app's commands that work from any screen (constants/shortcuts.ts),
     * and the native menu bar.
     *
     * - **App-level commands**: Settings (⌘,), About, Keyboard shortcuts
     *   (⌘/), the log file, privacy, import (⌘O), look for an e-reader (⌘K),
     *   grid/list (⌘1/⌘2) and Find duplicates. Their handlers are registered
     *   with the shortcuts store, so a key, a menu click and a button run the
     *   same code.
     * - **Library commands from elsewhere**: the library view registers
     *   Search (⌘F) and Show details (⌘I) while it is shown. From another
     *   screen, the fallbacks here open the library first and run the
     *   command once it is there (`runWhenRegistered`). They are registered
     *   in `created`, before any view, so the view's own handler always
     *   runs first.
     * - **The menu bar** (src-tauri/src/menu.rs, macOS): each click arrives
     *   as a `menu-command` event with the command id, run through
     *   `runCommand(id, 'menu')` (same rules as the keys: nothing runs
     *   behind a dialog).
     *
     * It also renders the two dialogs these commands open: About and
     * Keyboard shortcuts. Mounted by App.vue while a user is selected.
     */
    export default defineComponent({
        name: 'AppCommands',

        components: { AboutModal, ShortcutsDialog },

        data(): { aboutOpen: boolean, shortcutsOpen: boolean, unlistenMenu: (() => void) | null, unmounted: boolean } {
            return { aboutOpen: false, shortcutsOpen: false, unlistenMenu: null, unmounted: false }
        },

        computed: {
            ...mapStores(
                useShortcutsStore, useImportsStore, useDeviceStore, useLibraryStore, useBookActionsStore,
                useNotificationsStore,
            ),

            onLibrary(): boolean {
                return this.$route.name === 'library'
            },
        },

        created() {
            for (const [id, handler] of this.handlers()) { this.shortcutsStore.register(id, handler) }
        },

        async mounted() {
            try {
                const unlisten = await api.onMenuCommand(id => { this.shortcutsStore.runCommand(id, 'menu') })
                // Unmounted while the listener was being set up: drop it at once.
                if (this.unmounted) { unlisten() } else { this.unlistenMenu = unlisten }
            } catch (e) {
                console.error('[menu] could not listen for menu commands', e)
            }
        },

        beforeUnmount() {
            this.unmounted = true
            this.unlistenMenu?.()
            this.unlistenMenu = null
            for (const [id, handler] of this.handlers()) { this.shortcutsStore.unregister(id, handler) }
        },

        methods: {
            /** Every command handled here (the same functions are unregistered on unmount). */
            handlers(): [ShortcutId, ShortcutHandler][] {
                return [
                    ['app.settings', this.openSettings],
                    ['app.about', this.openAbout],
                    ['help.shortcuts', this.openShortcuts],
                    ['help.logs', this.openLogFolder],
                    ['help.privacy', this.openPrivacy],
                    ['library.import', this.importBooks],
                    ['library.detectDevice', this.detectDevice],
                    ['view.grid', this.showGrid],
                    ['view.list', this.showList],
                    ['library.findDuplicates', this.findDuplicates],
                    ['library.export', this.exportFromElsewhere],
                    ['library.search', this.searchFromElsewhere],
                    ['view.toggleDetails', this.toggleDetailsFromElsewhere],
                ]
            },

            openSettings() {
                void this.$router.push('/settings')
            },

            openPrivacy() {
                void this.$router.push({ path: '/settings', hash: '#privacy' })
            },

            openAbout() {
                this.aboutOpen = true
            },

            openShortcuts() {
                this.shortcutsOpen = true
            },

            openLogFolder() {
                api.openLogFolder().catch((e: unknown) => {
                    this.notificationsStore.push('error', this.$t('settings.logs.failed', { error: errorMessage(e) }))
                })
            },

            importBooks() {
                void this.importsStore.pickAndImport()
            },

            detectDevice() {
                void this.deviceStore.detectDevice()
            },

            showGrid() {
                this.libraryStore.setViewMode('grid')
                this.showLibrary()
            },

            showList() {
                this.libraryStore.setViewMode('list')
                this.showLibrary()
            },

            /** The Duplicates review is one of the library view's dialogs (LibraryDialogs). */
            findDuplicates() {
                this.bookActionsStore.openDuplicates()
                this.showLibrary()
            },

            /** The menu's Export… away from the library: there is no book shown to export. */
            exportFromElsewhere(): boolean {
                if (this.onLibrary) { return false }
                this.notificationsStore.push('info', this.$t('shortcuts.exportNothing'))
                return true
            },

            searchFromElsewhere(): boolean {
                return this.runInLibraryView('library.search')
            },

            toggleDetailsFromElsewhere(): boolean {
                return this.runInLibraryView('view.toggleDetails')
            },

            /**
             * A library-view command from another screen: open the library,
             * then run the command there once the view has registered it.
             * On the library, the view's own handler ran first; if it didn't
             * apply, neither does this.
             */
            runInLibraryView(id: ShortcutId): boolean {
                if (this.onLibrary) { return false }
                this.shortcutsStore.runWhenRegistered(id)
                void this.$router.push('/library')
                return true
            },

            showLibrary() {
                if (!this.onLibrary) { void this.$router.push('/library') }
            },
        },
    })
</script>
