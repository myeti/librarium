import { defineStore } from 'pinia'
import * as api from '@/services/api'
import { useLibraryStore } from '@/stores/library'
import { errorCode } from '@/utils/errors'
import { t } from '@/i18n'
import type { CalibreImportProgress, CalibreImportResult } from '@/types'

interface CalibreImportState {
    /** An import is running (one at a time: the backend refuses a second). */
    running: boolean
    /** Cancel was asked for; the backend stops before its next book. */
    cancelling: boolean
    /** The running import's progress (`calibre-import-progress`); `null` when none runs. */
    progress: CalibreImportProgress | null
}

/**
 * Rejections after which the library wasn't touched: another import is
 * running, or the Calibre library itself can't be read. Any other rejection
 * (the library became unreachable, its index couldn't be saved after books
 * were copied) leaves the book list out of date.
 */
const LIBRARY_UNTOUCHED = new Set(['CONFLICT', 'NOT_FOUND', 'INVALID_INPUT'])

/**
 * The Calibre import (docs/IPC.md, "Calibre import"): runs it, follows its
 * progress, cancels it, and brings the library's book list up to date
 * afterwards — whatever the outcome.
 */
export const useCalibreStore = defineStore('calibre', {
    state: (): CalibreImportState => ({
        running:    false,
        cancelling: false,
        progress:   null,
    }),

    getters: {
        /** 0–100, for a progress bar. */
        percent(state): number {
            const p = state.progress
            return p && p.total > 0 ? Math.min(100, Math.round(p.done / p.total * 100)) : 0
        },
    },

    actions: {
        /**
         * Imports Calibre books `bookIds` from `calibrePath` into the open
         * library. Resolves with the backend's result (also when cancelled:
         * `cancelled: true`); rejects with its error.
         *
         * Afterwards the book list matches what the backend saved: imported
         * books are added (which also refreshes the OPDS server's cache). If
         * the import stopped on a library problem (`result.error`, or a
         * rejection that may have copied files), the library is rescanned.
         */
        async importBooks(calibrePath: string, bookIds: number[]): Promise<CalibreImportResult> {
            const library = useLibraryStore()
            const { libraryPath, session } = library.requireLibrary()
            // What the backend would answer; `errorCode` reads the prefix.
            if (this.running) { throw new Error(`CONFLICT: ${t('settings.calibre.alreadyRunning')}`) }

            this.running = true
            this.cancelling = false
            this.progress = { done: 0, total: new Set(bookIds).size, currentTitle: null }
            // Listening first: the backend's first event may come right away.
            const unlisten = await api.onCalibreImportProgress(progress => {
                if (this.running) { this.progress = progress }
            })
            try {
                const result = await api.importFromCalibre(calibrePath, bookIds, libraryPath)
                if (session === library.session) {
                    library.addBooks(result.imported)
                    if (result.error) { await this.resync() }
                }
                return result
            } catch (e) {
                if (session === library.session && !LIBRARY_UNTOUCHED.has(errorCode(e) ?? '')) {
                    await this.resync()
                }
                throw e
            } finally {
                unlisten()
                this.running = false
                this.cancelling = false
                this.progress = null
            }
        },

        /**
         * Asks the running import to stop before its next book. The import
         * then resolves normally (`cancelled: true`), keeping what it had
         * imported. Rejects with the backend's error, if any.
         */
        async cancel(): Promise<void> {
            if (!this.running || this.cancelling) { return }
            this.cancelling = true
            try {
                await api.cancelCalibreImport()
            } catch (e) {
                this.cancelling = false
                throw e
            }
        },

        /** Rescans the library (the backend may have copied books it couldn't index) and tells the OPDS server. */
        async resync() {
            const library = useLibraryStore()
            await library.refresh('import')
            library.libraryChanged()
        },
    },
})
