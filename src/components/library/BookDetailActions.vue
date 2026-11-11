<template>
    <div class="detail-actions">
        <!-- Working on the book: its details, and its file. -->
        <div class="detail-actions__group">
            <BaseButton icon="edit-2" @click="bookActionsStore.edit([book])">{{ $t('detail.edit') }}</BaseButton>
            <BaseButton icon="zap" :loading="bookActionsStore.tidying" @click="bookActionsStore.tidyupOne(book)">
                {{ bookActionsStore.tidying ? $t('detail.tidying') : $t('detail.tidyUp') }}
            </BaseButton>
            <BaseButton icon="folder" @click="reveal">{{ $t('detail.openLocation') }}</BaseButton>
            <DropdownMenu v-if="exportFormats.length > 0" :label="$t('detail.export')" placement="top-start">
                <template #trigger="{ toggle, attrs }">
                    <BaseButton icon="download" v-bind="attrs" @click="toggle">
                        {{ $t('detail.export') }}
                        <vue-feather type="chevron-down" size="14" class="detail-actions__chevron" aria-hidden="true" />
                    </BaseButton>
                </template>
                <MenuItem v-for="fmt in exportFormats" :key="fmt" icon="download" @click="bookActionsStore.exportBook(book, fmt)">
                    {{ $t('detail.exportTo', { format: fmt.toUpperCase() }) }}
                </MenuItem>
            </DropdownMenu>
        </div>

        <!-- The rest, and what removes the book, at the end. -->
        <DropdownMenu :label="$t('detail.moreActions')" placement="top-end">
            <template #trigger="{ toggle, attrs }">
                <BaseButton variant="ghost" icon="more-horizontal" v-bind="attrs" @click="toggle">{{ $t('detail.more') }}</BaseButton>
            </template>
            <MenuItem icon="tag" @click="openCleanRename">{{ $t('detail.cleanRename') }}</MenuItem>
            <MenuItem v-if="isConvertible" icon="refresh-cw" @click="confirmingConvert = true">{{ $t('convert.menuItem') }}</MenuItem>
            <MenuItem icon="rotate-ccw" @click="confirmingReset = true">{{ $t('detail.resetData') }}</MenuItem>
            <MenuItem v-if="isDev && book.format === 'epub'" icon="code" @click="inspectingEpub = true">{{ $t('detail.inspectEpub') }}</MenuItem>
            <MenuSeparator />
            <!-- On the e-reader too: its copies can go (the library book stays). -->
            <MenuItem v-if="shelfItem && shelfItem.copies.length > 0" icon="x-circle" @click="bookActionsStore.requestRemoveFromDevice([shelfItem])">
                {{ $t('shelf.removeFromDevice', { device: deviceName }) }}
            </MenuItem>
            <MenuItem icon="minus-circle" @click="bookActionsStore.requestForget([book.id], 'single')">{{ $t('library.forgetMenu') }}</MenuItem>
            <MenuItem icon="trash-2" danger @click="bookActionsStore.requestDelete([book.id], 'single')">{{ $t('detail.delete') }}</MenuItem>
        </DropdownMenu>

        <EpubConvertModal
            v-if="confirmingConvert"
            :books="[book]"
            @converted="confirmingConvert = false"
            @close="confirmingConvert = false"
        />

        <!-- Reloading replaces what was edited in Librarium: asked first. -->
        <ConfirmDialog
            v-if="confirmingReset"
            :title="$t('detail.resetTitle', { title: displayTitle(book, $t) })"
            :message="$t('detail.resetBody')"
            :confirm-label="$t('detail.resetConfirm')"
            @confirm="resetData"
            @cancel="confirmingReset = false"
        />

        <EpubInspectModal v-if="inspectingEpub" :book-path="book.path" @close="inspectingEpub = false" />

        <ConfirmDialog
            v-if="confirmingRename"
            :title="$t('detail.cleanRenameTitle')"
            :confirm-label="$t('detail.cleanRenameConfirm')"
            :busy="renaming"
            :error="renameError"
            @confirm="doCleanRename"
            @cancel="confirmingRename = false"
        >
            <p class="rename-label">{{ $t('detail.cleanRenameNewName') }}</p>
            <p class="rename-name">{{ cleanFilename }}</p>
            <p class="rename-warning">
                <vue-feather type="alert-triangle" size="14" class="rename-warning__icon" aria-hidden="true" />
                {{ $t('detail.cleanRenameWarning') }}
            </p>
        </ConfirmDialog>
    </div>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import type { Book, ExportFormat } from '@/types'
    import * as api from '@/services/api'
    import { reportError } from '@/services/librarium-api'
    import { useLibraryStore, CONVERTIBLE_FORMATS } from '@/stores/library'
    import { useNotificationsStore } from '@/stores/notifications'
    import { useDeviceStore } from '@/stores/device'
    import { exportFormatsOf, useBookActionsStore } from '@/stores/bookActions'
    import type { ShelfItem } from '@/services/shelf'
    import { displayAuthor, displayTitle } from '@/utils/bookDisplay'
    import { deviceLabel } from '@/utils/device'
    import { errorMessage } from '@/utils/errors'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import DropdownMenu from '@/components/ui/DropdownMenu.vue'
    import MenuItem from '@/components/ui/MenuItem.vue'
    import MenuSeparator from '@/components/ui/MenuSeparator.vue'
    import ConfirmDialog from '@/components/ConfirmDialog.vue'
    import EpubConvertModal from '@/components/EpubConvertModal.vue'
    import EpubInspectModal from '@/components/EpubInspectModal.vue'


    /**
     * The action bar at the bottom of a book's details (BookDetailModal;
     * sending and reading are under the cover). On the left, working on the
     * book: Edit, Find metadata, Open file location, Export ▾ (the formats
     * it can be converted to); on the right, More ▾: rename the file from
     * the metadata, convert to EPUB, reload the details from the file
     * (inspect the EPUB, in development), then what takes the book away:
     * off the e-reader, out of the library, to the Trash.
     *
     * Actions shared with the rest of the library screen (edit, tidy up,
     * export, reset, remove, delete) are the book-actions store's, with
     * their confirmations. The ones only the details offer run here and
     * show their own dialogs (conversion, EPUB inspector, rename) and
     * failures.
     */
    export default defineComponent({
        name: 'BookDetailActions',

        components: {
            BaseButton, DropdownMenu, MenuItem, MenuSeparator, ConfirmDialog, EpubConvertModal, EpubInspectModal,
        },

        props: {
            book: { type: Object as PropType<Book>, required: true },
        },

        data() {
            return {
                confirmingRename: false,
                renaming: false,
                renameError: '',
                confirmingConvert: false,
                inspectingEpub: false,
                confirmingReset: false,
            }
        },

        computed: {
            ...mapStores(useLibraryStore, useNotificationsStore, useDeviceStore, useBookActionsStore),

            /** The book on the shelf: with its copies on the e-reader, if any. */
            shelfItem(): ShelfItem | undefined {
                return this.libraryStore.shelfById.get(this.book.id)
            },

            deviceName(): string {
                return deviceLabel(this.deviceStore.device)
            },

            isConvertible(): boolean {
                return CONVERTIBLE_FORMATS.includes(this.book.format.toLowerCase())
            },

            isDev(): boolean {
                return import.meta.env.DEV
            },

            /**
             * What `export_book` can write this book as: EPUB or AZW3, from
             * an EPUB, MOBI, AZW or AZW3 book (boko can't read PDF or CBZ,
             * and doesn't write PDF), its own format excluded.
             */
            exportFormats(): ExportFormat[] {
                return exportFormatsOf(this.book)
            },

            /** The canonical file name this book would be renamed to. */
            cleanFilename(): string {
                const m = this.book.metadata
                const ext = this.book.format.toLowerCase()
                const safe = (s: string) => s.replace(/[/\\:*?"<>|]/g, '_').trim()
                // Unknown values become the localised placeholder in the name.
                const author = safe(displayAuthor(this.book, this.$t))
                const title = safe(displayTitle(this.book, this.$t))
                if (m.series) {
                    const idx = m.seriesIndex != null ? ` ${m.seriesIndex}` : ''
                    return `${author} - ${safe(m.series)}${idx} - ${title}.${ext}`
                }
                return `${author} - ${title}.${ext}`
            },
        },

        watch: {
            // Another book is shown: its dialogs don't carry over.
            'book.id'() {
                this.confirmingRename = false
                this.renameError = ''
                this.confirmingReset = false
            },
        },

        methods: {
            async reveal() {
                const libraryPath = this.libraryStore.libraryPath
                if (!libraryPath) { return }
                try {
                    // The backend only reveals paths inside the open library.
                    await api.revealInExplorer(libraryPath, this.book.path)
                } catch (e) {
                    this.notificationsStore.push('error', this.$t('errors.revealFailed', { error: errorMessage(e) }))
                }
            },

            openCleanRename() {
                this.renameError = ''
                this.confirmingRename = true
            },

            displayTitle,

            /** "Reload details from the file", once confirmed (the store says how it went). */
            resetData() {
                this.confirmingReset = false
                void this.bookActionsStore.resetData(this.book)
            },

            /**
             * Renames through the library store (id-based; the backend keeps
             * the id, metadata and date added). The dialog stays open on
             * failure and shows why.
             */
            async doCleanRename() {
                this.renaming = true
                this.renameError = ''
                try {
                    await this.libraryStore.renameBook(this.book.id, this.cleanFilename)
                    this.confirmingRename = false
                } catch (e) {
                    console.error('[cleanRename]', e)
                    reportError('RENAME_FAILED', 'details.rename', e)
                    this.renameError = this.$t('errors.renameFailedWithReason', { error: errorMessage(e) })
                } finally {
                    this.renaming = false
                }
            },
        },
    })
</script>

<style lang="scss" scoped>
.detail-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
}

// A longer label (French) wraps the row rather than being cut.
.detail-actions__group {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
    min-width: 0;
}

.detail-actions__chevron {
    margin-left: 2px;
    opacity: .7;
}

.rename-label {
    margin-bottom: 6px;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-bold);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
}

.rename-name {
    margin-bottom: var(--space-4);
    padding: var(--space-2) var(--space-3);
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: var(--font-size-sm);
    color: var(--text-primary);
    word-break: break-all;
}

.rename-warning {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: 10px var(--space-3);
    background: var(--warning-muted);
    border: 1px solid color-mix(in srgb, var(--warning) 20%, transparent);
    border-radius: var(--radius-sm);
    font-size: var(--font-size-xs);
    line-height: var(--line-height);
    color: var(--warning);
}

.rename-warning__icon {
    flex-shrink: 0;
    margin-top: 1px;
}
</style>
