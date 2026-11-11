<template>
    <div class="book-row" :class="{ '-selected': selected, '-active': active, '-selecting': selectionMode }">
        <span class="book-row__cell -check">
            <SelectCheckbox :checked="selected" :label="$t('collection.select', { title })" />
        </span>
        <span class="book-row__cell -cover">
            <BookCover :book="item.book" :file="file" variant="row" />
        </span>
        <span class="book-row__cell -title">
            <span v-if="series" class="book-row__series truncate" :title="series">{{ series }}</span>
            <span class="book-row__title-line">
                <span class="book-row__title truncate" :title="title">{{ title }}</span>
                <FavoriteStar v-if="item.book" class="book-row__star" :book="item.book" variant="row" />
                <!-- Where the book is, when it is on the e-reader (as on the cards). -->
                <span v-if="item.presence !== 'libraryOnly'" class="book-row__where" :title="whereText">
                    <vue-feather v-if="item.presence === 'both'" type="book" size="11" aria-hidden="true" />
                    <vue-feather type="tablet" size="11" aria-hidden="true" />
                    <span class="sr-only">{{ whereText }}</span>
                </span>
            </span>
        </span>
        <span class="book-row__cell -author truncate" :title="author">{{ author }}</span>
        <span class="book-row__cell -category truncate" :title="categories">{{ categories }}</span>
        <span class="book-row__cell -language">{{ fmtLanguageShort(item.book?.metadata.language) }}</span>
        <span class="book-row__cell -format"><Badge :tone="formatTone">{{ format }}</Badge></span>
        <span v-if="showProgress" class="book-row__cell -progress">
            <template v-if="item.progress !== undefined">{{ item.progress }}%</template>
        </span>
        <span class="book-row__cell -actions">
            <template v-if="item.book">
                <IconButton
                    icon="edit-2"
                    size="sm"
                    tabindex="-1"
                    :label="$t('detail.edit')"
                    @click.stop="bookActionsStore.edit([item.book])"
                />
                <IconButton
                    :icon="item.presence === 'both' ? 'check-circle' : 'send'"
                    size="sm"
                    tabindex="-1"
                    :class="{ '-on-device': item.presence === 'both' }"
                    :label="sendLabel"
                    :disabled="!deviceStore.isConnected"
                    @click.stop="bookActionsStore.send([item.book])"
                />
            </template>
            <template v-else>
                <IconButton
                    icon="download"
                    size="sm"
                    tabindex="-1"
                    :label="$t('shelf.addToLibrary')"
                    @click.stop="bookActionsStore.addToLibrary([item])"
                />
                <IconButton
                    icon="x-circle"
                    size="sm"
                    tabindex="-1"
                    :label="$t('shelf.removeFromDevice', { device: deviceName })"
                    @click.stop="bookActionsStore.requestRemoveFromDevice([item])"
                />
            </template>
        </span>
    </div>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import type { BookFormat, DeviceBook } from '@/types'
    import { categoryLabel } from '@/i18n'
    import { fmtLanguageShort } from '@/utils/format'
    import { deviceLabel } from '@/utils/device'
    import { fileOf, itemAuthor, itemFormat, itemSeries, itemTitle, type ShelfItem } from '@/services/shelf'
    import { useDeviceStore } from '@/stores/device'
    import { useBookActionsStore } from '@/stores/bookActions'
    import BookCover from '@/components/library/BookCover.vue'
    import FavoriteStar from '@/components/library/FavoriteStar.vue'
    import SelectCheckbox from '@/components/library/SelectCheckbox.vue'
    import Badge from '@/components/ui/Badge.vue'
    import IconButton from '@/components/ui/IconButton.vue'
    import type { BadgeTone } from '@/components/ui/types'

    const FORMAT_TONES: readonly string[] = ['epub', 'pdf', 'mobi', 'azw3', 'cbz']

    /**
     * One item of the library list (a library book, or a file only on the
     * e-reader): checkbox, cover, series and title with its star (always
     * shown on a favorite, on hover otherwise) and where it is,
     * author, categories, language, format, reading progress (when the
     * e-reader reports some) and two quick actions: edit and send for a
     * library book, add to the library and remove from the device for a
     * file only on the device.
     *
     * The columns line up with the list header through the
     * `--list-columns` grid template set by LibraryList. Like BookCard it
     * is visual only: the list (BookCollection) owns focus, selection and
     * clicks. The two buttons are out of the Tab order (one Tab stop for
     * the whole list); the same actions are in the context menu and the
     * detail panel.
     */
    export default defineComponent({
        name: 'BookRow',

        components: { BookCover, FavoriteStar, SelectCheckbox, Badge, IconButton },

        props: {
            item: { type: Object as PropType<ShelfItem>, required: true },
            selected: { type: Boolean, default: false },
            active: { type: Boolean, default: false },
            selectionMode: { type: Boolean, default: false },
            /** The progress column is shown (the connected device reports progress). */
            showProgress: { type: Boolean, default: false },
            /** The connected e-reader's name ("Kindle"). */
            deviceName: { type: String, default: '' },
        },

        computed: {
            ...mapStores(useDeviceStore, useBookActionsStore),

            file(): DeviceBook | null { return fileOf(this.item) },
            title(): string { return itemTitle(this.item, this.$t) },
            author(): string { return itemAuthor(this.item, this.$t) },
            series(): string | null { return itemSeries(this.item) },
            format(): BookFormat { return itemFormat(this.item) },

            categories(): string {
                return (this.item.book?.metadata.categories ?? []).map(categoryLabel).join(', ')
            },

            formatTone(): BadgeTone {
                return FORMAT_TONES.includes(this.format) ? this.format as BadgeTone : 'neutral'
            },

            whereText(): string {
                return this.item.presence === 'both'
                    ? this.$t('shelf.inBoth', { device: this.deviceName })
                    : this.$t('shelf.onlyOnDevice', { device: this.deviceName })
            },

            sendLabel(): string {
                return this.$t('detail.sendTo', { device: deviceLabel(this.deviceStore.device) })
            },
        },

        methods: {
            fmtLanguageShort,
        },
    })
</script>

<style lang="scss" scoped>
.book-row {
    display: grid;
    grid-template-columns: var(--list-columns);
    align-items: center;
    column-gap: var(--space-3);
    height: 100%;
    padding-right: var(--space-2);
    border-radius: var(--radius-sm);
    border: 1px solid transparent;
    position: relative;
    cursor: pointer;
    contain: layout style;
    transition: background var(--transition), border-color var(--transition);

    &:hover {
        background: color-mix(in srgb, var(--text-secondary) 8%, transparent);
    }

    &.-selected {
        background: color-mix(in srgb, var(--secondary) 8%, transparent);
    }

    &.-active {
        background: color-mix(in srgb, var(--secondary) 12%, transparent);
        border-color: color-mix(in srgb, var(--secondary) 22%, transparent);

        // The accent bar on the left edge of the current row.
        &::before {
            content: '';
            position: absolute;
            left: 0;
            top: 6px;
            bottom: 6px;
            width: 3px;
            border-radius: 0 3px 3px 0;
            background: linear-gradient(180deg, var(--accent), var(--secondary));
        }
    }
}

.book-row__cell {
    min-width: 0;
    font-size: var(--font-size-xs);
    color: var(--text-secondary);

    &.-check {
        display: flex;
        justify-content: center;
        opacity: 0;
        transition: opacity var(--transition);
    }

    &.-cover {
        width: 36px;
    }

    &.-title {
        display: flex;
        flex-direction: column;
        gap: 1px;
    }

    &.-category,
    &.-language {
        color: var(--text-muted);
    }

    &.-language {
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }

    &.-progress {
        text-align: right;
        font-variant-numeric: tabular-nums;
    }

    &.-actions {
        display: flex;
        justify-content: flex-end;
        gap: var(--space-1);
    }
}

.book-row:hover .book-row__cell.-check,
.book-row.-selected .book-row__cell.-check,
.book-row.-selecting .book-row__cell.-check,
:focus-visible > .book-row .book-row__cell.-check {
    opacity: 1;
}

.book-row__series {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--text-muted);
}

.book-row__title-line {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
}

.book-row__star {
    opacity: 0;
    transition: opacity var(--transition);
}

.book-row:hover .book-row__star,
.book-row__star.-on,
:focus-visible > .book-row .book-row__star {
    opacity: 1;
}

.book-row__where {
    display: inline-flex;
    flex-shrink: 0;
    gap: 2px;
    color: var(--accent-text);
}

.book-row__title {
    font-family: var(--font-serif);
    font-size: calc(var(--font-size-md) * var(--serif-scale));
    font-weight: var(--serif-weight);
    color: var(--text-primary);
}

.-on-device {
    color: var(--color-on-device);
}
</style>
