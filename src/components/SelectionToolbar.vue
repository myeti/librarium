<template>
    <div class="selection-toolbar">
        <div class="selection-toolbar__info">
            <IconButton icon="x" :label="$t('collection.clearSelection')" @click="libraryStore.clearSelection()" />
            <span class="selection-toolbar__count" role="status">{{ countText }}</span>
            <BaseButton v-if="!allVisibleSelected" variant="ghost" size="sm" @click="libraryStore.selectAll()">
                {{ $t('selection.selectAll') }}
            </BaseButton>
        </div>
        <div class="selection-toolbar__actions">
            <!-- Library actions, on the selected library books. -->
            <template v-if="selectedBookIds.length > 0">
                <BaseButton size="sm" icon="edit-2" :title="$t('selection.editMeta')" @click="bookActionsStore.edit(libraryStore.selectedBooks)">
                    {{ $t('selection.editMeta') }}
                </BaseButton>
                <BaseButton size="sm" icon="zap" :title="$t('selection.tidyUp')" :loading="bookActionsStore.tidying" @click="bookActionsStore.askBatchTidyup()">
                    {{ bookActionsStore.tidying ? $t('selection.tidying') : $t('selection.tidyUp') }}
                </BaseButton>
                <BaseButton
                    size="sm"
                    icon="send"
                    :title="$t('selection.sendTo', { device: deviceName })"
                    :disabled="!deviceStore.isConnected"
                    @click="bookActionsStore.send(libraryStore.selectedBooks)"
                >
                    {{ $t('selection.sendTo', { device: deviceName }) }}
                </BaseButton>
            </template>
            <!-- Files only on the e-reader: they can be copied into the library. -->
            <BaseButton v-if="selectedFiles.length > 0" size="sm" icon="download" :title="$t('shelf.addToLibrary')" @click="bookActionsStore.addToLibrary(selectedFiles)">
                {{ $t('shelf.addToLibrary') }}
            </BaseButton>
            <BaseButton
                v-if="selectedOnDevice.length > 0"
                size="sm"
                variant="danger"
                icon="x-circle"
                :title="$t('shelf.removeFromDevice', { device: deviceName })"
                @click="bookActionsStore.requestRemoveFromDevice(selectedOnDevice)"
            >
                {{ $t('shelf.removeFromDevice', { device: deviceName }) }}
            </BaseButton>
            <BaseButton
                v-if="selectedBookIds.length > 0"
                size="sm"
                variant="danger"
                icon="trash-2"
                :title="$t('selection.delete')"
                @click="bookActionsStore.requestDelete(selectedBookIds, 'selection')"
            >
                {{ $t('selection.delete') }}
            </BaseButton>
            <DropdownMenu v-if="selectedBookIds.length > 0" :label="$t('collection.moreSelectionActions')" placement="bottom-end">
                <template #trigger="{ toggle, attrs }">
                    <IconButton icon="more-vertical" :label="$t('collection.moreSelectionActions')" v-bind="attrs" @click="toggle" />
                </template>
                <!-- Keeps the files: the books leave the library only. -->
                <MenuItem icon="minus-circle" @click="bookActionsStore.requestForget(selectedBookIds, 'selection')">
                    {{ $t('selection.forget') }}
                </MenuItem>
            </DropdownMenu>
        </div>
    </div>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { useLibraryStore } from '@/stores/library'
    import { useDeviceStore } from '@/stores/device'
    import { useBookActionsStore } from '@/stores/bookActions'
    import { deviceLabel } from '@/utils/device'
    import type { ShelfItem } from '@/services/shelf'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import IconButton from '@/components/ui/IconButton.vue'
    import DropdownMenu from '@/components/ui/DropdownMenu.vue'
    import MenuItem from '@/components/ui/MenuItem.vue'

    /**
     * The library's toolbar while books are selected: it takes the filter
     * bar's place (same height, so nothing moves) and offers what can be done
     * with all of them at once. Escape, or ×, clears the selection.
     *
     * The actions are the book-actions store's (each destructive one asks
     * first, and only reaches the books currently visible). Each applies to
     * the items it makes sense for: the library actions to the library
     * books, "Add to library" to the files only on the e-reader, "Remove
     * from Kindle" to everything on it; a button shows only when it has
     * something to act on. On a narrow
     * window the button labels give way to their icons (still announced:
     * the label is visually hidden, not removed).
     */
    export default defineComponent({
        name: 'SelectionToolbar',

        components: { BaseButton, IconButton, DropdownMenu, MenuItem },

        computed: {
            ...mapStores(useLibraryStore, useDeviceStore, useBookActionsStore),

            /** The selected library books' ids (the library actions apply to those). */
            selectedBookIds(): string[] {
                return this.libraryStore.selectedBooks.map(b => b.id)
            },

            /** The selected files that are only on the e-reader. */
            selectedFiles(): ShelfItem[] {
                return this.libraryStore.selectedItems.filter(item => item.presence === 'deviceOnly')
            },

            /** The selected items with files on the e-reader. */
            selectedOnDevice(): ShelfItem[] {
                return this.libraryStore.selectedItems.filter(item => item.copies.length > 0)
            },

            countText(): string {
                return this.$t('selection.count', { n: this.libraryStore.selectedItems.length })
            },

            allVisibleSelected(): boolean {
                const selected = this.libraryStore.selectedIds
                return this.libraryStore.visibleItems.every(item => selected.has(item.id))
            },

            deviceName(): string {
                return deviceLabel(this.deviceStore.device)
            },
        },
    })
</script>

<style lang="scss" scoped>
.selection-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    height: 100%;
    padding: 0 28px 0 22px;
    container: selection-toolbar / inline-size;
    animation: selection-in var(--duration) var(--ease-out);
}

.selection-toolbar__info,
.selection-toolbar__actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
}

.selection-toolbar__count {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
    white-space: nowrap;
}

// Narrow: icons only; the labels stay for screen readers.
@container selection-toolbar (max-width: 760px) {
    .selection-toolbar__actions :deep(.ui-button__label) {
        position: absolute;
        width: 1px;
        height: 1px;
        overflow: hidden;
        clip: rect(0, 0, 0, 0);
        white-space: nowrap;
    }
}

@keyframes selection-in {
    from { opacity: 0; transform: translateY(-4px); }
    to   { opacity: 1; transform: none; }
}
</style>
