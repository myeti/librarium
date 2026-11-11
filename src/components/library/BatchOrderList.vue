<template>
    <div class="batch-order">
        <p :id="hintId" class="batch-order__hint">{{ $t('metadata.orderHint') }}</p>
        <ol ref="list" class="batch-order__list" :aria-describedby="hintId">
            <li
                v-for="(book, index) in modelValue"
                :key="book.id"
                class="batch-order__item"
                :class="{ '-dragging': draggingId === book.id }"
            >
                <!-- The handle: dragged with the pointer, moved with ↑ ↓ Home End. -->
                <button
                    type="button"
                    class="batch-order__handle"
                    :aria-label="$t('metadata.moveBook', { title: titleOf(book), n: index + 1, total: modelValue.length })"
                    :disabled="disabled"
                    @pointerdown="startDrag($event, book.id)"
                    @keydown="onHandleKey($event, index)"
                >
                    <vue-feather type="menu" size="14" aria-hidden="true" />
                </button>
                <span class="batch-order__number" :class="{ '-volume': numbering }" aria-hidden="true">
                    {{ numbering ? $t('metadata.volumeShort', { n: formatVolume(start + index) }) : index + 1 }}
                </span>
                <BookCover :book="book" variant="row" class="batch-order__cover" />
                <span class="batch-order__text">
                    <span class="batch-order__title" :title="titleOf(book)">{{ titleOf(book) }}</span>
                    <span class="batch-order__detail">{{ detailOf(book) }}</span>
                </span>
            </li>
        </ol>
        <!-- Says where a book went after a move from the keyboard. -->
        <p class="sr-only" role="status">{{ announcement }}</p>
    </div>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import type { Book } from '@/types'
    import { displayAuthor, displayTitle, seriesLabel } from '@/utils/bookDisplay'
    import { uniqueId } from '@/utils/dom'
    import BookCover from './BookCover.vue'
    import { moved } from './collection'

    /**
     * The books of a batch edit, in an order the user chooses: each row
     * shows its place (or, with `numbering`, the volume it will get: `start`,
     * `start + 1`…), the cover, the title, and the author and current series.
     *
     * Reordering: drag a row by its handle (pointer events, not HTML drag
     * and drop, which the window's file drop disables on Windows), or focus
     * a handle and press ↑ / ↓ (one place), Home / End (first / last); the
     * move is announced and the handle keeps the focus. `v-model` holds the
     * order.
     */
    export default defineComponent({
        name: 'BatchOrderList',

        components: { BookCover },

        props: {
            modelValue: { type: Array as PropType<Book[]>, required: true },
            /** Show the volume each book gets instead of its place. */
            numbering:  { type: Boolean, default: false },
            /** The first volume number, with `numbering`. */
            start:      { type: Number, default: 1 },
            disabled:   { type: Boolean, default: false },
        },

        emits: {
            'update:modelValue': (_books: Book[]) => true,
        },

        data() {
            return {
                hintId: uniqueId('batch-order'),
                draggingId: null as string | null,
                announcement: '',
            }
        },

        beforeUnmount() {
            this.stopDrag()
        },

        methods: {
            titleOf(book: Book): string {
                return displayTitle(book, this.$t)
            },

            /** "Frank Herbert · Dune #1". */
            detailOf(book: Book): string {
                const series = seriesLabel(book)
                return series ? `${displayAuthor(book, this.$t)} · ${series}` : displayAuthor(book, this.$t)
            },

            /** "2", or "2.5" for a half volume (never a long float). */
            formatVolume(n: number): string {
                return Number.isInteger(n) ? String(n) : String(Math.round(n * 100) / 100)
            },

            move(from: number, to: number) {
                if (from === to || to < 0 || to >= this.modelValue.length) { return }
                this.$emit('update:modelValue', moved(this.modelValue, from, to))
            },

            // ── Keyboard ────────────────────────────────────────────────────

            onHandleKey(e: KeyboardEvent, index: number) {
                const last = this.modelValue.length - 1
                const target = e.key === 'ArrowUp' ? index - 1
                    : e.key === 'ArrowDown' ? index + 1
                        : e.key === 'Home' ? 0
                            : e.key === 'End' ? last
                                : null
                if (target === null) { return }
                e.preventDefault()
                if (target < 0 || target > last || target === index) { return }
                const book = this.modelValue[index]
                this.move(index, target)
                this.announcement = this.$t('metadata.bookMoved', { title: this.titleOf(book), n: target + 1, total: last + 1 })
                // The rows are re-rendered in their new order: follow the book.
                void this.$nextTick(() => this.handleAt(target)?.focus())
            },

            handleAt(index: number): HTMLButtonElement | null {
                const list = this.$refs.list as HTMLElement | undefined
                return list?.querySelectorAll<HTMLButtonElement>('.batch-order__handle')[index] ?? null
            },

            // ── Pointer ─────────────────────────────────────────────────────

            /** Starts dragging `id`'s row: it follows the pointer across the others. */
            startDrag(e: PointerEvent, id: string) {
                if (this.disabled || e.button !== 0) { return }
                e.preventDefault()
                this.draggingId = id
                window.addEventListener('pointermove', this.onDragMove)
                window.addEventListener('pointerup', this.stopDrag)
                window.addEventListener('pointercancel', this.stopDrag)
            },

            /** Moves the dragged book to the row under the pointer (by the rows' middles). */
            onDragMove(e: PointerEvent) {
                const list = this.$refs.list as HTMLElement | undefined
                if (!list || this.draggingId === null) { return }
                const from = this.modelValue.findIndex(b => b.id === this.draggingId)
                const rows = [...list.querySelectorAll<HTMLElement>('.batch-order__item')]
                let to = rows.length - 1
                for (let i = 0; i < rows.length; i++) {
                    const box = rows[i].getBoundingClientRect()
                    if (e.clientY < box.top + box.height / 2) {
                        to = i
                        break
                    }
                }
                if (from !== -1) { this.move(from, to) }
            },

            stopDrag() {
                this.draggingId = null
                window.removeEventListener('pointermove', this.onDragMove)
                window.removeEventListener('pointerup', this.stopDrag)
                window.removeEventListener('pointercancel', this.stopDrag)
            },
        },
    })
</script>

<style lang="scss" scoped>
.batch-order {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
}

.batch-order__hint {
    font-size: var(--font-size-xs);
    color: var(--text-muted);
}

.batch-order__list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-height: 0;
    overflow-y: auto;
    padding-right: 2px;
}

.batch-order__item {
    display: grid;
    grid-template-columns: auto minmax(26px, auto) 30px minmax(0, 1fr);
    align-items: center;
    gap: var(--space-2);
    padding: 5px 8px 5px 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--bg-elevated) 70%, transparent);
    transition: border-color var(--transition), box-shadow var(--transition), background var(--transition);

    &.-dragging {
        border-color: var(--accent);
        background: var(--accent-muted);
        box-shadow: var(--shadow);
    }
}

.batch-order__handle {
    display: grid;
    place-items: center;
    width: 24px;
    height: 32px;
    border-radius: var(--radius-xs);
    color: var(--text-muted);
    cursor: grab;
    touch-action: none;

    &:hover:not(:disabled) { color: var(--text-primary); background: var(--bg-hover); }
    &:active { cursor: grabbing; }
    &:disabled { cursor: default; opacity: .5; }
}

.batch-order__number {
    white-space: nowrap;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    font-variant-numeric: tabular-nums;
    text-align: center;
    color: var(--text-secondary);

    // The volume the book will get: in the accent, like a series label.
    &.-volume { color: var(--accent-text); }
}

.batch-order__cover {
    width: 30px;
}

.batch-order__text {
    display: flex;
    flex-direction: column;
    min-width: 0;
}

.batch-order__title {
    overflow: hidden;
    font-family: var(--font-serif);
    font-size: calc(var(--font-size-sm) * var(--serif-scale));
    font-weight: var(--serif-weight);
    line-height: 1.25;
    color: var(--text-primary);
    text-overflow: ellipsis;
    white-space: nowrap;
}

.batch-order__detail {
    overflow: hidden;
    font-size: var(--font-size-xs);
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
}
</style>
