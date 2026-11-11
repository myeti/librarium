<template>
    <DropdownMenu ref="menu" :label="menuLabel">
        <MenuItem v-if="targets.length === 1" icon="sidebar" @click="$emit('details', targets[0])">
            {{ $t('collection.showDetails') }}
        </MenuItem>
        <template v-if="books.length > 0">
            <MenuItem icon="edit-2" :hint="countHint(books.length)" @click="bookActionsStore.edit(books)">
                {{ $t('selection.editMeta') }}
            </MenuItem>
            <MenuItem icon="send" :hint="countHint(books.length)" :disabled="!deviceStore.isConnected" @click="bookActionsStore.send(books)">
                {{ $t('detail.sendTo', { device: deviceName }) }}
            </MenuItem>
            <!-- Unstars only when every one is a favorite (as the star itself). -->
            <MenuItem icon="star" :hint="countHint(books.length)" @click="bookActionsStore.toggleFavorite(books)">
                {{ allFavorites ? $t('favorites.remove') : $t('favorites.add') }}
            </MenuItem>
        </template>
        <MenuItem v-if="files.length > 0" icon="download" :hint="countHint(files.length)" @click="bookActionsStore.addToLibrary(files)">
            {{ $t('shelf.addToLibrary') }}
        </MenuItem>
        <MenuSeparator />
        <!-- Each removal names its place: the e-reader, the library, the Trash. -->
        <MenuItem v-if="onDevice.length > 0" icon="x-circle" danger :hint="countHint(onDevice.length)" @click="bookActionsStore.requestRemoveFromDevice(onDevice)">
            {{ $t('shelf.removeFromDevice', { device: deviceName }) }}
        </MenuItem>
        <template v-if="books.length > 0">
            <MenuItem icon="minus-circle" :hint="countHint(books.length)" @click="bookActionsStore.requestForget(bookIds, origin)">
                {{ $t('library.forgetMenu') }}
            </MenuItem>
            <MenuItem icon="trash-2" danger :hint="countHint(books.length)" @click="bookActionsStore.requestDelete(bookIds, origin)">
                {{ $t('library.trashConfirm') }}
            </MenuItem>
        </template>
    </DropdownMenu>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import type { Book } from '@/types'
    import { useBookActionsStore, type RemovalOrigin } from '@/stores/bookActions'
    import { useDeviceStore } from '@/stores/device'
    import { useLibraryStore } from '@/stores/library'
    import { deviceLabel } from '@/utils/device'
    import { itemTitle, type ShelfItem } from '@/services/shelf'
    import DropdownMenu from '@/components/ui/DropdownMenu.vue'
    import MenuItem from '@/components/ui/MenuItem.vue'
    import MenuSeparator from '@/components/ui/MenuSeparator.vue'
    import type { CollectionMenuRequest } from './collection'

    /**
     * The menu of an item in the grid or list (right-click, the ContextMenu
     * key, Shift+F10), built on DropdownMenu. What it offers depends on
     * where the books are:
     *
     * - library books: edit, send to the e-reader, add to (or remove
     *   from) the favorites, remove from the library, move to the Trash;
     * - files only on the e-reader: add to the library;
     * - anything on the e-reader (in both places, or only there): remove
     *   from the e-reader (the library book, if any, is kept).
     *
     * As in a file manager, right-clicking an item that is part of a
     * selection of several acts on the whole selection: each action takes
     * the items it applies to, and says how many. The actions themselves are
     * the book-actions store's (with their confirmations).
     */
    export default defineComponent({
        name: 'BookContextMenu',

        components: { DropdownMenu, MenuItem, MenuSeparator },

        emits: {
            /** "Show details": open the item in the detail panel. */
            details: (_item: ShelfItem) => true,
        },

        data(): { targets: ShelfItem[], origin: RemovalOrigin } {
            return { targets: [], origin: 'single' }
        },

        computed: {
            ...mapStores(useBookActionsStore, useDeviceStore, useLibraryStore),

            /** The library books among the targets. */
            books(): Book[] {
                return this.targets.flatMap(item => item.book ? [item.book] : [])
            },

            bookIds(): string[] {
                return this.books.map(b => b.id)
            },

            /** Every library book among the targets is a favorite. */
            allFavorites(): boolean {
                return this.books.length > 0 && this.books.every(b => b.favorite === true)
            },

            /** The files only on the e-reader among the targets. */
            files(): ShelfItem[] {
                return this.targets.filter(item => item.presence === 'deviceOnly')
            },

            /** The targets with files on the e-reader (in both places, or only there). */
            onDevice(): ShelfItem[] {
                return this.targets.filter(item => item.copies.length > 0)
            },

            deviceName(): string {
                return deviceLabel(this.deviceStore.device)
            },

            menuLabel(): string {
                if (this.targets.length === 1) {
                    return this.$t('collection.bookMenu', { title: itemTitle(this.targets[0], this.$t) })
                }
                return this.$t('collection.selectionMenu', { n: this.targets.length })
            },
        },

        methods: {
            /** "3 books" next to an action when the menu acts on a selection. */
            countHint(n: number): string {
                return this.targets.length > 1 ? this.$t('collection.bookCount', { n }) : ''
            },

            /** Opens the menu for `request.item` (or the selection it belongs to). */
            open(request: CollectionMenuRequest) {
                const selection = this.libraryStore.selectedIds
                if (selection.size > 1 && selection.has(request.item.id)) {
                    this.targets = this.libraryStore.selectedItems
                    this.origin = 'selection'
                } else {
                    this.targets = [request.item]
                    this.origin = 'single'
                }
                (this.$refs.menu as { openAt(x: number, y: number, origin: Element | null): void }).openAt(request.x, request.y, request.origin)
            },
        },
    })
</script>
