import { defineStore } from 'pinia'
import { save as saveDialog } from '@tauri-apps/plugin-dialog'
import * as api from '@/services/api'
import { useLibraryStore } from '@/stores/library'
import { useDeviceStore } from '@/stores/device'
import { useNotificationsStore } from '@/stores/notifications'
import { tidyupBook } from '@/services/tidyup'
import { batchMessages, emptyCounts, tidyupBatch } from '@/services/tidyup/batch'
import { stripUndefined } from '@/services/tidyup/merge'
import { analytics } from '@/services/analytics'
import { bucketCount } from '@/services/privacy'
import { reportError } from '@/services/librarium-api'
import { flyToDevice } from '@/utils/flyAnimation'
import { errorMessage } from '@/utils/errors'
import { displayTitle } from '@/utils/bookDisplay'
import { t } from '@/i18n'
import { deviceLabel } from '@/utils/device'
import { fileOf, type ShelfItem } from '@/services/shelf'
import type { Book, BookMetadata, CoverChange, DeleteResult, DeviceBook, ExportFormat, ForgetResult } from '@/types'

/** Formats `export_book` reads, and those it writes (docs/IPC.md). */
const EXPORTABLE_FROM: readonly string[] = ['epub', 'mobi', 'azw', 'azw3']
const EXPORT_FORMATS: readonly ExportFormat[] = ['epub', 'azw3']

/**
 * What `export_book` can write `book` as: EPUB or AZW3, from an EPUB, MOBI,
 * AZW or AZW3 book (boko can't read PDF or CBZ, and doesn't write PDF), its
 * own format excluded. Empty: the book can't be exported.
 */
export function exportFormatsOf(book: Book): ExportFormat[] {
    const current = book.format.toLowerCase()
    if (!EXPORTABLE_FROM.includes(current)) { return [] }
    return EXPORT_FORMATS.filter(f => f !== current)
}

/**
 * Where a delete or "remove from library" came from: a selection is cut
 * down to the books on screen. (The Duplicates review confirms and removes
 * copies itself, through the library store.)
 */
export type RemovalOrigin = 'selection' | 'single'

/** A delete or "remove from library" waiting for the user to confirm. */
export interface PendingRemoval {
    ids: string[]
    origin: RemovalOrigin
}

/** A single-book tidy-up result waiting for review. */
export interface TidyupReview {
    book: Book
    metadata: Partial<BookMetadata>
    source: string | null
    coverUrl: string | null
    confident: boolean
}

interface BookActionsState {
    /** Books open in the metadata editor (none: closed). */
    editing: Book[]
    pendingDelete: PendingRemoval | null
    deleting: boolean
    deleteError: string
    pendingForget: PendingRemoval | null
    forgetting: boolean
    forgetError: string
    /** Books waiting for "Send anyway" (some are already on the device). */
    pendingSend: Book[]
    /** The batch tidy-up confirmation is open. */
    confirmingBatchTidyup: boolean
    /** A tidy-up (one book or a batch) is running. */
    tidying: boolean
    review: TidyupReview | null
    /** The duplicates review is open. */
    duplicatesOpen: boolean
    /** Files waiting for "Remove from the e-reader" to be confirmed (null: no dialog). */
    pendingDeviceRemoval: DeviceBook[] | null
    removingFromDevice: boolean
    deviceRemovalError: string
    /** Paths of the device files being copied into the library. */
    addingToLibrary: string[]
}

/**
 * What can be done to books from the library screen, wherever the user
 * asked: the detail panel, the context menu, the selection toolbar, a list
 * row or a shortcut. Each of them calls an action here instead of passing
 * events up to the view.
 *
 * The state is the dialogs those actions open (rendered by LibraryDialogs):
 * every destructive action is confirmed first (`pending…`), and every
 * failure is shown, inline in the dialog that asked (`…Error`) or as a
 * toast.
 *
 * Safety rules kept from the library view:
 * - deletes go to the Trash, always after confirmation;
 * - an action on "the selection" only reaches books currently visible
 *   (`actionableIds`): ⌘A, a search, then Delete never deletes hidden books;
 * - a book already on the device is only sent again after "Send anyway";
 * - removing from the e-reader (for good: it has no Trash) is its own
 *   action, always confirmed, never mixed with "Move to Trash".
 */
export const useBookActionsStore = defineStore('bookActions', {
    state: (): BookActionsState => ({
        editing: [],
        pendingDelete: null,
        deleting: false,
        deleteError: '',
        pendingForget: null,
        forgetting: false,
        forgetError: '',
        pendingSend: [],
        confirmingBatchTidyup: false,
        tidying: false,
        review: null,
        duplicatesOpen: false,
        pendingDeviceRemoval: null,
        removingFromDevice: false,
        deviceRemovalError: '',
        addingToLibrary: [],
    }),

    getters: {
        /** "Move “Dune” to the Trash?" or "Move 3 books to the Trash?". */
        deleteTitle: (state): string => removalTitle(state.pendingDelete, 'trash'),
        forgetTitle: (state): string => removalTitle(state.pendingForget, 'forget'),
        /** How many of the books waiting for "Send anyway" are on the device already. */
        pendingSendOnDevice: (state): number => {
            const device = useDeviceStore()
            return state.pendingSend.filter(b => device.isOnDevice(b)).length
        },
        /** "Remove “Dune” from Kindle?" or "Remove 3 books from Kindle?". */
        deviceRemovalTitle: (state): string => {
            const files = state.pendingDeviceRemoval ?? []
            const device = deviceLabel(useDeviceStore().device)
            if (files.length === 1) {
                return t('shelf.removeConfirmTitleOne', { title: files[0].title || files[0].filename, device })
            }
            return t('shelf.removeConfirmTitle', { n: files.length, device })
        },
    },

    actions: {
        /**
         * Closes every dialog (another library was opened, the view went
         * away). Work in progress (a delete, a batch tidy-up) carries on.
         */
        reset() {
            this.editing = []
            if (!this.deleting) { this.pendingDelete = null }
            if (!this.forgetting) { this.pendingForget = null }
            this.pendingSend = []
            this.confirmingBatchTidyup = false
            this.review = null
            this.duplicatesOpen = false
            if (!this.removingFromDevice) { this.pendingDeviceRemoval = null }
        },

        // ── Metadata ─────────────────────────────────────────────────────────

        edit(books: readonly Book[]) {
            if (books.length > 0) { this.editing = [...books] }
        },

        closeEditor() {
            this.editing = []
        },

        /**
         * The metadata editor's submit handler. Rejects with a readable
         * message when anything failed, so the dialog stays open and shows it.
         */
        async saveMetadata(
            bookIds: string[],
            metadata: Partial<BookMetadata>,
            cover: CoverChange,
            seriesIndexes?: Readonly<Record<string, number>>,
        ): Promise<void> {
            const library = useLibraryStore()
            if (bookIds.length !== 1) {
                // Batch: only the fields the user filled in are sent, and
                // the backend merges them into each book's own metadata;
                // `seriesIndexes` numbers the volumes of a series.
                const { failures } = await library.batchUpdateMetadata(bookIds, metadata, seriesIndexes)
                if (failures.length > 0) {
                    throw new Error(t('metadata.batchPartialFailure', {
                        n: failures.length, total: bookIds.length, error: failures[0].error,
                    }))
                }
                useNotificationsStore().push('success', t('metadata.batchSaved', { n: bookIds.length }))
                return
            }
            const bookId = bookIds[0]
            await library.updateMetadata(bookId, metadata)
            try {
                if (cover.action === 'replace' && cover.base64) {
                    await library.setBookCover(bookId, cover.base64)
                } else if (cover.action === 'delete') {
                    await library.deleteBookCover(bookId)
                }
            } catch (e) {
                throw new Error(t('metadata.coverFailed', { error: errorMessage(e) }))
            }
        },

        // ── Device ───────────────────────────────────────────────────────────

        /** Sends books to the selected device, asking first if some are there already. */
        send(books: readonly Book[]) {
            if (books.length === 0) { return }
            books.forEach(b => flyToDevice(b.id))
            const device = useDeviceStore()
            if (books.some(b => device.isOnDevice(b))) {
                this.pendingSend = [...books]
            } else {
                void device.sendBooks([...books])
            }
        },

        confirmSend() {
            const books = this.pendingSend
            this.pendingSend = []
            if (books.length > 0) { void useDeviceStore().sendBooks(books) }
        },

        cancelSend() {
            this.pendingSend = []
        },

        // ── Between the library and the e-reader ─────────────────────────────

        /**
         * Copies files that are only on the e-reader into the library, one
         * at a time (each is a job in Activity; a failure is shown there and
         * as a toast). Once in, a file matches its new library book: it
         * shows as one item, in both places. Library books among `items`
         * are skipped.
         */
        async addToLibrary(items: readonly ShelfItem[]) {
            const library = useLibraryStore()
            const device = useDeviceStore()
            const libraryPath = library.libraryPath
            if (!libraryPath) { return }
            // Read once: if the user switches library meanwhile, the books
            // still land in the one they were meant for, and aren't shown
            // in the new one.
            const session = library.session
            const files = items.map(fileOf).filter((f): f is DeviceBook => f !== null && !this.addingToLibrary.includes(f.path))
            for (const file of files) {
                this.addingToLibrary.push(file.path)
                try {
                    const imported = await device.importFromDevice(file, libraryPath)
                    if (imported && session === library.session) { library.addBooks([imported]) }
                } finally {
                    this.addingToLibrary = this.addingToLibrary.filter(p => p !== file.path)
                }
            }
        },

        /**
         * Asks to remove the items' files from the e-reader: a file only on
         * the device, or every copy of a library book (the library book is
         * kept). Nothing on the device: nothing to ask.
         */
        requestRemoveFromDevice(items: readonly ShelfItem[]) {
            const files = new Map<string, DeviceBook>()
            for (const item of items) {
                for (const copy of item.copies) { files.set(copy.path, copy) }
            }
            if (files.size === 0) { return }
            this.deviceRemovalError = ''
            this.pendingDeviceRemoval = [...files.values()]
        },

        cancelRemoveFromDevice() {
            if (!this.removingFromDevice) { this.pendingDeviceRemoval = null }
        },

        /**
         * Deletes the files from the e-reader (for good), one after the
         * other. On a failure the dialog stays open, with the reason, listing
         * only the files still there.
         */
        async confirmRemoveFromDevice() {
            const files = this.pendingDeviceRemoval
            if (!files || this.removingFromDevice) { return }
            const device = useDeviceStore()
            this.removingFromDevice = true
            this.deviceRemovalError = ''
            const left: DeviceBook[] = []
            let failure = ''
            try {
                for (const file of files) {
                    try {
                        await device.deleteDeviceBook(file.path)
                    } catch (e) {
                        left.push(file)
                        failure ||= errorMessage(e)
                    }
                }
            } finally {
                this.removingFromDevice = false
            }
            const removed = files.length - left.length
            const name = deviceLabel(device.device)
            if (removed > 0) {
                useNotificationsStore().push('success', removed === 1 && files.length === 1
                    ? t('shelf.removedOne', { title: files[0].title || files[0].filename, device: name })
                    : t('shelf.removed', { n: removed, device: name }))
            }
            if (left.length === 0) {
                this.pendingDeviceRemoval = null
            } else {
                this.pendingDeviceRemoval = left
                this.deviceRemovalError = t('errors.deviceDeleteFailed', { error: failure })
            }
        },

        // ── Delete (always confirmed, always to the Trash) ───────────────────

        /** The books of `ids` that may be acted on: still listed, and visible for a selection. */
        actionableIds(ids: readonly string[], origin: RemovalOrigin): string[] {
            const library = useLibraryStore()
            let candidates = ids.filter(id => library.bookById.has(id))
            if (origin === 'selection') {
                const visible = new Set(library.filteredBooks.map(b => b.id))
                candidates = candidates.filter(id => visible.has(id))
            }
            return candidates
        },

        requestDelete(ids: readonly string[], origin: RemovalOrigin) {
            const candidates = this.actionableIds(ids, origin)
            if (candidates.length === 0) { return }
            this.deleteError = ''
            this.pendingDelete = { ids: candidates, origin }
        },

        cancelDelete() {
            if (!this.deleting) { this.pendingDelete = null }
        },

        async confirmDelete() {
            if (!this.pendingDelete || this.deleting) { return }
            const { ids } = this.pendingDelete
            this.deleting = true
            this.deleteError = ''
            try {
                const result = await useLibraryStore().deleteBooks(ids)
                this.pendingDelete = null
                reportDeleteResult(result)
            } catch (e) {
                // Nothing was deleted: keep the dialog open with the reason.
                reportError('DELETE_FAILED', 'library.trash', e)
                this.deleteError = t('errors.deleteFailed', { error: errorMessage(e) })
            } finally {
                this.deleting = false
            }
        },

        // ── Remove from library (files kept; always confirmed) ───────────────

        requestForget(ids: readonly string[], origin: RemovalOrigin) {
            const candidates = this.actionableIds(ids, origin)
            if (candidates.length === 0) { return }
            this.forgetError = ''
            this.pendingForget = { ids: candidates, origin }
        },

        cancelForget() {
            if (!this.forgetting) { this.pendingForget = null }
        },

        async confirmForget() {
            if (!this.pendingForget || this.forgetting) { return }
            const { ids } = this.pendingForget
            this.forgetting = true
            this.forgetError = ''
            try {
                const result = await useLibraryStore().forgetBooks(ids)
                this.pendingForget = null
                reportForgetResult(result)
            } catch (e) {
                reportError('FORGET_FAILED', 'library.forget', e)
                this.forgetError = t('errors.forgetFailed', { error: errorMessage(e) })
            } finally {
                this.forgetting = false
            }
        },

        // ── Other book actions ───────────────────────────────────────────────

        /**
         * The star of a book, or of a selection: stars them all, or unstars
         * them when they all are already. A failure is a toast (the stars
         * are put back).
         */
        async toggleFavorite(books: readonly Book[]) {
            if (books.length === 0) { return }
            const favorite = !books.every(b => b.favorite)
            try {
                await useLibraryStore().setFavorite(books.map(b => b.id), favorite)
            } catch (e) {
                useNotificationsStore().push('error', t('errors.favoriteFailed', { error: errorMessage(e) }))
            }
        },

        async resetData(book: Book) {
            try {
                await useLibraryStore().resetBookData(book.id)
                useNotificationsStore().push('success', t('detail.resetDone', { title: displayTitle(book, t) }))
            } catch (e) {
                console.error('[reset-book-data] failed', e)
                useNotificationsStore().push('error', t('errors.resetFailed', { error: errorMessage(e) }))
            }
        },

        /**
         * Exports a copy of `book` as `format`, where the user chooses (save
         * dialog). Without `format` (the menu's "Export…"), every format the
         * book can be exported as is offered in the dialog, and the file
         * name's extension picks one. The outcome is a toast.
         */
        async exportBook(book: Book, format?: ExportFormat) {
            const notifications = useNotificationsStore()
            const libraryPath = useLibraryStore().libraryPath
            const formats = format ? [format] : exportFormatsOf(book)
            if (!libraryPath) { return }
            if (formats.length === 0) {
                notifications.push('info', t('shortcuts.exportUnsupported'))
                return
            }
            const title = book.metadata.title || book.filename.replace(/\.[^.]+$/, '')
            const dest = await saveDialog({
                filters: formats.map(f => ({ name: f.toUpperCase(), extensions: [f] })),
                defaultPath: `${title}.${formats[0]}`,
            })
            if (!dest) { return }
            const extension = dest.split('.').pop()?.toLowerCase()
            const chosen = formats.find(f => f === extension) ?? formats[0]
            try {
                // Id-based: the backend finds the file in its index.
                await api.exportBook(libraryPath, book.id, chosen, dest)
                notifications.push('success', t('detail.exportSuccess', { format: chosen.toUpperCase() }))
            } catch (e) {
                notifications.push('error', t('detail.exportFailed', { error: errorMessage(e) }))
            }
        },

        openDuplicates() {
            this.duplicatesOpen = true
        },

        closeDuplicates() {
            this.duplicatesOpen = false
        },

        // ── Tidy-up ──────────────────────────────────────────────────────────

        /** Looks one book up online and opens the review of what was found. */
        async tidyupOne(book: Book) {
            if (this.tidying) { return }
            this.tidying = true
            const notifications = useNotificationsStore()
            try {
                const result = await tidyupBook({
                    title: book.metadata.title,
                    author: book.metadata.author,
                    isbn: book.metadata.isbn,
                })
                if (result.found) {
                    const metadata = { ...result.metadata }
                    // A language the book already has is kept.
                    if (book.metadata.language && book.metadata.language !== 'und') { delete metadata.language }
                    this.review = {
                        book,
                        metadata,
                        source: result.source,
                        coverUrl: result.coverUrl,
                        confident: result.confident,
                    }
                } else if (result.outcome === 'unreachable') {
                    notifications.push('error', t('tidyup.unreachable'))
                } else {
                    // Said explicitly: a button that spins and then does nothing
                    // reads as a broken feature, not as "no match".
                    notifications.push('info', t('tidyup.noMatch', { title: displayTitle(book, t) }))
                }
            } catch (e) {
                reportError('TIDY_UP_FAILED', 'tidyUp.book', e)
                notifications.push('error', t('tidyup.failed', { title: displayTitle(book, t) }))
            } finally {
                this.tidying = false
            }
        },

        closeReview() {
            this.review = null
        },

        /**
         * Applies what the user ticked in the review of one book (the
         * dialog's `submit`, see TidyupSubmit). A failed metadata save
         * rejects with the backend's error: the dialog stays open and shows
         * it, so the user can retry or cancel. A cover that can't be
         * downloaded doesn't undo the metadata: it is a warning toast, and
         * the review closes.
         */
        async applyReview(metadata: Partial<BookMetadata>, coverUrl: string | null): Promise<void> {
            const review = this.review
            if (!review) { return }
            const library = useLibraryStore()
            // Undefined keys would clear fields: only what was chosen is applied.
            const changes = stripUndefined(metadata)
            const appliedFields = Object.keys(changes)
            if (appliedFields.length > 0) { await library.updateMetadata(review.book.id, changes) }
            analytics.track('tidy_up_applied', {
                service:     review.source ?? 'unknown',
                field_count_bucket: bucketCount(appliedFields.length),
                fields:      appliedFields.join(','),
            })
            if (coverUrl) {
                try {
                    await library.saveCoverFromUrl(review.book.id, coverUrl)
                } catch (e) {
                    console.error('[tidy-up] cover download failed', e)
                    useNotificationsStore().push('warning', t('tidyup.coverFailed'))
                }
            }
            // Another review may have been opened meanwhile: only close this one.
            if (this.review === review) { this.review = null }
        },

        /** Asks before tidying the selection (applied without review). */
        askBatchTidyup() {
            if (useLibraryStore().selectedBooks.length > 0) { this.confirmingBatchTidyup = true }
        },

        cancelBatchTidyup() {
            this.confirmingBatchTidyup = false
        },

        /** Tidies the selected books (see services/tidyup/batch.ts for the rules). */
        async runBatchTidyup() {
            this.confirmingBatchTidyup = false
            const library = useLibraryStore()
            const notifications = useNotificationsStore()
            // A snapshot: saving below changes the store, and the summary
            // needs a stable total.
            const books = [...library.selectedBooks]
            if (books.length === 0 || this.tidying) { return }
            this.tidying = true
            const counts = emptyCounts()
            try {
                await tidyupBatch(books, {
                    lookup: tidyupBook,
                    current: id => library.bookById.get(id),
                    // Covers load lazily: a book never shown may have no cover
                    // in the store yet and still have one, so ask.
                    hasCover: async id => library.coverOf(id) !== null || (await library.ensureCover(id)) !== null,
                    saveMetadata: (id, patch) => library.updateMetadata(id, patch),
                    saveCover: (id, url) => library.saveCoverFromUrl(id, url),
                    log: (message, error) => console.error(message, error),
                }, counts)
                for (const m of batchMessages(counts, books.length)) {
                    notifications.push(m.type, m.message, m.duration)
                }
            } catch (e) {
                reportError('TIDY_UP_BATCH_FAILED', 'tidyUp.batch', e)
                notifications.push('error', t('tidyup.batchFailed', { n: books.length, done: counts.updated }))
            } finally {
                this.tidying = false
            }
        },
    },
})

function removalTitle(pending: PendingRemoval | null, kind: 'trash' | 'forget'): string {
    const ids = pending?.ids ?? []
    if (ids.length === 1) {
        const book = useLibraryStore().bookById.get(ids[0])
        if (book) {
            const title = book.metadata.title || book.filename
            return kind === 'trash'
                ? t('library.trashConfirmTitleOne', { title })
                : t('library.forgetConfirmTitleOne', { title })
        }
    }
    return kind === 'trash'
        ? t('library.trashConfirmTitle', { n: ids.length })
        : t('library.forgetConfirmTitle', { n: ids.length })
}

/** One toast for the batch, plus one for failures, rather than one per book. */
function reportDeleteResult(result: DeleteResult) {
    const notifications = useNotificationsStore()
    if (result.deleted.length > 0) {
        let message = t('library.trashDone', { n: result.deleted.length })
        if (result.libraryTrash) {
            message += ' ' + t('library.trashFallback', { path: result.libraryTrash })
        }
        // Longer when it names a folder the user may want to note.
        notifications.push('success', message, result.libraryTrash ? 15000 : undefined)
    }
    if (result.failures.length > 0) {
        notifications.push('error', t('library.trashFailures', {
            n: result.failures.length, error: result.failures[0].error,
        }))
    }
}

function reportForgetResult(result: ForgetResult) {
    const notifications = useNotificationsStore()
    if (result.forgotten.length > 0) {
        notifications.push('success', t('library.forgetDone', { n: result.forgotten.length }))
    }
    if (result.failures.length > 0) {
        notifications.push('error', t('library.forgetFailures', {
            n: result.failures.length, error: result.failures[0].error,
        }))
    }
}
