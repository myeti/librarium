import { defineStore } from 'pinia'
import * as api from '@/services/api'
import { useCalibreStore } from '@/stores/calibre'
import { useDeviceStore } from '@/stores/device'
import { useImportsStore } from '@/stores/imports'
import { useLibraryStore } from '@/stores/library'
import { useNotificationsStore } from '@/stores/notifications'
import { useOpdsStore } from '@/stores/opds'
import { useUsersStore } from '@/stores/users'
import { t } from '@/i18n'
import { fmtDate, fmtSize } from '@/utils/format'
import type { BackupExported, BackupManifest, BackupProgress, BackupRestored } from '@/types'

interface BackupState {
    /** What runs (one at a time: the backend refuses a second). */
    running: 'export' | 'restore' | null
    /** Cancel was asked for. */
    cancelling: boolean
    /** The running operation's progress (`backup-progress`); `null` when none runs. */
    progress: BackupProgress | null
}

/** Base name of a path, `/` or `\` separated. */
function fileName(path: string): string {
    return path.split(/[\\/]/).pop() ?? path
}

/**
 * Backups (Settings › Backup; docs/IPC.md, "Backup"): writing one of the
 * whole library root (every user, their books and Librarium's data about
 * them) as a zip, and restoring one, which replaces every user.
 *
 * A restore closes the library first (the backend swaps the user folders
 * under it) and reloads the users afterwards, like a fresh start: with
 * several users, the user picker is shown. When it is cancelled or fails,
 * the backend has left the library as it was, and it is reopened.
 *
 * The success toasts are pushed here, not by the view: after a restore the
 * Settings screen may already be gone (the user picker replaced it).
 */
export const useBackupStore = defineStore('backup', {
    state: (): BackupState => ({
        running:    null,
        cancelling: false,
        progress:   null,
    }),

    getters: {
        /** 0–100 by bytes copied, for a progress bar; `null` while listing files or replacing (indeterminate). */
        percent(state): number | null {
            const p = state.progress
            if (!p || p.phase !== 'copying' || p.bytesTotal === 0) { return null }
            return Math.min(100, Math.round(p.bytesDone / p.bytesTotal * 100))
        },

        /**
         * Why a restore can't start now (a message), or `null`: books are
         * being imported or sent, and the files they write would be erased
         * or end in the wrong library.
         */
        restoreBlocker(): string | null {
            if (useImportsStore().busy || useCalibreStore().running) { return t('settings.backup.busyImporting') }
            if (useDeviceStore().activeJobs.length > 0) { return t('settings.backup.busyTransferring') }
            return null
        },
    },

    actions: {
        /** The library root, which the backend checks is the settings' one. */
        requireRoot(): string {
            const root = useUsersStore().libraryRoot
            if (!root) { throw new Error(`NOT_FOUND: ${t('settings.backup.noRoot')}`) }
            return root
        },

        /** Runs `work` as the operation `kind`, following its progress. */
        async track<T>(kind: 'export' | 'restore', work: () => Promise<T>): Promise<T> {
            // What the backend would answer; `errorCode` reads the prefix.
            if (this.running) { throw new Error(`CONFLICT: ${t('settings.backup.alreadyRunning')}`) }
            this.running = kind
            this.cancelling = false
            this.progress = { phase: 'scanning', filesDone: 0, filesTotal: 0, bytesDone: 0, bytesTotal: 0 }
            // Listening first: the backend's first event comes right away.
            const unlisten = await api.onBackupProgress(progress => {
                if (this.running === kind) { this.progress = progress }
            })
            try {
                return await work()
            } finally {
                unlisten()
                this.running = null
                this.cancelling = false
                this.progress = null
            }
        },

        /**
         * Writes a backup into the folder `destDir`. Resolves with the
         * backend's result (also when cancelled); rejects with its error.
         * Success is told by a toast, with a button showing the file.
         */
        async exportTo(destDir: string): Promise<BackupExported> {
            const root = this.requireRoot()
            const result = await this.track('export', () => api.exportBackup(root, destDir))
            if (result.path) {
                useNotificationsStore().push('success', t('settings.backup.exported', {
                    name: fileName(result.path),
                    size: fmtSize(result.bytes),
                }), {
                    durationMs: 10000,
                    action: { label: t('settings.backup.showFile'), run: () => api.revealBackup() },
                })
            }
            return result
        },

        /**
         * Replaces every user with the backup's at `path` (`manifest`: what
         * `read_backup` said of it, for the toast). Resolves with the
         * backend's result (also when cancelled: nothing was replaced);
         * rejects with its error.
         */
        async restoreFrom(path: string, manifest: BackupManifest): Promise<BackupRestored> {
            const root = this.requireRoot()
            if (this.running) { throw new Error(`CONFLICT: ${t('settings.backup.alreadyRunning')}`) }
            const blocker = this.restoreBlocker
            if (blocker) { throw new Error(`CONFLICT: ${blocker}`) }
            const users = useUsersStore()
            // Closed, not signed out: the user picker would replace the
            // screen in the middle of the restore. The OPDS server is
            // stopped first: it may be reading a book being moved.
            await useOpdsStore().setLibraryRoot(null)
            await useLibraryStore().openLibrary(null)
            let restored: BackupRestored
            try {
                restored = await this.track('restore', () => api.restoreBackup(root, path))
            } catch (e) {
                // Failed: the backend left (or put back) everything as it was.
                await users.retry()
                throw e
            }
            if (restored.cancelled) {
                await users.retry()
                return restored
            }
            useNotificationsStore().push('success', t('settings.backup.restored', {
                n: restored.users.length,
                date: fmtDate(manifest.createdAt),
            }), 10000)
            // A fresh start: these are new users (several: the picker). A
            // failure to list them is shown by the picker and reported by `init`.
            await users.init().catch(() => undefined)
            return restored
        },

        /**
         * Asks the running export or restore to stop (a restore only stops
         * while extracting, before anything is replaced). It then resolves
         * normally with `cancelled: true`. Rejects with the backend's error.
         */
        async cancel(): Promise<void> {
            if (!this.running || this.cancelling) { return }
            this.cancelling = true
            try {
                await api.cancelBackup()
            } catch (e) {
                this.cancelling = false
                throw e
            }
        },
    },
})
