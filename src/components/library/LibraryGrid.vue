<template>
    <BookCollection
        ref="collection"
        class="library-grid"
        :items="items"
        :layout="layout"
        :active-id="activeId"
        :label="$t('collection.label')"
        :item-label="itemLabel"
        @update:active-id="$emit('update:activeId', $event)"
        @open="$emit('open', $event)"
        @menu="$emit('menu', $event)"
    >
        <template #default="{ item, selected, active }">
            <BookCard
                :item="item"
                :selected="selected"
                :active="active"
                :selection-mode="selectionMode"
                :device-name="deviceName"
            />
        </template>
    </BookCollection>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import type { GridLayout } from '@/utils/virtualWindow'
    import type { ShelfItem } from '@/services/shelf'
    import { itemOptionLabel } from './labels'
    import { GRID_LAYOUT, type CollectionMenuRequest } from './collection'
    import BookCollection from './BookCollection.vue'
    import BookCard from '@/components/BookCard.vue'

    /**
     * The shelf as a grid of covers, titles always visible under them.
     * Focus, keys, selection and virtual scrolling: BookCollection. Sizes:
     * GRID_LAYOUT.
     */
    export default defineComponent({
        name: 'LibraryGrid',

        components: { BookCollection, BookCard },

        props: {
            items: { type: Array as PropType<ShelfItem[]>, required: true },
            activeId: { type: String as PropType<string | null>, default: null },
            /** Some book is selected: every card shows its checkbox. */
            selectionMode: { type: Boolean, default: false },
            /** The connected e-reader's name, for the location badges ("Kindle"). */
            deviceName: { type: String, default: '' },
        },

        emits: {
            'update:activeId': (_id: string | null) => true,
            open: (_item: ShelfItem) => true,
            menu: (_request: CollectionMenuRequest) => true,
        },

        computed: {
            layout(): Readonly<GridLayout> { return GRID_LAYOUT },
        },

        methods: {
            itemLabel(item: ShelfItem): string {
                return itemOptionLabel(item, this.deviceName)
            },

            /** Puts keyboard focus on the active book. */
            focusActive() {
                (this.$refs.collection as { focusActive(): void } | undefined)?.focusActive()
            },
        },
    })
</script>
