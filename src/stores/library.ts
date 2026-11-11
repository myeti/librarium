import { defineStore } from 'pinia'
import { markRaw } from 'vue'
import * as api from '@/services/api'
import { analytics, Analytics } from '@/services/analytics'
import { findDuplicateGroups, type DuplicateGroup } from '@/services/duplicates'
import { useNotificationsStore } from '@/stores/notifications'
import { useOpdsStore } from '@/stores/opds'
import { t } from '@/i18n'
import { errorMessage, isLibraryUnavailable, isPermissionDenied } from '@/utils/errors'
import { sortedText } from '@/utils/collator'
import { coverUrl } from '@/utils/covers'
import { fmtTime } from '@/utils/format'
import { TaskQueue } from '@/utils/taskQueue'
import { buildShelf, visibleShelf, type ShelfItem, type ShelfScope } from '@/services/shelf'
// Imported by the device store too: each only reads the other inside its
// getters and actions, so the cycle is harmless.
import { useDeviceStore } from '@/stores/device'
import type {
    Book, BookFormat, BookMetadata, BookMetadataPatch, SortConfig, FilterConfig,
    BatchUpdateResult, DeleteResult, CoverInfo, ForgetResult,
} from '@/types'

export { foldForSearch } from '@/services/shelf'

/**
 * The library of the current user: its books, their covers, the selection,
 * filters and sort, and every action that changes books on disk.
 *
 * Ownership: the users store opens the library (`openLibrary`) whenever the
 * current user or the library root changes, whatever view is on screen.
 * Views only render this state and call actions; they never load it.
 *
 * Stale results: opening a library starts a new *session*. Every async
 * action remembers the session it started in and drops its result if the
 * library changed meanwhile. Without this, a slow scan of user A's NAS
 * folder finishing after the switch to user B would show A's books under B's
 * path, and later imports and deletes would target the wrong folder. Scans
 * also carry a sequence number so an older scan can't overwrite a newer one.
 *
 * Errors: mutating actions reject with the backend's `{ code, message }`
 * error (read it with `errorMessage` / `errorCode` from `@/utils/errors`) so
 * the caller can show it where the user acted (inline in a dialog, or a
 * toast). Background work (scans) reports failures itself.
 *
 * Unreachable library: when the library folder doesn't exist (NAS share not
 * mounted, drive unplugged) the backend refuses with the error code
 * `LIBRARY_UNAVAILABLE` instead of creating an empty folder; `unavailable`
 * is then set and the library view offers Retry. A folder that is there but
 * may not be read (`PERMISSION_DENIED`: on macOS, Librarium not yet allowed
 * to read network volumes) sets `permissionDenied` instead: the view shows
 * the backend's message, which says where to allow it, with Retry.
 *
 * Another instance: when a library is opened, the backend is asked whether
 * another Librarium (on this computer or another one sharing the NAS) used
 * it in the last few minutes; if so, a warning toast says so.
 *
 * ## How books are held (performance)
 *
 * A library can hold thousands of books, and the list view filters and sorts
 * all of them on every search keystroke. So:
 *
 * - **Books are frozen snapshots** (`freezeBook`). Vue never makes a frozen
 *   object reactive, so filtering and sorting read plain objects instead of
 *   going through a reactive proxy for every property, and nothing can
 *   mutate a book behind the store's back (that throws). A changed book is
 *   *replaced* in `books`; only the array is reactive.
 * - **Covers live apart from books**, in `covers` (book id → cover file).
 *   Covers arrive one by one while the user scrolls; keeping them out of
 *   `books` means an arriving cover re-renders only the card that reads it
 *   (`coverOf(id)` tracks that one key) and never re-runs the filtering,
 *   sorting or duplicate detection of the whole library. (Replacing the book
 *   object instead would have re-run them once per cover: N covers, N full
 *   re-sorts.) `scan_library` returns books with their cover fields; they
 *   are moved into `covers` on the way in, so a book in `books` never
 *   carries a cover.
 * - **The selection** is a reactive `Set` of ids: `selectedIds.has(id)`
 *   tracks one key, and selecting never touches `books`.
 * - `bookById` is an id → book map, rebuilt only when `books` changes, so
 *   lookups by id don't scan the list.
 *
 * Covers are asked for lazily (`requestCover`, called by cards that become
 * visible): `ensure_cover` runs at most COVER_CONCURRENCY at a time, and
 * results that arrive after another library was opened are dropped. Covers
 * are file paths shown through the asset protocol (`coverUrl`), never data
 * URLs held in this state.
 */

/** How many `ensure_cover` calls may run at once (each may open a book on a NAS). */
export const COVER_CONCURRENCY = 4

/**
 * Cover requests are queued per session and book: a request of the previous
 * library still running must not stand in for the same id in the new one
 * (reopening a library, switching back to a user).
 */
function coverKey(session: number, bookId: string): string {
    return `${session}:${bookId}`
}

type ScanTrigger = 'startup' | 'manual' | 'import'
export type ViewMode = 'grid' | 'list'

export function emptyFilters(): FilterConfig {
    return { search: '', formats: [], authors: [], series: [], languages: [], categories: [], presence: [] }
}

/** Formats that `convert_to_epub` accepts. */
export const CONVERTIBLE_FORMATS: readonly string[] = ['mobi', 'azw', 'azw3']

/**
 * The book as the store keeps it: without its cover fields (those go to
 * `covers`), and frozen, with its metadata and categories, so Vue leaves it
 * non-reactive and any attempt to modify it in place fails loudly.
 */
export function freezeBook(book: Book): Book {
    const { coverPath: _coverPath, coverVersion: _coverVersion, ...rest } = book
    const categories = rest.metadata.categories
    const metadata = Object.freeze({
        ...rest.metadata,
        ...(categories ? { categories: Object.freeze([...categories]) as string[] } : {}),
    })
    return Object.freeze({ ...rest, metadata })
}

/** The cover the backend sent with a book, if any. */
function coverInfoOf(book: Book): CoverInfo | null {
    return book.coverPath ? { coverPath: book.coverPath, coverVersion: book.coverVersion ?? 0 } : null
}

/**
 * Builds the full metadata object `update_metadata` expects. That command
 * *replaces* the metadata: a key that is absent (or `undefined`, which JSON
 * drops) clears the field. `title`, `author` and `categories` are required
 * by the backend, so they get empty defaults rather than being omitted.
 */
function toFullMetadata(m: Partial<BookMetadata>): BookMetadata {
    const out: Record<string, unknown> = {}
    for (const [k, v] of Object.entries(m)) {
        if (v !== undefined) { out[k] = v }
    }
    return {
        ...(out as Partial<BookMetadata>),
        title:      m.title ?? '',
        author:     m.author ?? '',
        categories: m.categories ? [...m.categories] : [],
    }
}

function bucketBookCount(n: number): string {
    return Analytics.bucketBookCount(n)
}

/** The view chosen last time; the grid of covers until one is chosen. */
function storedViewMode(): ViewMode {
    return localStorage.getItem('viewMode') === 'list' ? 'list' : 'grid'
}

interface LibraryState {
    /** Frozen snapshots (`freezeBook`), in library order. Replaced, never mutated. */
    books: Book[]
    /** Cover files by book id: only books whose cover is known have an entry. */
    covers: Record<string, CoverInfo>
    loading: boolean
    /** Why the library couldn't be opened; shown by LibraryView with a Retry button. */
    error: string | null
    /** The library folder doesn't exist (share not mounted): LibraryView says so, with Retry. */
    unavailable: boolean
    /** The library folder is there but may not be read (macOS privacy settings): LibraryView shows `error`, with Retry. */
    permissionDenied: boolean
    /**
     * The selected items of the library screen: library book ids, and
     * `device:…` ids for files only on the e-reader (services/shelf.ts).
     */
    selectedIds: Set<string>
    /** The quick switch above the books: everything, the library's, the device's. */
    scope: ShelfScope
    libraryPath: string | null
    viewMode: ViewMode
    sort: SortConfig
    filters: FilterConfig
    /** Bumped by `openLibrary`: identifies which library results belong to. */
    session: number
    /** Bumped by every scan: only the latest scan may write `books`. */
    scanSeq: number
    /** Pending `ensure_cover` calls, by book id (not reactive: bookkeeping only). */
    coverQueue: TaskQueue
    /** Books already asked about in this session (answered, or failed): not asked again. */
    coverChecked: Set<string>
    /** Books whose cover file the webview failed to load this session (see `coverFailedToLoad`). */
    coverBroken: Set<string>
}

export const useLibraryStore = defineStore('library', {
    state: (): LibraryState => ({
        books:        [],
        covers:       {},
        loading:      false,
        error:        null,
        unavailable:  false,
        permissionDenied: false,
        selectedIds:  new Set(),
        scope:        'all',
        libraryPath:  null,
        viewMode:     storedViewMode(),
        sort:         { field: 'title', order: 'asc' },
        filters:      emptyFilters(),
        session:      0,
        scanSeq:      0,
        coverQueue:   markRaw(new TaskQueue(COVER_CONCURRENCY)),
        coverChecked: markRaw(new Set()),
        coverBroken:  markRaw(new Set()),
    }),

    getters: {
        /** Books by id; rebuilt only when `books` changes. */
        bookById: (state): Map<string, Book> => new Map(state.books.map(b => [b.id, b])),

        /** The cover of a book, or null while unknown (or when it has none). */
        coverOf: (state) => (bookId: string): CoverInfo | null => state.covers[bookId] ?? null,

        /** The URL to show a book's cover, or null. */
        coverUrlOf: (state) => (bookId: string): string | null => {
            const cover = state.covers[bookId]
            return cover ? coverUrl(cover) : null
        },

        allAuthors: (state): string[] =>
            sortedText(new Set(state.books.map(b => b.metadata.author).filter(Boolean))),

        allSeries: (state): string[] =>
            sortedText(new Set(state.books.map(b => b.metadata.series).filter((s): s is string => Boolean(s)))),

        allLanguages: (state): string[] =>
            sortedText(new Set(state.books.map(b => b.metadata.language).filter((l): l is string => Boolean(l)))),

        allCategories: (state): string[] =>
            sortedText(new Set(state.books.flatMap(b => b.metadata.categories ?? []))),

        /**
         * The library's books and, while an e-reader is connected, the
         * books on it, merged (services/shelf.ts): a library book on the
         * device is one item with its copies; a file no library book matches
         * is an item of its own. Rebuilt when either side changes.
         */
        shelf(state): ShelfItem[] {
            const device = useDeviceStore()
            if (!device.isConnected) { return buildShelf(state.books, [], {}) }
            return buildShelf(state.books, device.deviceBooks, device.readingProgress)
        },

        /** Shelf items by id. */
        shelfById(): Map<string, ShelfItem> {
            return new Map(this.shelf.map(item => [item.id, item]))
        },

        /** The scope in force: "the device's books" means nothing without a device. */
        effectiveScope(state): ShelfScope {
            return useDeviceStore().isConnected ? state.scope : 'all'
        },

        /**
         * The items to show: in the scope, filtered by the search (title,
         * author or series; a device file's name; case and accents ignored)
         * and the filters, then sorted (unknown values last, volumes of a
         * series in order: see `visibleShelf`).
         */
        visibleItems(state): ShelfItem[] {
            return visibleShelf(this.shelf, this.effectiveScope, state.filters, state.sort)
        },

        /** The library books shown, in display order (what library actions on "visible" books may reach). */
        filteredBooks(): Book[] {
            return this.visibleItems.flatMap(item => item.book ? [item.book] : [])
        },

        /** The selected items, books and device files, in shelf order. */
        selectedItems(state): ShelfItem[] {
            return this.shelf.filter(item => state.selectedIds.has(item.id))
        },

        /** The selected books, in library order. */
        selectedBooks: (state): Book[] => state.books.filter(b => state.selectedIds.has(b.id)),

        /** Groups of probable duplicates (see services/duplicates.ts). */
        duplicateGroups: (state): DuplicateGroup[] => findDuplicateGroups(state.books),
    },

    actions: {
        // ── Opening and scanning ─────────────────────────────────────────────

        /**
         * Opens the library at `path` (or closes it with `null`). Called by the
         * users store when the user or the library root changes, and when the
         * library folder is changed in Settings.
         *
         * Everything that belonged to the previous library is reset first:
         * books, covers, selection, filters and search. Results of any work
         * still running for the previous library are discarded when they arrive.
         */
        async openLibrary(path: string | null, triggeredBy: ScanTrigger = 'startup') {
            this.session++
            this.libraryPath = path
            this.books = []
            this.covers = {}
            this.selectedIds = new Set()
            this.filters = emptyFilters()
            this.error = null
            this.unavailable = false
            this.permissionDenied = false
            this.loading = false
            // Covers of the previous library: queued requests are dropped
            // (running ones finish, and their results are discarded).
            this.coverQueue.clear()
            this.coverChecked = markRaw(new Set())
            this.coverBroken = markRaw(new Set())
            if (!path) { return }

            const session = this.session
            const scanned = await this.scan(triggeredBy)
            if (scanned && session === this.session) {
                await this.checkOtherInstance()
            }
        },

        /**
         * Warns (once per opening) when another Librarium used this library in
         * the last few minutes: edits made from both at the same time can
         * overwrite each other. Advisory only: any failure is ignored.
         */
        async checkOtherInstance() {
            const libraryPath = this.libraryPath
            if (!libraryPath) { return }
            const session = this.session
            try {
                const status = await api.libraryLockStatus(libraryPath)
                const other = status?.otherInstance
                if (!other || session !== this.session) { return }
                const time = fmtTime(other.updatedAt)
                useNotificationsStore().push('warning', t('library.lockedByOther', { host: other.host, time }), 15000)
            } catch {
                // Nothing can be detected (read-only share…): no warning.
            }
        },

        /**
         * Rescans the open library (Settings › Library › Rescan, after a Calibre
         * import, the Retry of a failed scan).
         * Unlike `openLibrary` it keeps the filters, and only prunes the
         * selection of books that disappeared.
         */
        async refresh(triggeredBy: ScanTrigger = 'manual') {
            if (!this.libraryPath) { return }
            const session = this.session
            const scanned = await this.scan(triggeredBy)
            if (scanned && session === this.session) {
                this.pruneSelection()
            }
        },

        /**
         * Runs `scan_library` for the open library. Returns true when the result
         * was applied, false when it failed or was superseded (another scan
         * started, or another library was opened meanwhile).
         */
        async scan(triggeredBy: ScanTrigger): Promise<boolean> {
            const path = this.libraryPath
            if (!path) { return false }
            const session = this.session
            const seq = ++this.scanSeq
            const isLatest = () => session === this.session && seq === this.scanSeq

            this.loading = true
            this.error = null
            this.unavailable = false
            this.permissionDenied = false
            const start = Date.now()
            try {
                const books = await api.scanLibrary(path)
                if (!isLatest()) { return false }
                // Covers the scan knows replace ours; covers loaded this
                // session that it doesn't know (a full-size cover used
                // because no thumbnail could be made) are kept, for the
                // books that are still there.
                const covers: Record<string, CoverInfo> = {}
                for (const book of books) {
                    const cover = coverInfoOf(book) ?? this.covers[book.id]
                    if (cover) { covers[book.id] = { coverPath: cover.coverPath, coverVersion: cover.coverVersion } }
                }
                this.covers = covers
                this.books = books.map(freezeBook)
                analytics.track('library_scan', {
                    book_count_bucket: bucketBookCount(books.length),
                    duration_bucket:   Analytics.bucketDurationMs(Date.now() - start),
                    triggered_by:      triggeredBy,
                })
                return true
            } catch (e) {
                if (!isLatest()) { return false }
                this.error = errorMessage(e)
                this.unavailable = isLibraryUnavailable(e)
                this.permissionDenied = isPermissionDenied(e)
                if (this.unavailable) {
                    useNotificationsStore().push('warning', `${t('library.unavailableTitle')}. ${t('library.unavailableBody', { path })}`)
                } else if (this.permissionDenied) {
                    useNotificationsStore().push('warning', this.error)
                } else {
                    useNotificationsStore().push('error', t('errors.libraryOpenFailed', { error: this.error }))
                }
                analytics.track('error', {
                    component:  'library_scan',
                    error_type: this.unavailable ? 'library_unavailable' : this.permissionDenied ? 'permission_denied' : 'scan_failed',
                })
                return false
            } finally {
                if (isLatest()) { this.loading = false }
            }
        },

        // ── Covers ───────────────────────────────────────────────────────────

        /**
         * Asks the backend for a book's cover, lazily: cards call this when
         * they become visible. Does nothing if the book has a cover already,
         * was already asked about this session, or is already queued (that
         * only adds interest, see `cancelCoverRequest`). Requests run
         * COVER_CONCURRENCY at a time; a result that arrives after another
         * library was opened is dropped.
         *
         * Returns true when the caller now holds interest in a queued
         * request, and should call `cancelCoverRequest` if it stops caring.
         */
        requestCover(bookId: string): boolean {
            const libraryPath = this.libraryPath
            if (!libraryPath || this.coverChecked.has(bookId) || this.covers[bookId]) { return false }
            const session = this.session
            return this.coverQueue.push(coverKey(session, bookId), async () => {
                if (session !== this.session) { return }
                let info: CoverInfo | null = null
                try {
                    info = await api.ensureCover(libraryPath, bookId)
                } catch {
                    // Unreadable book, unknown id…: it keeps its placeholder,
                    // and isn't asked about again this session.
                }
                if (session !== this.session) { return }
                this.coverChecked.add(bookId)
                if (info) { this.setCover(bookId, info) }
            })
        },

        /**
         * The card of `bookId` left the screen before its cover request
         * started: withdraw its interest so fast scrolling through thousands
         * of books doesn't queue them all. The request is dropped only if
         * nobody else (the detail panel) is waiting for it.
         */
        cancelCoverRequest(bookId: string) {
            this.coverQueue.cancel(coverKey(this.session, bookId))
        },

        /**
         * The webview couldn't load a book's cover file (removed from
         * `.covers/` behind our back, by another computer sharing the
         * library…): forget it and ask the backend once more, which makes the
         * thumbnail again or finds the cover anew. After a second failure in
         * the same session the card keeps its placeholder rather than a broken
         * image.
         */
        coverFailedToLoad(bookId: string) {
            const firstFailure = !this.coverBroken.has(bookId)
            this.coverBroken.add(bookId)
            if (firstFailure) {
                this.coverChecked.delete(bookId)
            } else {
                this.coverChecked.add(bookId)
            }
            // Cards watch their cover: clearing it makes a visible card ask again.
            this.clearCover(bookId)
        },

        /**
         * The cover of one book, right away (not queued), for decisions that
         * depend on it: batch tidy-up adds a cover only to books that have
         * none, and a book whose cover was never looked at has no entry in
         * `covers` yet. Rejects if the backend can't tell.
         */
        async ensureCover(bookId: string): Promise<CoverInfo | null> {
            const { libraryPath, session } = this.requireLibrary()
            const known = this.covers[bookId]
            if (known) { return known }
            const info = await api.ensureCover(libraryPath, bookId)
            if (session === this.session) {
                this.coverChecked.add(bookId)
                if (info) { this.setCover(bookId, info) }
            }
            return info
        },

        // ── Mutations (all reject with the backend's error) ─────────────────

        /** The open library, or a rejection when there is none (no user selected yet). */
        requireLibrary(): { libraryPath: string, session: number } {
            if (!this.libraryPath) { throw new Error(t('errors.noLibrary')) }
            return { libraryPath: this.libraryPath, session: this.session }
        },

        /** The OPDS server caches the book list; tell it the library changed. */
        libraryChanged() {
            void useOpdsStore().invalidateCache()
        },

        /** Imports files and adds the new books to the list. Resolves with the books actually added. */
        async importBooks(paths: string[]): Promise<Book[]> {
            const { libraryPath, session } = this.requireLibrary()
            const imported = await api.importBooks(paths, libraryPath)
            if (session !== this.session) { return [] }
            return this.addBooks(imported)
        },

        /**
         * Adds books the backend already saved (imports, Store downloads,
         * Calibre and device imports). A book already listed (same id or path)
         * is not added twice: `import_books` returns the existing entry for a
         * file the library already holds, and a batch can name a file twice.
         */
        addBooks(books: Book[]): Book[] {
            const seenIds = new Set(this.books.map(b => b.id))
            const seenPaths = new Set(this.books.map(b => b.path))
            const added: Book[] = []
            for (const book of books) {
                if (seenIds.has(book.id) || seenPaths.has(book.path)) { continue }
                seenIds.add(book.id)
                seenPaths.add(book.path)
                const cover = coverInfoOf(book)
                if (cover) { this.setCover(book.id, cover) }
                added.push(freezeBook(book))
            }
            if (added.length > 0) {
                this.books.push(...added)
                this.libraryChanged()
            }
            return added
        },

        /** MOBI/AZW/AZW3 → EPUB. The backend keeps the id and moves the original to the trash. */
        async convertToEpub(book: Book): Promise<Book> {
            const { libraryPath, session } = this.requireLibrary()
            const converted = await api.convertToEpub(libraryPath, book.id)
            if (session === this.session) {
                this.replaceBook(converted)
                this.libraryChanged()
            }
            return converted
        },

        /**
         * Saves one book's metadata. `changes` is applied over the book's
         * current metadata: a key that is present replaces the value, and a
         * key present with the value `undefined` clears that field (this is
         * how the edit dialog clears a series). Keys that are absent keep
         * their value. Callers that only want to *add* information (tidy-up)
         * must strip `undefined` keys first.
         *
         * For EPUBs the backend also writes the metadata into the file, and
         * rejects (changing nothing) if that fails.
         */
        async updateMetadata(bookId: string, changes: Partial<BookMetadata>): Promise<Book> {
            const { libraryPath, session } = this.requireLibrary()
            const current = this.bookById.get(bookId)
            if (!current) { throw new Error(t('errors.bookNotFound')) }
            const metadata = toFullMetadata({ ...current.metadata, ...changes })
            const updated = await api.updateMetadata(libraryPath, bookId, metadata)
            if (session === this.session) {
                this.replaceBook(updated)
                this.libraryChanged()
            }
            analytics.track('book_edit', { format: current.format })
            return updated
        },

        /**
         * Applies the same patch to several books. Only the fields present in
         * `patch` change; the backend merges them into each book's *own*
         * metadata. `seriesIndexes` (book id → volume) then numbers books
         * one by one (a series, in the order the user chose). (The old code sent book #1's whole metadata, which copied
         * its title, author, ISBN… onto every selected book.)
         *
         * Resolves with the updated books and the per-book failures; rejects
         * only if the command itself failed.
         */
        async batchUpdateMetadata(
            bookIds: string[],
            patch: BookMetadataPatch,
            seriesIndexes?: Readonly<Record<string, number>>,
        ): Promise<BatchUpdateResult> {
            const { libraryPath, session } = this.requireLibrary()
            const cleanPatch: BookMetadataPatch = {}
            for (const [k, v] of Object.entries(patch)) {
                if (v !== undefined) { (cleanPatch as Record<string, unknown>)[k] = v }
            }
            const res = await api.batchUpdateMetadata(libraryPath, bookIds, cleanPatch, seriesIndexes)
            const result: BatchUpdateResult = { books: res?.books ?? [], failures: res?.failures ?? [] }
            if (session === this.session) {
                this.replaceBooks(result.books)
                if (result.books.length > 0) { this.libraryChanged() }
            }
            analytics.track('book_edit_batch', { book_count_bucket: bucketBookCount(result.books.length) })
            return result
        },

        /**
         * Moves books to the trash (system Trash, or the library's
         * `.librarium/trash` folder on volumes without one). Only books the
         * backend reports as deleted leave the list; the others stay, and
         * their reasons are in `failures`.
         */
        async deleteBooks(bookIds: string[]): Promise<DeleteResult> {
            const { libraryPath, session } = this.requireLibrary()
            const res = await api.deleteBooks(libraryPath, bookIds)
            const result: DeleteResult = {
                deleted:      res?.deleted ?? [],
                failures:     res?.failures ?? [],
                libraryTrash: res?.libraryTrash ?? null,
            }
            if (session === this.session && result.deleted.length > 0) {
                this.removeBooks(result.deleted)
                this.libraryChanged()
            }
            return result
        },

        /**
         * "Remove from library": the books leave the library but their files
         * stay on disk; scans won't add them back (importing a file again
         * does). Only books the backend reports as forgotten leave the list.
         */
        async forgetBooks(bookIds: string[]): Promise<ForgetResult> {
            const { libraryPath, session } = this.requireLibrary()
            const res = await api.forgetBooks(libraryPath, bookIds)
            const result: ForgetResult = { forgotten: res?.forgotten ?? [], failures: res?.failures ?? [] }
            if (session === this.session && result.forgotten.length > 0) {
                this.removeBooks(result.forgotten)
                this.libraryChanged()
            }
            return result
        },

        /**
         * Stars (or unstars) books. The list changes at once (a starred book
         * moves up among the favorites) and is put back if saving fails;
         * rejects with the backend's error then. Only the library index
         * changes: no file is touched, and the OPDS server isn't concerned.
         */
        async setFavorite(bookIds: string[], favorite: boolean): Promise<void> {
            const { libraryPath, session } = this.requireLibrary()
            const before = bookIds.flatMap(id => this.bookById.get(id) ?? [])
            this.replaceBooks(before.map(book => ({ ...book, favorite })))
            try {
                const saved = await api.setFavorite(libraryPath, before.map(b => b.id), favorite)
                if (session === this.session) { this.replaceBooks(saved) }
            } catch (e) {
                if (session === this.session) { this.replaceBooks(before) }
                throw e
            }
        },

        async renameBook(bookId: string, newFilename: string): Promise<Book> {
            const { libraryPath, session } = this.requireLibrary()
            const updated = await api.renameBook(libraryPath, bookId, newFilename)
            if (session === this.session) {
                this.replaceBook(updated)
                this.libraryChanged()
            }
            return updated
        },

        /** Re-reads metadata and cover from the file, discarding edits made in Librarium. */
        async resetBookData(bookId: string): Promise<Book> {
            const { libraryPath, session } = this.requireLibrary()
            const fresh = await api.resetBookData(libraryPath, bookId)
            if (session !== this.session) { return fresh }
            this.replaceBook(fresh)
            this.clearCover(bookId)
            this.coverChecked.delete(bookId)
            this.libraryChanged()
            // The cover comes back from the file's embedded image, if any.
            await this.ensureCover(bookId).catch(() => null)
            return fresh
        },

        async saveCoverFromUrl(bookId: string, url: string): Promise<void> {
            const { libraryPath, session } = this.requireLibrary()
            const info = await api.saveCoverFromUrl(libraryPath, bookId, url)
            if (session === this.session) { this.setCover(bookId, info) }
        },

        async setBookCover(bookId: string, imageBase64: string): Promise<void> {
            const { libraryPath, session } = this.requireLibrary()
            const info = await api.setBookCover(libraryPath, bookId, imageBase64)
            if (session === this.session) { this.setCover(bookId, info) }
        },

        async deleteBookCover(bookId: string): Promise<void> {
            const { libraryPath, session } = this.requireLibrary()
            await api.deleteBookCover(libraryPath, bookId)
            if (session === this.session) {
                this.clearCover(bookId)
                // The backend now answers "no cover" for this book: don't ask.
                this.coverChecked.add(bookId)
            }
        },

        // ── Local state ──────────────────────────────────────────────────────

        /**
         * Replaces a listed book by id with the backend's new version (its
         * cover stays in `covers`; a cover the backend sent with it wins).
         * A book that isn't listed is ignored.
         */
        replaceBook(updated: Book) {
            this.replaceBooks([updated])
        },

        /** `replaceBook` for several books, in one pass over the list. */
        replaceBooks(updated: Book[]) {
            if (updated.length === 0) { return }
            const byId = new Map(updated.map(b => [b.id, b]))
            const books = this.books.slice()
            let changed = false
            books.forEach((book, i) => {
                const next = byId.get(book.id)
                if (!next) { return }
                const cover = coverInfoOf(next)
                if (cover) { this.setCover(next.id, cover) }
                books[i] = freezeBook(next)
                changed = true
            })
            if (changed) { this.books = books }
        },

        /** Takes books off the list (and the selection) after the backend removed them. */
        removeBooks(ids: string[]) {
            const gone = new Set(ids)
            this.books = this.books.filter(b => !gone.has(b.id))
            for (const id of gone) {
                this.selectedIds.delete(id)
                delete this.covers[id]
            }
        },

        /** Shows a cover: only the cards reading this book's entry re-render. */
        setCover(bookId: string, info: CoverInfo) {
            this.covers[bookId] = { coverPath: info.coverPath, coverVersion: info.coverVersion }
        },

        clearCover(bookId: string) {
            delete this.covers[bookId]
        },

        toggleSelect(id: string) {
            if (this.selectedIds.has(id)) { this.selectedIds.delete(id) } else { this.selectedIds.add(id) }
        },

        /** Selects every item shown (books and device files). */
        selectAll() {
            this.visibleItems.forEach(item => this.selectedIds.add(item.id))
        },

        /** Replaces the selection (a Shift-click range); ids not on the shelf are ignored. */
        setSelection(ids: Iterable<string>) {
            const listed = this.shelfById
            this.selectedIds = new Set([...ids].filter(id => listed.has(id)))
        },

        clearSelection() {
            this.selectedIds.clear()
        },

        /**
         * Drops selected items that are no longer visible. Runs whenever the
         * filters, the search or the scope change (and LibraryView runs it
         * when the device's books change): an action on "the selection"
         * must never reach books the user can't see (Cmd+A, then a search,
         * then Delete must not delete hidden books too).
         */
        pruneSelection() {
            const visible = new Set(this.visibleItems.map(item => item.id))
            for (const id of [...this.selectedIds]) {
                if (!visible.has(id)) { this.selectedIds.delete(id) }
            }
        },

        /** Shows everything, the library's books, or the device's. */
        setScope(scope: ShelfScope) {
            this.scope = scope
            this.pruneSelection()
        },

        setSort(config: SortConfig) {
            this.sort = config
        },

        setFilters(config: Partial<FilterConfig>) {
            this.filters = { ...this.filters, ...config }
            this.pruneSelection()
        },

        setViewMode(mode: ViewMode) {
            this.viewMode = mode
            localStorage.setItem('viewMode', mode)
        },

        /** How many books of each format the library holds (formats it has none of are absent). */
        getFormatCounts(): Partial<Record<BookFormat, number>> {
            const counts: Partial<Record<BookFormat, number>> = {}
            this.books.forEach(b => { counts[b.format] = (counts[b.format] ?? 0) + 1 })
            return counts
        },
    },
})
