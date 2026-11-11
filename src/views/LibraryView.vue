<template>
    <div class="library-view">
        <!-- One slot, one height: the selection toolbar replaces the filter bar in place. -->
        <div class="library-toolbar">
            <SelectionToolbar v-if="hasSelection" />
            <FilterSortBar v-else ref="filterBar" />
        </div>

        <div class="library-main">
            <div class="library-collection">
                <!-- The library folder isn't there: a NAS share not mounted, a drive unplugged. -->
                <EmptyState
                    v-if="libraryStore.unavailable && !libraryStore.loading"
                    alert
                    icon="hard-drive"
                    :title="$t('library.unavailableTitle')"
                    :description="$t('library.unavailableBody', { path: libraryStore.libraryPath ?? '' })"
                >
                    <template #actions>
                        <BaseButton variant="primary" @click="retryUnavailable">{{ $t('library.retry') }}</BaseButton>
                    </template>
                </EmptyState>

                <!-- The folder is there but Librarium may not read it (macOS:
                     Privacy & Security › Files and Folders). The backend's
                     message says where to allow it. Not "is the NAS connected?". -->
                <EmptyState
                    v-else-if="libraryStore.permissionDenied && !libraryStore.loading"
                    alert
                    icon="lock"
                    :title="$t('library.permissionDeniedTitle')"
                    :description="libraryStore.error ?? ''"
                >
                    <template #actions>
                        <BaseButton variant="primary" @click="retryUnavailable">{{ $t('library.retry') }}</BaseButton>
                    </template>
                </EmptyState>

                <!-- The library couldn't be read (unreadable index…). -->
                <EmptyState
                    v-else-if="libraryStore.error && !libraryStore.loading"
                    alert
                    icon="alert-triangle"
                    :title="$t('library.openFailedTitle')"
                    :description="libraryStore.error"
                >
                    <template #actions>
                        <BaseButton variant="primary" @click="libraryStore.refresh('manual')">{{ $t('library.retry') }}</BaseButton>
                    </template>
                </EmptyState>

                <!-- First scan only: a rescan keeps the books on screen. -->
                <EmptyState
                    v-else-if="libraryStore.loading && libraryStore.books.length === 0"
                    loading
                    :description="$t('library.loading')"
                />

                <EmptyState
                    v-else-if="libraryStore.shelf.length === 0"
                    icon="book-open"
                    :icon-size="56"
                    :title="$t('library.emptyTitle')"
                    :description="$t('library.emptyBody')"
                >
                    <template #actions>
                        <BaseButton variant="primary" icon="plus" @click="importFiles">{{ $t('library.import') }}</BaseButton>
                    </template>
                </EmptyState>

                <!-- The device's books, and there are none (no search or filter to blame). -->
                <EmptyState
                    v-else-if="visibleItems.length === 0 && deviceScopeEmpty"
                    icon="tablet"
                    :title="$t('shelf.deviceEmptyTitle', { device: deviceName })"
                    :description="$t('shelf.deviceEmptyBody')"
                />

                <EmptyState
                    v-else-if="visibleItems.length === 0"
                    icon="search"
                    :title="$t('library.noResults')"
                    :description="$t('library.noResultsBody')"
                />

                <LibraryGrid
                    v-else-if="libraryStore.viewMode === 'grid'"
                    ref="collection"
                    :items="visibleItems"
                    :active-id="activeId"
                    :selection-mode="hasSelection"
                    :device-name="deviceName"
                    @update:active-id="setActive"
                    @open="openPanel"
                    @menu="openMenu"
                />
                <LibraryList
                    v-else
                    ref="collection"
                    :items="visibleItems"
                    :active-id="activeId"
                    :selection-mode="hasSelection"
                    :show-progress="showProgress"
                    :device-name="deviceName"
                    @update:active-id="setActive"
                    @open="openPanel"
                    @menu="openMenu"
                />

                <IconButton
                    class="import-fab"
                    icon="plus"
                    :label="$t('library.import')"
                    :title="importTitle"
                    @click="importFiles"
                />
            </div>
        </div>

        <!-- The details: a dialog over the books, which stay as they are. -->
        <BookDetailModal
            v-if="panelItem"
            :item="panelItem"
            :index="panelIndex"
            :total="visibleItems.length"
            @step="step"
            @close="closePanel"
        />

        <BookContextMenu ref="menu" @details="openPanel" />
        <LibraryDialogs />
    </div>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { useLibraryStore } from '@/stores/library'
    import { useDeviceStore } from '@/stores/device'
    import { useUsersStore } from '@/stores/users'
    import { useShortcutsStore, type ShortcutHandler } from '@/stores/shortcuts'
    import { useImportsStore } from '@/stores/imports'
    import { useBookActionsStore } from '@/stores/bookActions'
    import { useNotificationsStore } from '@/stores/notifications'
    import { useModalsStore } from '@/stores/modals'
    import { formatCombo, SHORTCUTS } from '@/constants/shortcuts'
    import { isShelfScope, type ShelfItem } from '@/services/shelf'
    import { deviceLabel } from '@/utils/device'
    import { nextActiveId } from '@/utils/collectionNav'
    import FilterSortBar from '@/components/FilterSortBar.vue'
    import SelectionToolbar from '@/components/SelectionToolbar.vue'
    import BookDetailModal from '@/components/library/BookDetailModal.vue'
    import LibraryGrid from '@/components/library/LibraryGrid.vue'
    import LibraryList from '@/components/library/LibraryList.vue'
    import BookContextMenu from '@/components/library/BookContextMenu.vue'
    import LibraryDialogs from '@/components/library/LibraryDialogs.vue'
    import EmptyState from '@/components/ui/EmptyState.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import IconButton from '@/components/ui/IconButton.vue'
    import type { CollectionMenuRequest } from '@/components/library/collection'
    import type { ShortcutId } from '@/constants/shortcuts'

    interface LibraryViewData {
        /** The collection's current book (keyboard cursor; the details follow it). */
        activeId: string | null
        /** The item in the details (null: closed): a library book's id, or a device file's `device:…` id. */
        panelBookId: string | null
    }

    /** The collection component's methods the view calls. */
    interface CollectionRef { focusActive(): void }

    /**
     * The library screen: the toolbar (filters, or the selection's actions),
     * the books (LibraryGrid or LibraryList: a virtualised, keyboard-driven
     * collection), the detail panel and the book menu. It renders the
     * library store (which the users store opens for the current user) and
     * doesn't load anything itself.
     *
     * Where the work is:
     * - book actions and their dialogs: stores/bookActions.ts + LibraryDialogs;
     * - imports (button, ⌘O, files dropped anywhere): stores/imports.ts +
     *   FileDropTarget (App.vue);
     * - focus, keys, selection, virtual scrolling: BookCollection.
     *
     * **Details.** A book opens (click, Enter, ⌘I, the menu's Show
     * details) in BookDetailModal, a dialog over the books: they don't move.
     * ← → and its arrows show the previous / next book as shown here (the
     * collection's active book follows, so focus comes back to the last
     * book seen when it closes); Escape, ⌘I or × close it.
     *
     * **Shortcuts and menu commands** (constants/shortcuts.ts, through the
     * shortcuts store), registered while the view is shown: ⌘A select all
     * shown (again: none), Delete/⌫ move the selection (or the focused book)
     * to the Trash after confirming, Escape clears the selection, ⌘F
     * search, ⌘I show/hide the details, and the menu's Export… (both also
     * while the details are open, and only then among dialogs). ⌘O import,
     * ⌘K look for a device, ⌘1/⌘2 grid/list and Find duplicates work from
     * any view: they are App-level commands (components/AppCommands.vue).
     */
    export default defineComponent({
        name: 'LibraryView',

        components: {
            FilterSortBar, SelectionToolbar, BookDetailModal, LibraryGrid, LibraryList, BookContextMenu,
            LibraryDialogs, EmptyState, BaseButton, IconButton,
        },

        data(): LibraryViewData {
            return {
                activeId: null,
                panelBookId: null,
            }
        },

        computed: {
            ...mapStores(
                useLibraryStore, useDeviceStore, useUsersStore, useShortcutsStore, useImportsStore, useBookActionsStore,
                useNotificationsStore, useModalsStore,
            ),

            /** What the collection shows: library books and device files (the library store's shelf). */
            visibleItems(): ShelfItem[] {
                return this.libraryStore.visibleItems
            },

            deviceName(): string {
                return deviceLabel(this.deviceStore.device)
            },

            /** The device's books are asked for, and the device has none. */
            deviceScopeEmpty(): boolean {
                return this.libraryStore.effectiveScope === 'device'
                    && !this.libraryStore.shelf.some(item => item.presence !== 'libraryOnly')
            },

            /** The list's progress column: some book shown has progress on the device. */
            showProgress(): boolean {
                return this.libraryStore.visibleItems.some(item => item.progress !== undefined)
            },

            hasSelection(): boolean {
                return this.libraryStore.selectedIds.size > 0
            },

            panelItem(): ShelfItem | null {
                return this.panelBookId === null ? null : this.libraryStore.shelfById.get(this.panelBookId) ?? null
            },

            /** The details' item among the books shown (-1: not among them). */
            panelIndex(): number {
                return this.panelBookId === null ? -1 : this.visibleItems.findIndex(item => item.id === this.panelBookId)
            },

            importTitle(): string {
                const combo = SHORTCUTS['library.import'].keys[0]
                return `${this.$t('library.import')} (${formatCombo(combo, this.shortcutsStore.isMac)})`
            },
        },

        watch: {
            // Another library was opened (user switch, new folder): nothing
            // on screen may still point at the previous one.
            'libraryStore.session'() {
                this.activeId = null
                this.panelBookId = null
                this.bookActionsStore.reset()
            },

            // Filtered, sorted, or books removed: the active book stays if it
            // is still shown, else the one that took its place becomes active.
            visibleItems(next: ShelfItem[], previous: ShelfItem[]) {
                this.activeId = nextActiveId(previous.map(b => b.id), next.map(b => b.id), this.activeId)
            },

            // The device's books changed (unplugged, a book removed or sent):
            // the selection only keeps what is still shown.
            'libraryStore.shelf'() {
                this.libraryStore.pruneSelection()
            },

            // The details' item went away: a book deleted, a device file
            // removed, or a device file added to the library (it is now the
            // library book: the details follow it). Else they close.
            panelItem(item: ShelfItem | null, previous: ShelfItem | null) {
                if (item !== null || this.panelBookId === null) { return }
                const path = previous?.presence === 'deviceOnly' ? previous.copies[0]?.path : undefined
                const now = path ? this.libraryStore.shelf.find(i => i.book && i.copies.some(c => c.path === path)) : undefined
                if (now) {
                    this.panelBookId = now.id
                    this.activeId = now.id
                } else {
                    this.closePanel()
                }
            },

            // The device went away while its books were shown: show everything again.
            'deviceStore.isConnected'(connected: boolean) {
                if (!connected && this.libraryStore.scope === 'device') {
                    this.libraryStore.setScope('all')
                    this.notificationsStore.push('info', this.$t('shelf.deviceGone'))
                }
            },

            // `/library?scope=device` (the sidebar's device card, the old /device address).
            '$route.query.scope': {
                handler(scope: unknown) {
                    if (!isShelfScope(scope)) { return }
                    this.libraryStore.setScope(scope)
                    void this.$router.replace({ query: {} })
                },
                immediate: true,
            },

            // The device that reports progress changed: read its progress.
            'deviceStore.device.id': {
                handler(id: string | undefined) {
                    if (id && Object.keys(this.deviceStore.readingProgress).length === 0) {
                        void this.deviceStore.loadReadingProgress()
                    }
                },
                immediate: true,
            },
        },

        mounted() {
            for (const [id, handler] of this.shortcutHandlers()) { this.shortcutsStore.register(id, handler) }
        },

        beforeUnmount() {
            for (const [id, handler] of this.shortcutHandlers()) { this.shortcutsStore.unregister(id, handler) }
            // Dialogs opened from here don't survive the view.
            this.bookActionsStore.reset()
        },

        methods: {
            /** Every global shortcut the library screen handles (registered while it is shown). */
            shortcutHandlers(): [ShortcutId, ShortcutHandler][] {
                return [
                    ['library.selectAll', this.onSelectAllShortcut],
                    ['library.delete', this.onDeleteShortcut],
                    ['library.close', this.onEscapeShortcut],
                    ['library.search', this.onSearchShortcut],
                    ['view.toggleDetails', this.onToggleDetailsShortcut],
                    ['library.export', this.onExportCommand],
                ]
            },

            collection(): CollectionRef | null {
                return (this.$refs.collection as CollectionRef | undefined) ?? null
            },

            // ── Active book and details ─────────────────────────────────────

            /** The collection moved to another book (the details follow it, if open). */
            setActive(id: string | null) {
                this.activeId = id
                if (this.panelBookId !== null && id !== null) { this.panelBookId = id }
            },

            openPanel(item: ShelfItem) {
                this.activeId = item.id
                this.panelBookId = item.id
            },

            /** Closes the details; focus goes back to the book last shown in them. */
            closePanel() {
                if (this.panelBookId === null) { return }
                this.panelBookId = null
                // After the dialog's own focus return (to the book it opened on).
                void this.$nextTick(() => this.collection()?.focusActive())
            },

            /** Moves the active book (and the details) by `delta` in the visible list. */
            step(delta: number): boolean {
                const books = this.visibleItems
                const current = this.panelBookId ?? this.activeId
                const index = books.findIndex(b => b.id === current)
                const next = books[index + delta]
                if (index === -1 || !next) { return false }
                this.setActive(next.id)
                return true
            },

            openMenu(request: CollectionMenuRequest) {
                (this.$refs.menu as { open(r: CollectionMenuRequest): void } | undefined)?.open(request)
            },

            importFiles() {
                void this.importsStore.pickAndImport()
            },

            /** "Library folder not reachable" → Retry: users and library are read again. */
            async retryUnavailable() {
                await this.usersStore.retry()
            },

            // ── Shortcuts ───────────────────────────────────────────────────

            /** ⌘A: every book shown; again: none. */
            onSelectAllShortcut(): boolean {
                const books = this.visibleItems
                if (books.length === 0) { return false }
                const selected = this.libraryStore.selectedIds
                if (books.every(b => selected.has(b.id))) { this.libraryStore.clearSelection() } else { this.libraryStore.selectAll() }
                return true
            },

            /**
             * Delete / ⌫: the selection, else the focused book, else the
             * panel's book; always confirmed. Library books go to the Trash;
             * files only on the e-reader (which have no other removal) are
             * offered for removal from it. A library book is never removed
             * from the device by this key.
             */
            onDeleteShortcut(): boolean {
                if (this.hasSelection) {
                    const books = this.libraryStore.selectedBooks.map(b => b.id)
                    if (books.length > 0) {
                        this.bookActionsStore.requestDelete(books, 'selection')
                    } else {
                        this.bookActionsStore.requestRemoveFromDevice(this.libraryStore.selectedItems)
                    }
                    return true
                }
                const focused = document.activeElement instanceof Element
                    ? document.activeElement.closest<HTMLElement>('[role="option"][data-book-id]')?.dataset.bookId
                    : undefined
                const id = focused ?? this.panelBookId
                const item = id ? this.libraryStore.shelfById.get(id) : undefined
                if (!item) { return false }
                if (item.book) {
                    this.bookActionsStore.requestDelete([item.id], 'single')
                } else {
                    this.bookActionsStore.requestRemoveFromDevice([item])
                }
                return true
            },

            /** Escape: clears the selection (the details, a dialog, close on their own Escape). */
            onEscapeShortcut(): boolean {
                if (!this.hasSelection) { return false }
                this.libraryStore.clearSelection()
                return true
            },

            /**
             * Whether a command may run now: no dialog is open, or only the
             * details (a dialog opened over them has the keyboard).
             */
            onlyDetailsOpen(): boolean {
                const dialogs = this.modalsStore.count
                return dialogs === 0 || (dialogs === 1 && this.panelBookId !== null)
            },

            onSearchShortcut(): boolean {
                const bar = this.$refs.filterBar as { focusSearch(): void } | undefined
                if (!bar) { return false }
                bar.focusSearch()
                return true
            },

            /** ⌘I: closes the details, or opens them on the active (else the only selected) book. */
            onToggleDetailsShortcut(): boolean {
                if (!this.onlyDetailsOpen()) { return false }
                if (this.panelBookId !== null) {
                    this.closePanel()
                    return true
                }
                const selected = this.libraryStore.selectedItems
                const id = this.activeId ?? (selected.length === 1 ? selected[0].id : null)
                const item = id ? this.visibleItems.find(b => b.id === id) : undefined
                if (!item) { return false }
                this.openPanel(item)
                return true
            },

            /** The menu's Export…: the book in the details, else the only selected one. */
            onExportCommand(): boolean {
                if (!this.onlyDetailsOpen()) { return false }
                const selected = this.libraryStore.selectedBooks
                const book = this.panelItem?.book ?? (selected.length === 1 ? selected[0] : null)
                if (book) {
                    void this.bookActionsStore.exportBook(book)
                } else {
                    this.notificationsStore.push('info', this.$t('shortcuts.exportNothing'))
                }
                return true
            },
        },
    })
</script>

<style lang="scss" scoped>
// Toolbar height: both bars fill the same slot, so swapping them moves nothing.
$toolbar-height: 68px;

.library-view {
    position: relative;
    display: grid;
    grid-template-rows: $toolbar-height minmax(0, 1fr);
    height: 100%;
    overflow: hidden;
}

.library-toolbar {
    position: relative;
    z-index: var(--z-sticky);
    min-width: 0;
}

.library-main {
    display: grid;
    grid-template-rows: minmax(0, 1fr);
    min-height: 0;
}

.library-collection {
    position: relative;
    min-width: 0;
    min-height: 0;
}

// ── Import button ───────────────────────────────────────────────────────────

// IconButton, drawn as the round accent button of the library (selectors
// doubled up to win over IconButton's own size and colours).
.library-collection > .import-fab.ui-icon-button {
    position: absolute;
    right: var(--space-5);
    bottom: var(--space-5);
    z-index: var(--z-raised);
    width: 52px;
    height: 52px;
    border-radius: var(--radius-full);
    background: linear-gradient(135deg, var(--accent-strong) 0%, var(--accent-dark) 100%);
    color: var(--on-accent);
    box-shadow: 0 4px 16px color-mix(in srgb, var(--accent-dark) 40%, transparent);
    transition: background var(--transition), transform var(--transition), box-shadow var(--transition);

    &:hover {
        background: linear-gradient(135deg, var(--accent-hover) 0%, var(--accent-dark) 100%);
        color: var(--on-accent);
        transform: scale(1.06);
        box-shadow: 0 6px 20px color-mix(in srgb, var(--accent-dark) 45%, transparent);
    }

    :deep(svg) {
        width: 22px;
        height: 22px;
    }
}
</style>
