<template>
    <SettingsSection id="backup">
        <SettingRow :label="$t('settings.backup.exportTitle')">
            <template #description>
                {{ $t('settings.backup.exportDescription') }}
                <span v-if="exporting" class="settings-hint" aria-live="polite">{{ progressText }}</span>
                <span v-if="exportError" class="settings-error backup-settings__error" role="alert">
                    <vue-feather type="alert-circle" size="14" aria-hidden="true" />
                    {{ exportError }}
                </span>
            </template>
            <div v-if="exporting" class="backup-settings__running">
                <ProgressBar
                    class="backup-settings__bar"
                    :value="backupStore.percent"
                    :label="$t('settings.backup.exporting')"
                    :value-text="progressText"
                    show-value
                />
                <BaseButton variant="ghost" size="sm" :disabled="backupStore.cancelling" @click="cancel">
                    {{ backupStore.cancelling ? $t('settings.backup.cancelling') : $t('common.cancel') }}
                </BaseButton>
            </div>
            <BaseButton
                v-else
                icon="archive"
                :disabled="!usersStore.libraryRoot || backupStore.running !== null"
                @click="exportBackup"
            >
                {{ $t('settings.backup.export') }}
            </BaseButton>
        </SettingRow>

        <SettingRow :label="$t('settings.backup.restoreTitle')">
            <template #description>
                {{ $t('settings.backup.restoreDescription') }}
                <span v-if="restoreError" class="settings-error backup-settings__error" role="alert">
                    <vue-feather type="alert-circle" size="14" aria-hidden="true" />
                    {{ restoreError }}
                </span>
            </template>
            <BaseButton
                icon="rotate-ccw"
                :loading="reading"
                :disabled="!usersStore.libraryRoot || backupStore.running !== null"
                @click="chooseBackup"
            >
                {{ $t('settings.backup.restore') }}
            </BaseButton>
        </SettingRow>

        <RestoreBackupDialog
            v-if="chosen"
            :path="chosen.path"
            :manifest="chosen.manifest"
            @close="chosen = null"
        />
    </SettingsSection>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { open } from '@tauri-apps/plugin-dialog'
    import { readBackup } from '@/services/api'
    import { useBackupStore } from '@/stores/backup'
    import { useNotificationsStore } from '@/stores/notifications'
    import { useUsersStore } from '@/stores/users'
    import { errorMessage } from '@/utils/errors'
    import SettingRow from '@/components/ui/SettingRow.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import ProgressBar from '@/components/ui/ProgressBar.vue'
    import type { BackupManifest } from '@/types'
    import RestoreBackupDialog from './RestoreBackupDialog.vue'
    import SettingsSection from './SettingsSection.vue'
    import { backupProgressText } from './backup'

    /**
     * Settings › Backup: back up the whole library (every user, their books,
     * covers and metadata) into a zip in a folder the user picks, with
     * progress and Cancel; and restore one, after RestoreBackupDialog's
     * warning that everything there now is erased. The work and its toasts
     * are the backup store's; this shows it and the errors.
     */
    export default defineComponent({
        name: 'BackupSettings',

        components: { SettingsSection, SettingRow, BaseButton, ProgressBar, RestoreBackupDialog },

        data() {
            return {
                exportError: '',
                restoreError: '',
                /** `read_backup` is reading the chosen file. */
                reading: false,
                /** The backup to restore, once read: the dialog is open. */
                chosen: null as { path: string, manifest: BackupManifest } | null,
            }
        },

        computed: {
            ...mapStores(useBackupStore, useUsersStore),

            exporting(): boolean {
                return this.backupStore.running === 'export'
            },

            progressText(): string {
                return backupProgressText(this.backupStore.progress)
            },
        },

        methods: {
            async exportBackup() {
                this.exportError = ''
                const dir = await open({ directory: true, multiple: false, title: this.$t('settings.backup.exportPickTitle') })
                if (typeof dir !== 'string') { return }
                try {
                    const result = await this.backupStore.exportTo(dir)
                    if (result.cancelled) {
                        useNotificationsStore().push('info', this.$t('settings.backup.exportCancelled'))
                    }
                } catch (e) {
                    this.exportError = this.$t('settings.backup.exportFailed', { error: errorMessage(e) })
                }
            },

            async chooseBackup() {
                this.restoreError = ''
                const path = await open({
                    multiple: false,
                    directory: false,
                    title: this.$t('settings.backup.restorePickTitle'),
                    filters: [{ name: this.$t('settings.backup.fileType'), extensions: ['zip'] }],
                })
                if (typeof path !== 'string') { return }
                this.reading = true
                try {
                    this.chosen = { path, manifest: await readBackup(path) }
                } catch (e) {
                    this.restoreError = errorMessage(e)
                } finally {
                    this.reading = false
                }
            },

            async cancel() {
                try {
                    await this.backupStore.cancel()
                } catch (e) {
                    this.exportError = this.$t('settings.backup.cancelFailed', { error: errorMessage(e) })
                }
            },
        },
    })
</script>

<style lang="scss" scoped>
.backup-settings__running {
    display: flex;
    align-items: center;
    gap: var(--space-3);
}

.backup-settings__bar {
    width: 200px;
}

.backup-settings__error {
    margin-top: var(--space-2);
}
</style>
