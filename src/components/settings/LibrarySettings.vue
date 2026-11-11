<template>
    <SettingsSection id="library">
        <SettingRow :label="$t('settings.libraryPath.title')" :description="$t('settings.libraryPath.description')" stacked>
            <div class="settings-stack">
                <div class="settings-path">
                    <span class="settings-path__value" :title="usersStore.libraryRoot ?? undefined">{{ usersStore.libraryRoot ?? '—' }}</span>
                    <Badge v-if="!settingsStore.libraryRootOverride" tone="neutral">{{ $t('settings.libraryPath.isDefault') }}</Badge>
                    <Badge v-else-if="pathStatus === 'checking'" tone="neutral" icon="loader">{{ $t('settings.libraryPath.checking') }}</Badge>
                    <Badge v-else-if="pathStatus === 'ok'" tone="success" icon="check">{{ $t('settings.libraryPath.reachable') }}</Badge>
                    <Badge v-else-if="pathStatus === 'unreachable'" tone="warning" icon="alert-triangle">{{ $t('settings.libraryPath.unreachable') }}</Badge>
                </div>
                <div class="settings-actions">
                    <BaseButton icon="folder" size="sm" @click="browseLibraryPath">{{ $t('settings.libraryPath.browse') }}</BaseButton>
                    <BaseButton
                        v-if="settingsStore.libraryRootOverride"
                        variant="ghost"
                        size="sm"
                        icon="rotate-ccw"
                        @click="confirmingReset = true"
                    >
                        {{ $t('settings.libraryPath.reset') }}
                    </BaseButton>
                    <SaveStatus for="library.folder" />
                </div>
            </div>
        </SettingRow>

        <SettingRow :label="$t('settings.rescan.title')" :description="$t('settings.rescan.description')">
            <BaseButton icon="refresh-cw" :loading="rescanning" :disabled="!libraryStore.libraryPath" @click="rescanLibrary">
                {{ rescanning ? $t('settings.rescan.scanning') : $t('settings.rescan.button') }}
            </BaseButton>
        </SettingRow>

        <SettingRow :label="$t('settings.users.title')">
            <template #description>
                {{ $t('settings.users.description') }}
                <span class="settings-hint">
                    {{ $t('settings.users.current', { name: userName }) }} · {{ $t('settings.users.count', { n: usersStore.users.length }) }}
                </span>
            </template>
            <BaseButton icon="repeat" @click="usersStore.clearUser()">{{ $t('settings.users.switch') }}</BaseButton>
        </SettingRow>

        <SettingRow :label="$t('settings.calibre.title')" :description="$t('settings.calibre.description')">
            <BaseButton icon="download" @click="showCalibre = true">{{ $t('settings.calibre.open') }}</BaseButton>
        </SettingRow>

        <CalibreImportModal v-if="showCalibre" @close="showCalibre = false" @imported="onImported" />

        <!-- Going back to the default folder: the books in the chosen one aren't touched, but leave the app. -->
        <ConfirmDialog
            v-if="confirmingReset"
            :title="$t('settings.libraryPath.resetTitle')"
            :message="$t('settings.libraryPath.resetBody', { path: settingsStore.libraryRootOverride ?? '' })"
            :confirm-label="$t('settings.libraryPath.reset')"
            @confirm="resetLibraryPath"
            @cancel="confirmingReset = false"
        />
    </SettingsSection>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { open } from '@tauri-apps/plugin-dialog'
    import * as api from '@/services/api'
    import { analytics } from '@/services/analytics'
    import { bucketCount } from '@/services/privacy'
    import { useLibraryStore } from '@/stores/library'
    import { useSettingsStore } from '@/stores/settings'
    import { useUsersStore } from '@/stores/users'
    import { useNotificationsStore } from '@/stores/notifications'
    import { useSaveStatusStore } from '@/stores/saveStatus'
    import { errorMessage } from '@/utils/errors'
    import SettingRow from '@/components/ui/SettingRow.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import Badge from '@/components/ui/Badge.vue'
    import ConfirmDialog from '@/components/ConfirmDialog.vue'
    import CalibreImportModal from '@/components/CalibreImportModal.vue'
    import SettingsSection from './SettingsSection.vue'
    import SaveStatus from './SaveStatus.vue'
    import type { Book } from '@/types'

    type PathStatus = 'idle' | 'checking' | 'ok' | 'unreachable'

    /**
     * Settings › Library: the library folder (change it, or go back to the
     * default after confirming; whether it can be reached), rescan, who is
     * signed in (switch user), and the Calibre import.
     *
     * A new folder reopens everything from it: `usersStore.init()` reads the
     * users there and opens the current user's library (or shows the user
     * picker), so nothing keeps pointing at the old folder.
     */
    export default defineComponent({
        name: 'LibrarySettings',

        components: { SettingsSection, SettingRow, BaseButton, Badge, ConfirmDialog, CalibreImportModal, SaveStatus },

        data(): { pathStatus: PathStatus, rescanning: boolean, showCalibre: boolean, confirmingReset: boolean } {
            return { pathStatus: 'idle', rescanning: false, showCalibre: false, confirmingReset: false }
        },

        computed: {
            ...mapStores(useLibraryStore, useSettingsStore, useUsersStore, useNotificationsStore, useSaveStatusStore),

            userName(): string { return this.usersStore.currentUser ?? '' },
        },

        async mounted() {
            if (this.settingsStore.libraryRootOverride) {
                await this.checkPath(this.settingsStore.libraryRootOverride)
            }
        },

        methods: {
            async checkPath(path: string) {
                this.pathStatus = 'checking'
                try {
                    this.pathStatus = await api.checkPathAccessible(path) ? 'ok' : 'unreachable'
                } catch {
                    this.pathStatus = 'unreachable'
                }
            },

            async browseLibraryPath() {
                const path = await open({ directory: true, multiple: false, title: this.$t('settings.libraryPath.browseTitle') })
                if (!path || typeof path !== 'string') { return }
                void this.saveStatusStore.track('library.folder', this.settingsStore.setLibraryRootOverride(path))
                await this.reloadUsers()
                await this.checkPath(path)
            },

            async resetLibraryPath() {
                this.confirmingReset = false
                void this.saveStatusStore.track('library.folder', this.settingsStore.setLibraryRootOverride(null))
                this.pathStatus = 'idle'
                await this.reloadUsers()
            },

            async reloadUsers() {
                try {
                    await this.usersStore.init()
                } catch (e) {
                    this.notificationsStore.push('error', this.$t('errors.libraryOpenFailed', { error: errorMessage(e) }))
                }
            },

            async rescanLibrary() {
                if (!this.libraryStore.libraryPath || this.rescanning) { return }
                this.rescanning = true
                try {
                    await this.libraryStore.refresh('manual')
                } finally {
                    this.rescanning = false
                }
            },

            onImported(books: Book[]) {
                // The Calibre store has already added them to the library.
                analytics.track('calibre_import', { book_count_bucket: bucketCount(books.length) })
            },
        },
    })
</script>
