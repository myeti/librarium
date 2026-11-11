<template>
    <MetadataModal
        v-if="actions.editing.length > 0"
        :books="actions.editing"
        :submit="actions.saveMetadata"
        @close="actions.closeEditor()"
    />

    <TidyupReviewModal
        v-if="actions.review"
        :book="actions.review.book"
        :metadata="actions.review.metadata"
        :source="actions.review.source"
        :cover-url="actions.review.coverUrl"
        :confident="actions.review.confident"
        :submit="actions.applyReview"
        @close="actions.closeReview()"
    />

    <ConfirmDialog
        v-if="actions.confirmingBatchTidyup"
        :title="$t('tidyup.batchConfirmTitle', { n: libraryStore.selectedBooks.length })"
        :message="$t('tidyup.batchConfirmBody', { sources: tidyupSources })"
        :details="$t('tidyup.batchConfirmDetails')"
        :confirm-label="$t('selection.tidyUp')"
        @confirm="actions.runBatchTidyup()"
        @cancel="actions.cancelBatchTidyup()"
    />

    <!-- Every delete ends here: books go to the Trash, after confirmation. -->
    <ConfirmDialog
        v-if="actions.pendingDelete"
        danger
        :title="actions.deleteTitle"
        :message="$t('library.trashConfirmBody')"
        :details="$t('library.trashConfirmDetails')"
        :confirm-label="$t('library.trashConfirm')"
        :busy="actions.deleting"
        :error="actions.deleteError"
        @confirm="actions.confirmDelete()"
        @cancel="actions.cancelDelete()"
    />

    <!-- "Remove from the e-reader": for good (e-readers have no Trash); the library keeps its books. -->
    <ConfirmDialog
        v-if="actions.pendingDeviceRemoval"
        danger
        :title="actions.deviceRemovalTitle"
        :message="$t('shelf.removeConfirmBody')"
        :confirm-label="$t('shelf.removeConfirm')"
        :busy="actions.removingFromDevice"
        :error="actions.deviceRemovalError"
        @confirm="actions.confirmRemoveFromDevice()"
        @cancel="actions.cancelRemoveFromDevice()"
    />

    <!-- "Remove from library": the files stay on disk, so not a danger dialog. -->
    <ConfirmDialog
        v-if="actions.pendingForget"
        :title="actions.forgetTitle"
        :message="$t('library.forgetConfirmBody')"
        :details="$t('library.forgetConfirmDetails')"
        :confirm-label="$t('library.forgetConfirm')"
        :busy="actions.forgetting"
        :error="actions.forgetError"
        @confirm="actions.confirmForget()"
        @cancel="actions.cancelForget()"
    />

    <!-- Some of the books are on the device already. -->
    <ConfirmDialog
        v-if="actions.pendingSend.length > 0"
        :title="$t('deviceLibrary.confirmSend')"
        :message="$t('deviceLibrary.confirmSendBody', { n: actions.pendingSendOnDevice })"
        :confirm-label="$t('deviceLibrary.sendAnyway')"
        @confirm="actions.confirmSend()"
        @cancel="actions.cancelSend()"
    />

    <!-- Confirms and removes extra copies itself; closes once no group is left. -->
    <DuplicatesModal
        v-if="actions.duplicatesOpen"
        :groups="libraryStore.duplicateGroups"
        @close="actions.closeDuplicates()"
    />
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { useLibraryStore } from '@/stores/library'
    import { useBookActionsStore } from '@/stores/bookActions'
    import { availableTidyupSources, tidyupSourceLabel } from '@/services/tidyup'
    import MetadataModal from '@/components/MetadataModal.vue'
    import TidyupReviewModal from '@/components/TidyupReviewModal.vue'
    import DuplicatesModal from '@/components/DuplicatesModal.vue'
    import ConfirmDialog from '@/components/ConfirmDialog.vue'

    /**
     * The library screen's dialogs, opened by the book-actions store
     * (stores/bookActions.ts): the metadata editor, the tidy-up review and
     * batch confirmation, the Trash / "remove from library" / "send anyway"
     * confirmations and the duplicates review. Each shows while the store
     * has something pending, and calls the store back.
     */
    export default defineComponent({
        name: 'LibraryDialogs',

        components: { MetadataModal, TidyupReviewModal, DuplicatesModal, ConfirmDialog },

        computed: {
            ...mapStores(useLibraryStore, useBookActionsStore),

            /** Short name for the template. */
            actions(): ReturnType<typeof useBookActionsStore> {
                return this.bookActionsStore
            },

            /** "Google Books, Open Library": the services batch tidy-up asks, in order. */
            tidyupSources(): string {
                return availableTidyupSources().map(tidyupSourceLabel).join(', ')
            },
        },
    })
</script>
