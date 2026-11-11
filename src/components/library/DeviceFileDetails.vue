<template>
    <div class="file-details">
        <p class="file-details__where">
            <vue-feather type="tablet" size="12" aria-hidden="true" />
            {{ $t('shelf.onlyOnDevice', { device: deviceName }) }}
        </p>
        <!-- The dialog's heading and its first focus (BookDetailModal). -->
        <h2 :id="titleId" class="file-details__title" tabindex="-1">{{ title }}</h2>
        <p class="file-details__author">{{ author }}</p>
        <span class="ornament -rule file-details__rule" aria-hidden="true"></span>
        <dl class="file-facts">
            <div class="file-facts__row">
                <dt>{{ $t('shelf.file') }}</dt>
                <dd>{{ file.filename }}</dd>
            </div>
            <div class="file-facts__row">
                <dt>{{ $t('library.colFormat') }}</dt>
                <dd>{{ format.toUpperCase() }}</dd>
            </div>
            <div v-if="file.sizeBytes !== undefined" class="file-facts__row">
                <dt>{{ $t('shelf.size') }}</dt>
                <dd>{{ fmtSize(file.sizeBytes) }}</dd>
            </div>
            <div v-if="item.progress !== undefined" class="file-facts__row">
                <dt>{{ $t('collection.colProgress') }}</dt>
                <dd>{{ file.readStatus === 'finished' ? $t('devices.finished') : `${item.progress}%` }}</dd>
            </div>
        </dl>
        <p class="file-details__hint">{{ $t('shelf.onlyOnDeviceHint', { device: deviceName }) }}</p>
    </div>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import type { BookFormat, DeviceBook } from '@/types'
    import { fileFormat, itemAuthor, itemTitle, type ShelfItem } from '@/services/shelf'
    import { useDeviceStore } from '@/stores/device'
    import { deviceLabel } from '@/utils/device'
    import { fmtSize } from '@/utils/format'

    /**
     * The details of a file that is only on the e-reader (no library book
     * matches it), in BookDetailModal: what the device says about it
     * (title, author, file name, format, size, reading progress), and that
     * it can be added to the library (the buttons are the modal's, under
     * the cover).
     */
    export default defineComponent({
        name: 'DeviceFileDetails',

        props: {
            /** A device-only shelf item (`presence: 'deviceOnly'`). */
            item: { type: Object as PropType<ShelfItem>, required: true },
            /** Id of the title heading (the dialog is labelled by it). */
            titleId: { type: String, required: true },
        },

        computed: {
            ...mapStores(useDeviceStore),

            file(): DeviceBook { return this.item.copies[0] },
            title(): string { return itemTitle(this.item, this.$t) },
            author(): string { return itemAuthor(this.item, this.$t) },
            format(): BookFormat { return fileFormat(this.file.filename) },
            deviceName(): string { return deviceLabel(this.deviceStore.device) },
        },

        methods: {
            fmtSize,
        },
    })
</script>

<style lang="scss" scoped>
.file-details {
    display: flex;
    flex-direction: column;
    min-width: 0;
}

.file-details__where {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin: 0 0 var(--space-1);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--accent-text);
}

.file-details__title {
    margin: 0 0 var(--space-2);
    font-family: var(--font-serif);
    font-size: calc(var(--font-size-2xl) * 1.15 * var(--serif-scale));
    font-weight: var(--serif-weight);
    line-height: 1.1;
    color: var(--text-primary);
    overflow-wrap: anywhere;
    text-wrap: balance;
    outline: none;
}

.file-details__author {
    margin: 0 0 var(--space-4);
    font-family: var(--font-serif);
    font-style: italic;
    font-size: calc(var(--font-size-lg) * var(--serif-scale));
    color: var(--text-secondary);
    overflow-wrap: anywhere;
}

.file-details__rule {
    width: 140px;
    margin: 0 0 var(--space-4);
}

// As a book's facts (BookMetadataTable): label over value, side by side.
.file-facts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: var(--space-3) var(--space-4);
    margin: 0 0 var(--space-4);
    padding: var(--space-4) 0;
    background:
        linear-gradient(90deg, var(--hairline), transparent 85%) top / 100% 1px no-repeat,
        linear-gradient(90deg, var(--hairline), transparent 85%) bottom / 100% 1px no-repeat;
}

.file-facts__row {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;

    dt {
        font-size: var(--font-size-xs);
        font-weight: var(--font-weight-semibold);
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--text-muted);
    }

    dd {
        margin: 0;
        font-size: var(--font-size-sm);
        color: var(--text-primary);
        overflow-wrap: anywhere;
    }
}

.file-details__hint {
    max-width: 520px;
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--text-secondary);
}
</style>
