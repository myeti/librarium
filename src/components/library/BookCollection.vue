<template>
    <div ref="scroller" class="book-collection" :class="`-${layout.kind}`" @scroll.passive="onScroll">
        <!-- The list's column header (sticky): part of the scroll content so it lines up with the rows. -->
        <slot name="header" />
        <div
            ref="canvas"
            class="book-collection__canvas"
            role="listbox"
            aria-multiselectable="true"
            :aria-label="label"
            :style="canvasStyle"
            @keydown="onKeydown"
            @click="onClick"
            @change="onChange"
            @contextmenu="onContextMenu"
            @focusin="onFocusIn"
            @focusout="onFocusOut"
        >
            <div
                v-for="row in rows"
                :key="row.index"
                class="book-collection__row"
                role="none"
                :style="row.style"
            >
                <div
                    v-for="cell in row.cells"
                    :key="cell.item.id"
                    class="book-collection__item"
                    role="option"
                    :data-index="cell.index"
                    :data-book-id="cell.item.id"
                    :tabindex="cell.index === activeIndex ? 0 : -1"
                    :aria-selected="selectedIds.has(cell.item.id) ? 'true' : 'false'"
                    :aria-posinset="cell.index + 1"
                    :aria-setsize="items.length"
                    :aria-label="itemLabel(cell.item)"
                >
                    <slot
                        :item="cell.item"
                        :index="cell.index"
                        :selected="selectedIds.has(cell.item.id)"
                        :active="cell.index === activeIndex && activeId !== null"
                    />
                </div>
            </div>
        </div>
    </div>
</template>

<script lang="ts">
    import { defineComponent, markRaw, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import type { ShelfItem } from '@/services/shelf'
    import { useLibraryStore } from '@/stores/library'
    import { isMacPlatform } from '@/constants/shortcuts'
    import { clickIntent, keyIntent, navigate, RangeSelection } from '@/utils/collectionNav'
    import {
        anchorOffset, computeMetrics, firstFullyVisibleRow, restoreAnchor, rowAt, rowOf, rowsPerPage, rowTop,
        scrollTopToReveal, totalHeight, visibleRows,
        type CollectionLayout, type Metrics, type RowRange,
    } from '@/utils/virtualWindow'
    import type { CollectionMenuRequest } from './collection'

    /** A rendered row of the virtual window. */
    interface WindowRow {
        index: number
        style: Record<string, string>
        cells: { item: ShelfItem, index: number }[]
    }

    /** Where the user was looking before the list or the layout changed. */
    interface ScrollAnchor {
        id: string
        offset: number
    }

    interface CollectionData {
        /** Scroller size, px (measured; 0 until mounted). */
        width: number
        height: number
        /** Height of the sticky header above the canvas (the list's column header), px. */
        headerHeight: number
        /** Rows rendered, overscan included; `top` is the first fully visible one. */
        window: RowRange & { top: number }
        /** Shift-click / Shift-arrow ranges (not reactive). */
        range: RangeSelection
        /** The book whose option last had focus inside the collection. */
        focusedId: string | null
        /** The last active id this component asked for (its own changes don't scroll). */
        emittedActiveId: string | null
        /** The books just changed: scrolling follows the anchor, not the active book. */
        booksChanging: boolean
        resizeObserver: ResizeObserver | null
        refocusTimer: ReturnType<typeof setTimeout> | null
    }

    const OVERSCAN_ROWS = 2

    /**
     * The library's books as one keyboard-focusable collection, virtualised:
     * only the rows on screen (plus OVERSCAN_ROWS above and below) are
     * rendered, so a library of thousands of books scrolls as smoothly as one
     * of thirty. LibraryGrid and LibraryList wrap it with their layout and
     * render each book through the default slot.
     *
     * **Virtual window** (maths in utils/virtualWindow.ts): the scroller is
     * measured (ResizeObserver: window resize, panel opened or resized), the
     * layout gives the number of columns and the row height, and the
     * scroll position gives the rows to render. When the books change (a
     * search, a sort) or the layout does (the panel opened), the book at
     * the top of the screen (or the active one, if on screen) is kept in the
     * same place rather than jumping.
     *
     * **Keyboard and selection** (model in utils/collectionNav.ts): a
     * `listbox` with a roving tabindex. Only the active book is in the Tab
     * order; the arrows (grid-aware), Home/End and PageUp/PageDown move it,
     * Shift extends the selection, Space toggles it, Enter opens the book,
     * the ContextMenu key and Shift+F10 open its menu. A click opens the
     * book, ⌘/Ctrl-click toggles it, Shift-click selects a range; the
     * checkbox toggles. The active book's row is always rendered, even when
     * scrolled away, so keyboard focus is never lost to the virtual window.
     *
     * The active book is the parent's (`v-model:activeId`): the detail
     * panel shows it. The selection is the library store's.
     */
    export default defineComponent({
        name: 'BookCollection',

        props: {
            /** The shelf's items to show (library books and device files), in order. */
            items: { type: Array as PropType<ShelfItem[]>, required: true },
            layout: { type: Object as PropType<CollectionLayout>, required: true },
            activeId: { type: String as PropType<string | null>, default: null },
            /** Accessible name of the listbox. */
            label: { type: String, required: true },
            /** Accessible name of one book ("Dune, Frank Herbert, 42% read"). */
            itemLabel: { type: Function as PropType<(item: ShelfItem) => string>, required: true },
        },

        emits: {
            'update:activeId': (_id: string | null) => true,
            /** Enter or a plain click: show the book (the detail panel). */
            open: (_item: ShelfItem) => true,
            menu: (_request: CollectionMenuRequest) => true,
        },

        data(): CollectionData {
            return {
                width: 0,
                height: 0,
                headerHeight: 0,
                window: { first: 0, last: -1, top: 0 },
                range: markRaw(new RangeSelection()),
                focusedId: null,
                emittedActiveId: null,
                booksChanging: false,
                resizeObserver: null,
                refocusTimer: null,
            }
        },

        computed: {
            ...mapStores(useLibraryStore),

            selectedIds(): Set<string> {
                return this.libraryStore.selectedIds
            },

            metrics(): Metrics {
                return computeMetrics(this.layout, this.width)
            },

            ids(): string[] {
                return this.items.map(b => b.id)
            },

            indexById(): Map<string, number> {
                return new Map(this.ids.map((id, i) => [id, i]))
            },

            /**
             * The book that has the Tab stop: the active one, or (none, or
             * filtered out) the first one fully on screen.
             */
            activeIndex(): number {
                if (this.items.length === 0) { return -1 }
                const index = this.activeId === null ? undefined : this.indexById.get(this.activeId)
                if (index !== undefined) { return index }
                return Math.min(this.items.length - 1, this.window.top * this.metrics.columns)
            },

            rows(): WindowRow[] {
                const m = this.metrics
                const rows: WindowRow[] = []
                const last = Math.min(this.window.last, Math.ceil(this.items.length / m.columns) - 1)
                for (let r = this.window.first; r <= last; r++) { rows.push(this.makeRow(r)) }
                // Keep the active book's row even when it is scrolled away, so
                // a focused book never vanishes from under the keyboard.
                if (this.activeIndex >= 0) {
                    const activeRow = rowOf(this.activeIndex, m.columns)
                    if (activeRow < this.window.first || activeRow > last) { rows.push(this.makeRow(activeRow)) }
                }
                return rows
            },

            canvasStyle(): Record<string, string> {
                const m = this.metrics
                return {
                    height: `${totalHeight(m, this.items.length)}px`,
                    '--columns': String(m.columns),
                    '--gap': `${m.gap}px`,
                    '--pad-x': `${m.paddingX}px`,
                }
            },
        },

        watch: {
            // Flush 'pre': runs before the DOM shows the new list, so the
            // anchor is read from what is still on screen.
            items(_next: ShelfItem[], previous: ShelfItem[]) {
                const anchor = this.captureAnchor(previous)
                this.booksChanging = true
                void this.$nextTick(() => {
                    this.booksChanging = false
                    this.restoreScroll(anchor)
                    this.refocusIfLost()
                })
            },

            // Something else changed the active book (← → with the panel
            // open): show it. Not for changes made here (a click shouldn't
            // scroll), nor when the list itself changed (the anchor rules).
            activeId(id: string | null) {
                const own = id === this.emittedActiveId
                this.emittedActiveId = null
                if (own || this.booksChanging) { return }
                const index = id === null ? undefined : this.indexById.get(id)
                if (index !== undefined) { this.reveal(index) }
            },

            metrics() {
                this.updateWindow()
            },
        },

        mounted() {
            this.measure()
            if (typeof ResizeObserver !== 'undefined') {
                this.resizeObserver = new ResizeObserver(() => this.onResize())
                this.resizeObserver.observe(this.$refs.scroller as HTMLElement)
            }
            // Coming back to the view (or switching grid ⇄ list): show the active book.
            const index = this.activeId === null ? undefined : this.indexById.get(this.activeId)
            if (index !== undefined) { this.reveal(index) }
        },

        beforeUnmount() {
            this.resizeObserver?.disconnect()
            if (this.refocusTimer) { clearTimeout(this.refocusTimer) }
        },

        methods: {
            makeRow(r: number): WindowRow {
                const m = this.metrics
                const start = r * m.columns
                return {
                    index: r,
                    style: { top: `${rowTop(m, r)}px`, height: `${m.itemHeight}px` },
                    cells: this.items.slice(start, start + m.columns).map((item, k) => ({ item, index: start + k })),
                }
            },

            scrollerEl(): HTMLElement | null {
                return (this.$refs.scroller as HTMLElement | undefined) ?? null
            },

            /**
             * The scroll position, in canvas coordinates. The header is
             * sticky: the canvas row under it at scroll position s is row
             * s of the canvas, and the header hides the top `headerHeight`
             * px of the viewport.
             */
            canvasScrollTop(): number {
                return this.scrollerEl()?.scrollTop ?? 0
            },

            /** The viewport height left for books (below the sticky header). */
            viewportHeight(): number {
                return Math.max(0, this.height - this.headerHeight)
            },

            measure() {
                const scroller = this.scrollerEl()
                const canvas = this.$refs.canvas as HTMLElement | undefined
                if (!scroller) { return }
                this.width = scroller.clientWidth
                this.height = scroller.clientHeight
                this.headerHeight = canvas?.offsetTop ?? 0
                this.updateWindow()
            },

            onResize() {
                // Keep what the user was looking at when the columns change.
                const anchor = this.captureAnchor(this.items)
                this.measure()
                void this.$nextTick(() => {
                    this.restoreScroll(anchor)
                    this.refocusIfLost()
                })
            },

            onScroll() {
                this.updateWindow()
            },

            /** Recomputes the rows to render; assigns only when they changed (no re-render per scroll event). */
            updateWindow() {
                const m = this.metrics
                const count = this.items.length
                const scrollTop = this.canvasScrollTop()
                const viewport = this.viewportHeight()
                const { first, last } = visibleRows(m, count, scrollTop, viewport, OVERSCAN_ROWS)
                const top = firstFullyVisibleRow(m, count, scrollTop, viewport)
                const w = this.window
                if (w.first !== first || w.last !== last || w.top !== top) {
                    this.window = { first, last, top }
                }
            },

            // ── Scroll anchoring ────────────────────────────────────────────

            /** The active book if it is on screen, else the first book of the top row. */
            captureAnchor(items: readonly ShelfItem[]): ScrollAnchor | null {
                if (items.length === 0) { return null }
                const m = this.metrics
                const scrollTop = this.canvasScrollTop()
                const viewport = this.viewportHeight()
                const activeIndex = this.activeId === null ? -1 : items.findIndex(b => b.id === this.activeId)
                if (activeIndex !== -1) {
                    const offset = anchorOffset(m, activeIndex, scrollTop)
                    if (offset + m.itemHeight > 0 && offset < viewport) { return { id: items[activeIndex].id, offset } }
                }
                const topIndex = Math.min(items.length - 1, rowAt(m, items.length, scrollTop) * m.columns)
                return { id: items[topIndex].id, offset: anchorOffset(m, topIndex, scrollTop) }
            },

            /** Puts the anchor book back where it was; leaves the position alone (clamped) if it is gone. */
            restoreScroll(anchor: ScrollAnchor | null) {
                const scroller = this.scrollerEl()
                if (!scroller) { return }
                const index = anchor ? this.indexById.get(anchor.id) : undefined
                if (anchor && index !== undefined) {
                    const target = restoreAnchor(this.metrics, this.items.length, this.viewportHeight(), index, anchor.offset)
                    if (Math.abs(target - this.canvasScrollTop()) >= 1) { scroller.scrollTop = target }
                }
                this.updateWindow()
            },

            // ── Active book, focus ──────────────────────────────────────────

            /** Scrolls just enough to show book `index` (the window follows at once). */
            reveal(index: number) {
                const scroller = this.scrollerEl()
                if (!scroller || this.height === 0) { return }
                const current = this.canvasScrollTop()
                const target = scrollTopToReveal(this.metrics, this.items.length, index, current, this.viewportHeight())
                if (Math.abs(target - current) >= 1) {
                    scroller.scrollTop = target
                    this.updateWindow()
                }
            },

            /**
             * Makes book `index` the active one. From the keyboard it is also
             * scrolled into view and focused; a click leaves the scroll alone.
             */
            moveTo(index: number, from: 'keyboard' | 'pointer') {
                const book = this.items[index]
                if (!book) { return }
                this.emittedActiveId = book.id
                if (book.id !== this.activeId) { this.$emit('update:activeId', book.id) }
                if (from === 'keyboard') {
                    this.reveal(index)
                    void this.$nextTick(() => this.focusIndex(index))
                }
            },

            optionAt(index: number): HTMLElement | null {
                const canvas = this.$refs.canvas as HTMLElement | undefined
                return canvas?.querySelector<HTMLElement>(`[role="option"][data-index="${index}"]`) ?? null
            },

            focusIndex(index: number) {
                this.optionAt(index)?.focus({ preventScroll: true })
            },

            /** Focuses the active book (after the panel closed, for instance). */
            focusActive() {
                if (this.activeIndex < 0) { return }
                this.reveal(this.activeIndex)
                void this.$nextTick(() => this.focusIndex(this.activeIndex))
            },

            /** The option element an event happened in, and its index. */
            optionOf(target: EventTarget | null): { el: HTMLElement, index: number } | null {
                const el = target instanceof Element ? target.closest<HTMLElement>('[role="option"]') : null
                if (!el) { return null }
                const index = Number(el.dataset.index)
                return Number.isInteger(index) && this.items[index] ? { el, index } : null
            },

            onFocusIn(e: FocusEvent) {
                const option = this.optionOf(e.target)
                this.focusedId = option ? this.items[option.index].id : null
            },

            onFocusOut(e: FocusEvent) {
                // Focus going elsewhere on the page (not to a dialog or menu
                // opened from here, not lost because the book was removed).
                const next = e.relatedTarget
                if (next instanceof Element && !(this.$refs.canvas as HTMLElement).contains(next)
                    && !next.closest('dialog, [role="menu"]')) {
                    this.focusedId = null
                }
            },

            /**
             * Focus was on a book and the page lost it: the book was removed
             * (deleted, filtered out), or its element was re-created (the
             * number of columns changed, so books moved to other rows). Put
             * it back on the same book, or the one that took its place, once
             * dialogs have handed focus back.
             */
            refocusIfLost() {
                const id = this.focusedId
                if (id === null) { return }
                if (this.refocusTimer) { clearTimeout(this.refocusTimer) }
                this.refocusTimer = setTimeout(() => {
                    this.refocusTimer = null
                    const lost = !document.activeElement || document.activeElement === document.body
                    if (!lost || this.focusedId !== id) { return }
                    const index = this.indexById.get(id)
                    if (index !== undefined) {
                        this.reveal(index)
                        void this.$nextTick(() => this.focusIndex(index))
                    } else {
                        this.focusActive()
                    }
                }, 0)
            },

            // ── Selection ───────────────────────────────────────────────────

            setAnchor(index: number) {
                this.range.setAnchor(this.ids[index] ?? null, this.selectedIds)
            },

            toggleAt(index: number) {
                const id = this.ids[index]
                if (id === undefined) { return }
                this.libraryStore.toggleSelect(id)
                this.setAnchor(index)
            },

            selectRangeTo(index: number) {
                const next = this.range.extend(this.ids, index, this.selectedIds, this.activeIndex)
                this.libraryStore.setSelection(next)
            },

            // ── Events ──────────────────────────────────────────────────────

            onKeydown(e: KeyboardEvent) {
                if (e.defaultPrevented || this.items.length === 0) { return }
                const intent = keyIntent(e)
                if (!intent) { return }
                const option = this.optionOf(e.target)
                // Keys on a control inside a book (a list row's button) are its own.
                const onControl = option !== null && e.target !== option.el
                if (onControl && intent.type !== 'move' && intent.type !== 'menu') { return }
                const current = option?.index ?? this.activeIndex

                if (intent.type === 'move') {
                    e.preventDefault()
                    const m = this.metrics
                    const next = navigate(intent.key, current, this.items.length, m.columns, rowsPerPage(m, this.viewportHeight()))
                    if (intent.extend) {
                        this.selectRangeTo(next)
                    } else {
                        this.setAnchor(next)
                    }
                    this.moveTo(next, 'keyboard')
                } else if (intent.type === 'toggle') {
                    e.preventDefault()
                    if (intent.range) { this.selectRangeTo(current) } else { this.toggleAt(current) }
                    this.moveTo(current, 'keyboard')
                } else if (intent.type === 'open') {
                    e.preventDefault()
                    this.moveTo(current, 'keyboard')
                    this.$emit('open', this.items[current])
                } else {
                    e.preventDefault()
                    const el = option?.el ?? this.optionAt(current)
                    if (!el) { return }
                    const box = el.getBoundingClientRect()
                    this.$emit('menu', { item: this.items[current], x: box.left + 16, y: box.top + Math.min(box.height, 48), origin: el })
                }
            },

            onClick(e: MouseEvent) {
                const option = this.optionOf(e.target)
                if (!option) { return }
                const { index, el } = option
                // A control inside the book (list row buttons) did its own thing.
                if (e.target !== el && (e.target as Element).closest('button, a, input, label')) { return }
                const intent = clickIntent(e, isMacPlatform())
                if (intent === 'range') {
                    this.selectRangeTo(index)
                    this.moveTo(index, 'pointer')
                } else if (intent === 'toggle') {
                    this.toggleAt(index)
                    this.moveTo(index, 'pointer')
                } else {
                    this.setAnchor(index)
                    this.moveTo(index, 'pointer')
                    this.$emit('open', this.items[index])
                }
                el.focus({ preventScroll: true })
            },

            /** A book's checkbox was clicked (see SelectCheckbox). */
            onChange(e: Event) {
                const input = e.target
                if (!(input instanceof HTMLInputElement) || !input.closest('[data-select-toggle]')) { return }
                const option = this.optionOf(input)
                if (!option) { return }
                this.toggleAt(option.index)
                this.moveTo(option.index, 'pointer')
                // Back on the book, so the keyboard keeps working from here.
                option.el.focus({ preventScroll: true })
            },

            onContextMenu(e: MouseEvent) {
                const option = this.optionOf(e.target)
                if (!option) { return }
                e.preventDefault()
                this.moveTo(option.index, 'pointer')
                option.el.focus({ preventScroll: true })
                this.$emit('menu', { item: this.items[option.index], x: e.clientX, y: e.clientY, origin: option.el })
            },
        },
    })
</script>

<style lang="scss" scoped>
.book-collection {
    position: relative;
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    overscroll-behavior: contain;
    // Scrolling repaints only this box.
    contain: strict;
}

.book-collection__canvas {
    position: relative;
    width: 100%;
    user-select: none;
    -webkit-user-select: none;
    outline: none;
}

.book-collection__row {
    position: absolute;
    left: 0;
    right: 0;
    padding: 0 var(--pad-x);
    contain: layout style;
}

.-grid .book-collection__row {
    display: grid;
    grid-template-columns: repeat(var(--columns), minmax(0, 1fr));
    gap: var(--gap);
}

.book-collection__item {
    position: relative;
    height: 100%;
    min-width: 0;
    border-radius: var(--radius-sm);

    // The ring sits on the option; the card inside draws its own focus look.
    &:focus-visible {
        outline-offset: 2px;
    }
}
</style>
