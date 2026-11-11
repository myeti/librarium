<template>
    <BaseModal
        bare
        size="sm"
        role="alertdialog"
        :labelledby="titleId"
        :describedby="bodyId"
        :dismissible="!running"
        :show-close="false"
        @close="close"
    >
        <div class="restore-dialog">
            <!-- Before: what will be erased, and by what. -->
            <template v-if="!running">
                <h2 :id="titleId" class="restore-dialog__title">
                    <vue-feather type="alert-triangle" size="16" class="restore-dialog__icon" aria-hidden="true" />
                    {{ $t('settings.backup.confirmTitle') }}
                </h2>
                <div :id="bodyId" class="restore-dialog__body">
                    <p class="restore-dialog__message">{{ $t('settings.backup.confirmMessage') }}</p>
                    <dl class="restore-dialog__facts">
                        <dt>{{ $t('settings.backup.factErased') }}</dt>
                        <dd class="-erased">{{ erasedText }}</dd>
                        <dt>{{ $t('settings.backup.factBackup') }}</dt>
                        <dd>
                            {{ $t('settings.backup.madeOn', { date: madeOn, version: manifest.appVersion }) }}
                            <span class="restore-dialog__sub">{{ backupText }}</span>
                        </dd>
                        <dt>{{ $t('settings.backup.factFolder') }}</dt>
                        <dd class="-path" :title="usersStore.libraryRoot ?? undefined">{{ usersStore.libraryRoot }}</dd>
                    </dl>
                </div>
                <p v-if="backupStore.restoreBlocker" class="settings-warning restore-dialog__notice">
                    <vue-feather type="clock" size="14" aria-hidden="true" />
                    {{ backupStore.restoreBlocker }}
                </p>
                <Checkbox v-model="understood" class="restore-dialog__check" :label="$t('settings.backup.confirmCheck')" />
                <p v-if="error" class="settings-error restore-dialog__notice" role="alert">
                    <vue-feather type="alert-circle" size="14" aria-hidden="true" />
                    {{ error }}
                </p>
                <div class="restore-dialog__actions">
                    <BaseButton variant="ghost" data-autofocus @click="close">{{ $t('common.cancel') }}</BaseButton>
                    <BaseButton variant="danger" icon="rotate-ccw" :disabled="!canRestore" @click="restore">
                        {{ $t('settings.backup.confirmButton') }}
                    </BaseButton>
                </div>
            </template>

            <!-- During: progress; Cancel until the users are being swapped. -->
            <template v-else>
                <h2 :id="titleId" class="restore-dialog__title">{{ $t('settings.backup.restoring') }}</h2>
                <div :id="bodyId" class="restore-dialog__body">
                    <ProgressBar
                        :value="backupStore.percent"
                        :label="$t('settings.backup.restoring')"
                        :value-text="progressText"
                        size="md"
                        show-value
                    />
                    <p class="restore-dialog__progress" aria-live="polite">{{ progressText }}</p>
                    <p class="restore-dialog__sub">{{ $t('settings.backup.restoringHint') }}</p>
                </div>
                <div class="restore-dialog__actions">
                    <BaseButton ref="cancelRunning" variant="ghost" :disabled="!canCancel" @click="cancel">
                        {{ backupStore.cancelling ? $t('settings.backup.cancelling') : $t('common.cancel') }}
                    </BaseButton>
                </div>
            </template>
        </div>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import { useBackupStore } from '@/stores/backup'
    import { useNotificationsStore } from '@/stores/notifications'
    import { useUsersStore } from '@/stores/users'
    import { errorMessage } from '@/utils/errors'
    import { fmtDate, fmtSize, fmtTime } from '@/utils/format'
    import { uniqueId } from '@/utils/dom'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import Checkbox from '@/components/ui/Checkbox.vue'
    import ProgressBar from '@/components/ui/ProgressBar.vue'
    import type { BackupManifest } from '@/types'
    import { backupProgressText } from './backup'

    /** "Alice, Clara, Zoé". */
    function listOf(names: string[]): string {
        return names.join(', ')
    }

    /**
     * Restoring a backup, which erases the library: first the warning, with
     * what goes (the users there now), what comes (the backup's date, users
     * and size) and where, behind a checkbox "I understand…" (Cancel has the
     * focus); then the progress, with Cancel while the backup is extracted
     * (afterwards the users are being swapped, which can't be stopped).
     *
     * A cancel or a failure leaves the library as it was: the dialog says
     * so (a toast; the error, in the dialog). After a restore the users are
     * reloaded by the store, which may replace the whole screen by the user
     * picker; the dialog closes either way.
     */
    export default defineComponent({
        name: 'RestoreBackupDialog',

        components: { BaseModal, BaseButton, Checkbox, ProgressBar },

        props: {
            /** The backup file. */
            path:     { type: String, required: true },
            /** What `read_backup` said of it. */
            manifest: { type: Object as PropType<BackupManifest>, required: true },
        },

        emits: ['close'],

        data() {
            return {
                titleId: uniqueId('restore-title'),
                bodyId:  uniqueId('restore-body'),
                understood: false,
                error: '',
            }
        },

        computed: {
            ...mapStores(useBackupStore, useUsersStore),

            running(): boolean {
                return this.backupStore.running === 'restore'
            },

            canRestore(): boolean {
                return this.understood && !this.backupStore.restoreBlocker && this.backupStore.running === null
            },

            /** Cancel is heard until the users are swapped. */
            canCancel(): boolean {
                return !this.backupStore.cancelling && this.backupStore.progress?.phase !== 'replacing'
            },

            madeOn(): string {
                return `${fmtDate(this.manifest.createdAt)} ${fmtTime(this.manifest.createdAt)}`
            },

            erasedText(): string {
                const users = this.usersStore.users
                return users.length > 0
                    ? this.$t('settings.backup.erasedUsers', { n: users.length, names: listOf(users) })
                    : this.$t('settings.backup.erasedNothing')
            },

            backupText(): string {
                const users = this.manifest.users
                return this.$t('settings.backup.backupHolds', {
                    n: users.length,
                    names: listOf(users),
                    files: this.manifest.files,
                    size: fmtSize(this.manifest.bytes),
                })
            },

            progressText(): string {
                return backupProgressText(this.backupStore.progress)
            },
        },

        watch: {
            /** The confirm step's buttons are gone: focus moves to Cancel. */
            running(now: boolean) {
                if (now) {
                    void this.$nextTick(() => {
                        ((this.$refs.cancelRunning as { $el?: HTMLElement } | undefined)?.$el)?.focus()
                    })
                }
            },
        },

        methods: {
            close() {
                if (!this.running) { this.$emit('close') }
            },

            async restore() {
                if (!this.canRestore) { return }
                this.error = ''
                try {
                    const result = await this.backupStore.restoreFrom(this.path, this.manifest)
                    if (result.cancelled) {
                        useNotificationsStore().push('info', this.$t('settings.backup.restoreCancelled'))
                    }
                    this.$emit('close')
                } catch (e) {
                    this.error = this.$t('settings.backup.restoreFailed', { error: errorMessage(e) })
                }
            },

            async cancel() {
                try {
                    await this.backupStore.cancel()
                } catch (e) {
                    this.error = this.$t('settings.backup.cancelFailed', { error: errorMessage(e) })
                }
            },
        },
    })
</script>

<style lang="scss" scoped>
.restore-dialog {
    padding: 20px 22px 16px;
}

.restore-dialog__title {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 10px;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
}

.restore-dialog__icon {
    flex-shrink: 0;
    color: var(--danger);
}

.restore-dialog__body {
    font-size: var(--font-size-sm);
    line-height: var(--line-height);
    color: var(--text-secondary);
}

.restore-dialog__message {
    margin: 0 0 12px;
    color: var(--text-primary);
}

.restore-dialog__facts {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 6px 14px;
    margin: 0;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--bg-elevated) 70%, transparent);
    font-size: var(--font-size-xs);

    dt {
        font-weight: var(--font-weight-semibold);
        color: var(--text-muted);
    }

    dd {
        margin: 0;
        min-width: 0;
        color: var(--text-primary);

        &.-erased { color: var(--danger); }

        &.-path {
            overflow: hidden;
            font-family: var(--font-mono);
            text-overflow: ellipsis;
            white-space: nowrap;
        }
    }
}

.restore-dialog__sub {
    display: block;
    font-size: var(--font-size-xs);
    color: var(--text-muted);
}

.restore-dialog__progress {
    margin: 10px 0 2px;
    color: var(--text-primary);
}

.restore-dialog__notice {
    margin-top: 12px;
}

.restore-dialog__check {
    margin-top: 14px;
}

.restore-dialog__actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 18px;
}
</style>
