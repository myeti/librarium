import { defineStore } from 'pinia'
import { open } from '@tauri-apps/plugin-dialog'
import { useLibraryStore, CONVERTIBLE_FORMATS } from '@/stores/library'
import { useNotificationsStore } from '@/stores/notifications'
import { reportError } from '@/services/librarium-api'
import { errorMessage } from '@/utils/errors'
import { t } from '@/i18n'
import type { Book } from '@/types'

/** File types the library imports (the backend checks again). */
export const IMPORTABLE_EXTENSIONS = ['epub', 'pdf', 'mobi', 'azw3', 'cbz'] as const

const IMPORTABLE = new RegExp(`\\.(${IMPORTABLE_EXTENSIONS.join('|')})$`, 'i')

/**
 * Within this window an identical drop is a repeat of the previous one, not
 * a second import: Windows (WebView2) can deliver the same drop payload
 * twice for one gesture, which would import every file twice.
 */
export const DROP_REPEAT_WINDOW_MS = 2000

/** The paths of `paths` the library can import, each once, in order. */
export function importablePaths(paths: readonly string[]): string[] {
    return [...new Set(paths)].filter(p => IMPORTABLE.test(p))
}

/**
 * What the window's drag-and-drop listener receives (Tauri's
 * `DragDropEvent`, reduced to what is used here).
 */
export type FileDropEvent =
    | { type: 'enter' | 'over' | 'leave' }
    | { type: 'drop', paths: string[] }

interface ImportsState {
    /** Files are being dragged over the window: the drop overlay is shown. */
    dragging: boolean
    /** Imports waiting or running. */
    pending: number
    /** Books just imported that could be converted to EPUB (the offer dialog shows them). */
    conversionOffer: Book[]
    /** The import chain: each import starts when the previous one has settled. */
    queue: Promise<void>
    /** The last drop taken, to ignore a repeat of it (see DROP_REPEAT_WINDOW_MS). */
    lastDrop: { key: string, at: number } | null
}

/**
 * Importing files into the library, from anywhere in the app: the Import
 * button and ⌘O (file picker) and files dropped on the window, whatever
 * view is showing (FileDropTarget, mounted by App.vue, feeds it the
 * window's drag-and-drop events).
 *
 * - Imports run **one at a time**: two `import_books` calls in flight
 *   would each load, modify and save the library index, and one would
 *   lose the other's books.
 * - A failure is shown as a toast (and reported); the other imports go on.
 * - Imported MOBI/AZW/AZW3 books are offered for EPUB conversion
 *   (`conversionOffer`, shown by FileDropTarget).
 */
export const useImportsStore = defineStore('imports', {
    state: (): ImportsState => ({
        dragging: false,
        pending: 0,
        conversionOffer: [],
        queue: Promise.resolve(),
        lastDrop: null,
    }),

    getters: {
        busy: (state): boolean => state.pending > 0,
    },

    actions: {
        /**
         * Imports `paths` after the imports already queued. Resolves with
         * the books added (none when it failed: the failure was shown).
         */
        importPaths(paths: readonly string[]): Promise<Book[]> {
            const files = [...paths]
            this.pending++
            const run = this.queue.then(() => this.runImport(files))
            this.queue = run.then(() => undefined, () => undefined)
            return run.finally(() => { this.pending-- })
        },

        async runImport(paths: string[]): Promise<Book[]> {
            let imported: Book[]
            try {
                imported = await useLibraryStore().importBooks(paths)
            } catch (e) {
                reportError('IMPORT_FAILED', 'import.files', e)
                useNotificationsStore().push('error', t('errors.importFailed', { error: errorMessage(e) }))
                return []
            }
            const convertible = imported.filter(b => CONVERTIBLE_FORMATS.includes(b.format.toLowerCase()))
            if (convertible.length > 0) { this.conversionOffer = convertible }
            return imported
        },

        /** The Import button and ⌘O: picks files, then imports them. */
        async pickAndImport(): Promise<void> {
            const selected = await open({
                multiple: true,
                filters: [{ name: t('library.importFilterName'), extensions: [...IMPORTABLE_EXTENSIONS] }],
            })
            if (!selected) { return }
            await this.importPaths(Array.isArray(selected) ? selected : [selected])
        },

        /**
         * One event of the window's drag and drop: shows or hides the drop
         * overlay, and imports what was dropped. Dropping nothing importable
         * (a folder, a .txt) says so instead of silently doing nothing.
         */
        onFileDrop(event: FileDropEvent, now = Date.now()): Promise<Book[]> | null {
            if (event.type === 'enter' || event.type === 'over') {
                this.dragging = true
                return null
            }
            this.dragging = false
            if (event.type !== 'drop') { return null }

            const paths = importablePaths(event.paths)
            if (paths.length === 0) {
                if (event.paths.length > 0) { useNotificationsStore().push('info', t('collection.nothingToImport')) }
                return null
            }
            const key = JSON.stringify(paths)
            if (this.lastDrop && this.lastDrop.key === key && now - this.lastDrop.at < DROP_REPEAT_WINDOW_MS) {
                return null
            }
            this.lastDrop = { key, at: now }
            return this.importPaths(paths)
        },

        dismissConversionOffer() {
            this.conversionOffer = []
        },
    },
})
