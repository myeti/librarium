<template>
    <div class="library-list" :class="{ '-with-progress': showProgress }">
        <BookCollection
            ref="collection"
            :items="items"
            :layout="layout"
            :active-id="activeId"
            :label="$t('collection.label')"
            :item-label="itemLabel"
            @update:active-id="$emit('update:activeId', $event)"
            @open="$emit('open', $event)"
            @menu="$emit('menu', $event)"
        >
            <template #header>
                <div class="list-header">
                    <span class="list-header__cell -check">
                        <input
                            type="checkbox"
                            class="list-header__check"
                            :checked="allSelected"
                            :indeterminate="someSelected"
                            :aria-label="$t('collection.selectAllVisible')"
                            :disabled="items.length === 0"
                            @change="toggleAll"
                        />
                    </span>
                    <span class="list-header__cell"></span>
                    <span class="list-header__cell">{{ $t('library.colTitle') }}</span>
                    <span class="list-header__cell">{{ $t('library.colAuthor') }}</span>
                    <span class="list-header__cell -category">{{ $t('library.colCategory') }}</span>
                    <span class="list-header__cell -language">{{ $t('library.colLang') }}</span>
                    <span class="list-header__cell">{{ $t('library.colFormat') }}</span>
                    <span v-if="showProgress" class="list-header__cell -progress">{{ $t('collection.colProgress') }}</span>
                    <span class="list-header__cell"></span>
                </div>
            </template>
            <template #default="{ item, selected, active }">
                <BookRow
                    :item="item"
                    :selected="selected"
                    :active="active"
                    :selection-mode="selectionMode"
                    :show-progress="showProgress"
                    :device-name="deviceName"
                />
            </template>
        </BookCollection>
    </div>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import type { ListLayout } from '@/utils/virtualWindow'
    import type { ShelfItem } from '@/services/shelf'
    import { useLibraryStore } from '@/stores/library'
    import { itemOptionLabel } from './labels'
    import { LIST_LAYOUT, type CollectionMenuRequest } from './collection'
    import BookCollection from './BookCollection.vue'
    import BookRow from './BookRow.vue'

    /**
     * The shelf as a list of rows with columns, under a sticky header
     * whose checkbox selects (or deselects) every book shown. Focus, keys,
     * selection and virtual scrolling: BookCollection. Sizes: LIST_LAYOUT.
     *
     * The header and the rows share one grid template (`--list-columns`);
     * on a narrow list the category and language columns are dropped.
     */
    export default defineComponent({
        name: 'LibraryList',

        components: { BookCollection, BookRow },

        props: {
            items: { type: Array as PropType<ShelfItem[]>, required: true },
            activeId: { type: String as PropType<string | null>, default: null },
            selectionMode: { type: Boolean, default: false },
            /** Show the progress column (the connected e-reader reports progress). */
            showProgress: { type: Boolean, default: false },
            /** The connected e-reader's name, for the location badges ("Kindle"). */
            deviceName: { type: String, default: '' },
        },

        emits: {
            'update:activeId': (_id: string | null) => true,
            open: (_item: ShelfItem) => true,
            menu: (_request: CollectionMenuRequest) => true,
        },

        computed: {
            ...mapStores(useLibraryStore),

            layout(): Readonly<ListLayout> { return LIST_LAYOUT },

            selectedCount(): number {
                const selected = this.libraryStore.selectedIds
                return this.items.reduce((n, item) => n + (selected.has(item.id) ? 1 : 0), 0)
            },

            allSelected(): boolean {
                return this.items.length > 0 && this.selectedCount === this.items.length
            },

            someSelected(): boolean {
                return this.selectedCount > 0 && !this.allSelected
            },
        },

        methods: {
            itemLabel(item: ShelfItem): string {
                return itemOptionLabel(item, this.deviceName)
            },

            toggleAll() {
                if (this.allSelected) { this.libraryStore.clearSelection() } else { this.libraryStore.selectAll() }
            },

            focusActive() {
                (this.$refs.collection as { focusActive(): void } | undefined)?.focusActive()
            },
        },
    })
</script>

<style lang="scss" scoped>
.library-list {
    height: 100%;
    container: library-list / inline-size;
}

// One template for the header and every row (BookRow reads it):
// check, cover, title, author, category, language, format, [progress,] actions.
.library-list :deep(.book-collection) {
    --list-columns: 28px 36px minmax(0, 4fr) minmax(0, 2fr) minmax(0, 2fr) 48px 64px 64px;
}

.library-list.-with-progress :deep(.book-collection) {
    --list-columns: 28px 36px minmax(0, 4fr) minmax(0, 2fr) minmax(0, 2fr) 48px 64px 44px 64px;
}

@container library-list (max-width: 720px) {
    .library-list :deep(.book-collection) {
        --list-columns: 28px 36px minmax(0, 3fr) minmax(0, 2fr) 64px 64px;
    }

    .library-list.-with-progress :deep(.book-collection) {
        --list-columns: 28px 36px minmax(0, 3fr) minmax(0, 2fr) 64px 44px 64px;
    }

    .library-list :deep(.-category),
    .library-list :deep(.-language) {
        display: none;
    }
}

.list-header {
    position: sticky;
    top: 0;
    z-index: var(--z-raised);
    display: grid;
    grid-template-columns: var(--list-columns);
    align-items: center;
    column-gap: var(--space-3);
    height: 32px;
    padding: 0 calc(20px + var(--space-2)) 0 20px;
    background: var(--bg-header-sticky);
    border-bottom: 1px solid var(--border);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
}

.list-header__cell {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;

    &.-check {
        display: flex;
        justify-content: center;
    }

    &.-progress { text-align: right; }
}

.list-header__check {
    width: 16px;
    height: 16px;
    accent-color: var(--accent-strong);
    cursor: pointer;
}
</style>
