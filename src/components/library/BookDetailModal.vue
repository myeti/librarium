<template>
    <BaseModal
        bare
        size="xl"
        class="book-detail"
        :labelledby="titleId"
        @close="$emit('close')"
        @keydown="onKeydown"
    >
        <div class="book-detail__layout">
            <!-- The book itself: its cover, and what to do with it. -->
            <aside class="book-detail__aside">
                <BookCover :book="book" :file="file" variant="panel" :lazy="false" class="book-detail__cover" />
                <div class="book-detail__primary">
                    <template v-if="book">
                        <BaseButton variant="primary" icon="send" block :disabled="!deviceStore.isConnected" @click="bookActionsStore.send([book])">
                            {{ $t('detail.sendTo', { device: deviceName }) }}
                        </BaseButton>
                        <BaseButton v-if="isReadable" icon="book-open" block @click="openReader">{{ $t('detail.read') }}</BaseButton>
                    </template>
                    <template v-else>
                        <BaseButton variant="primary" icon="download" block :loading="adding" @click="bookActionsStore.addToLibrary([item])">
                            {{ $t('shelf.addToLibrary') }}
                        </BaseButton>
                        <BaseButton variant="danger" icon="x-circle" block @click="bookActionsStore.requestRemoveFromDevice([item])">
                            {{ $t('shelf.removeFromDevice', { device: deviceName }) }}
                        </BaseButton>
                    </template>
                </div>
            </aside>

            <div class="book-detail__main">
                <!-- Star, previous / next book, close. -->
                <div class="book-detail__bar">
                    <IconButton
                        v-if="book"
                        icon="star"
                        class="book-detail__star"
                        :pressed="book.favorite === true"
                        :label="$t('favorites.toggle')"
                        @click="bookActionsStore.toggleFavorite([book])"
                    />
                    <span class="book-detail__nav">
                        <IconButton icon="chevron-left" :label="$t('detail.previousBook')" :disabled="!hasPrevious" @click="$emit('step', -1)" />
                        <span v-if="index >= 0" class="book-detail__position">{{ $t('detail.position', { n: index + 1, total }) }}</span>
                        <IconButton icon="chevron-right" :label="$t('detail.nextBook')" :disabled="!hasNext" @click="$emit('step', 1)" />
                    </span>
                    <IconButton icon="x" :label="$t('common.close')" @click="$emit('close')" />
                </div>

                <div class="book-detail__content">
                    <template v-if="book">
                        <BookDetailHeader :book="book" :title-id="titleId" :device-name="onDevice ? deviceName : ''" />
                        <BookMetadataTable :book="book" :progress="item.progress" />
                        <p v-if="book.metadata.description" class="book-detail__description">{{ book.metadata.description }}</p>
                    </template>
                    <DeviceFileDetails v-else :item="item" :title-id="titleId" />
                </div>

                <footer v-if="book" class="book-detail__actions">
                    <BookDetailActions :book="book" />
                </footer>
            </div>
        </div>

        <ReaderModal v-if="readingOpen && book" :book="book" @close="readingOpen = false" />
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import type { Book, DeviceBook } from '@/types'
    import { analytics } from '@/services/analytics'
    import type { ShelfItem } from '@/services/shelf'
    import { useBookActionsStore } from '@/stores/bookActions'
    import { useDeviceStore } from '@/stores/device'
    import { useShortcutsStore } from '@/stores/shortcuts'
    import { deviceLabel } from '@/utils/device'
    import { uniqueId } from '@/utils/dom'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import IconButton from '@/components/ui/IconButton.vue'
    import ReaderModal from '@/components/ReaderModal.vue'
    import BookCover from './BookCover.vue'
    import BookDetailHeader from './BookDetailHeader.vue'
    import BookMetadataTable from './BookMetadataTable.vue'
    import BookDetailActions from './BookDetailActions.vue'
    import DeviceFileDetails from './DeviceFileDetails.vue'

    const READABLE_FORMATS: readonly string[] = ['epub', 'pdf', 'mobi', 'azw3']

    /**
     * A book's details, in a dialog over the library (the books don't move
     * aside): the cover on the left with what to do with the book under it
     * (send it to the e-reader, read it), the details on the right (title,
     * author, badges, facts, description) under a bar with the star,
     * previous / next book and close, and the book's other actions at the
     * bottom (BookDetailActions). A file only on the e-reader gets the same
     * frame: add it to the library, or remove it from the device.
     *
     * Keys (BaseModal keeps them in the dialog): Escape closes; ← → show the
     * previous / next book of the library as shown (`step`); Delete / ⌫
     * moves the book to the Trash after confirming (a device file: offers
     * to remove it from the device); the shortcuts that work over a dialog
     * (⌘I: close the details) go to the shortcuts store. Focus starts on the
     * title, and goes back to the book when the dialog closes (LibraryView).
     */
    export default defineComponent({
        name: 'BookDetailModal',

        components: {
            BaseModal, BaseButton, IconButton, ReaderModal, BookCover, BookDetailHeader, BookMetadataTable,
            BookDetailActions, DeviceFileDetails,
        },

        props: {
            item: { type: Object as PropType<ShelfItem>, required: true },
            /** The item's place in the library as shown (-1: not in it). */
            index: { type: Number, default: -1 },
            /** How many items are shown. */
            total: { type: Number, default: 0 },
        },

        emits: {
            close: () => true,
            /** Show the previous (-1) or next (1) item. */
            step: (_delta: number) => true,
        },

        data() {
            return {
                titleId: uniqueId('book-title'),
                readingOpen: false,
            }
        },

        computed: {
            ...mapStores(useBookActionsStore, useDeviceStore, useShortcutsStore),

            book(): Book | null { return this.item.book },

            /** A file only on the e-reader (no library book). */
            file(): DeviceBook | null { return this.item.book ? null : this.item.copies[0] ?? null },

            deviceName(): string { return deviceLabel(this.deviceStore.device) },

            /** A library book with a copy on the e-reader. */
            onDevice(): boolean { return this.item.presence === 'both' },

            isReadable(): boolean {
                return !!this.book && READABLE_FORMATS.includes(this.book.format.toLowerCase())
            },

            adding(): boolean {
                return !!this.file && this.bookActionsStore.addingToLibrary.includes(this.file.path)
            },

            hasPrevious(): boolean { return this.index > 0 },
            hasNext(): boolean { return this.index >= 0 && this.index < this.total - 1 },
        },

        watch: {
            // Another book: the reader doesn't carry over.
            'item.id'() {
                this.readingOpen = false
            },
        },

        mounted() {
            // Focus on the title: read out first, and no control looks pressed.
            void this.$nextTick(() => document.getElementById(this.titleId)?.focus({ preventScroll: true }))
        },

        methods: {
            openReader() {
                if (!this.book) { return }
                analytics.track('reader_open', { format: this.book.format })
                this.readingOpen = true
            },

            onKeydown(e: KeyboardEvent) {
                const target = e.target as HTMLElement | null
                // A menu, a text field: their keys are theirs.
                if (target?.closest('[role="menu"], input, textarea, select, [contenteditable="true"]')) { return }
                const plain = !e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey
                if (plain && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
                    e.preventDefault()
                    const delta = e.key === 'ArrowLeft' ? -1 : 1
                    if (delta < 0 ? this.hasPrevious : this.hasNext) { this.$emit('step', delta) }
                    return
                }
                if (plain && (e.key === 'Delete' || e.key === 'Backspace')) {
                    e.preventDefault()
                    if (this.book) {
                        this.bookActionsStore.requestDelete([this.book.id], 'single')
                    } else {
                        this.bookActionsStore.requestRemoveFromDevice([this.item])
                    }
                    return
                }
                // ⌘I and the other shortcuts that work over a dialog.
                this.shortcutsStore.handleKeydown(e)
            },
        },
    })
</script>

<style lang="scss" scoped>
// The panel (BaseModal's, `bare`): a fixed height, so the footer stays put
// while the details scroll. (Global: it is BaseModal's element, outside
// this component's scope.)
:global(.ui-modal__panel.book-detail) {
    height: min(720px, 100%);
    container: book-detail / inline-size;
    background:
        radial-gradient(90% 60% at 18% 0%, color-mix(in srgb, var(--accent) 10%, transparent), transparent 70%),
        var(--bg-modal);
}

.book-detail__layout {
    display: grid;
    grid-template-columns: 280px minmax(0, 1fr);
    height: 100%;
    min-height: 0;
}

// ── The cover column ───────────────────────────────────────────────────────

.book-detail__aside {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    min-height: 0;
    padding: var(--space-6) var(--space-5) var(--space-5) var(--space-6);
    overflow-y: auto;
    // A quiet spine down the middle: the hairline, fading at both ends.
    background: linear-gradient(180deg, transparent, var(--hairline) 15%, var(--hairline) 85%, transparent) right / 1px 100% no-repeat;
}

// The cover stands like a book on a stand: a deep shadow, a gilt edge.
.book-detail__cover {
    box-shadow:
        0 2px 2px rgba(0,0,0,.2),
        0 18px 40px -10px rgba(0,0,0,.55),
        0 0 0 1px color-mix(in srgb, var(--hairline) 60%, transparent);
}

.book-detail__primary {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
}

// ── The details column ─────────────────────────────────────────────────────

.book-detail__main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
}

.book-detail__bar {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-1);
    flex-shrink: 0;
    padding: var(--space-3) var(--space-3) 0;
}

// A favorite's star: filled, in the accent.
.book-detail__star[aria-pressed="true"] {
    color: var(--accent-text);

    :deep(svg) { fill: currentColor; }
}

.book-detail__nav {
    display: inline-flex;
    align-items: center;
    margin: 0 var(--space-2);
}

.book-detail__position {
    min-width: 52px;
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
    text-align: center;
    color: var(--text-muted);
}

.book-detail__content {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    min-height: 0;
    padding: var(--space-2) var(--space-6) var(--space-5);
    overflow-y: auto;
    overscroll-behavior: contain;
}

.book-detail__description {
    margin: 0;
    max-width: 68ch;
    font-family: var(--font-serif);
    font-size: calc(var(--font-size-md) * var(--serif-scale));
    line-height: 1.6;
    color: var(--text-secondary);
    white-space: pre-line;
    overflow-wrap: anywhere;
}

.book-detail__actions {
    flex-shrink: 0;
    padding: var(--space-3) var(--space-6) var(--space-4);
    background: linear-gradient(90deg, transparent, var(--hairline) 15%, var(--hairline) 85%, transparent) top / 100% 1px no-repeat;
}

// A narrow window: the cover above the details, everything in one scroll.
@container book-detail (max-width: 640px) {
    .book-detail__layout {
        grid-template-columns: 1fr;
        grid-template-rows: auto auto;
        overflow-y: auto;
    }

    .book-detail__aside {
        flex-direction: row;
        align-items: flex-end;
        overflow: visible;
        padding: var(--space-5) var(--space-5) 0;
        background: none;
    }

    .book-detail__cover { width: 140px; flex-shrink: 0; }
    .book-detail__primary { flex: 1; }
    .book-detail__bar { position: absolute; top: 0; right: 0; }
    .book-detail__content { overflow: visible; }
}
</style>
