<template>
    <div class="detail-header">
        <p v-if="series" class="detail-header__series">{{ series }}</p>
        <!-- The dialog's heading and its first focus (BookDetailModal). -->
        <h2 :id="titleId" class="detail-header__title" tabindex="-1">{{ title }}</h2>
        <p class="detail-header__author">{{ author }}</p>
        <span class="ornament -rule detail-header__rule" aria-hidden="true"></span>
        <div class="detail-header__tags">
            <Badge :tone="formatTone">{{ book.format }}</Badge>
            <Badge v-if="deviceName" tone="neutral" icon="tablet">{{ $t('detail.onDevice', { device: deviceName }) }}</Badge>
            <Badge v-for="cat in book.metadata.categories ?? []" :key="cat" tone="accent">{{ categoryLabel(cat) }}</Badge>
        </div>
    </div>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import type { Book } from '@/types'
    import { categoryLabel } from '@/i18n'
    import { displayAuthor, displayTitle, seriesLabel } from '@/utils/bookDisplay'
    import Badge from '@/components/ui/Badge.vue'
    import type { BadgeTone } from '@/components/ui/types'

    const FORMAT_TONES: readonly string[] = ['epub', 'pdf', 'mobi', 'azw3', 'cbz']

    /**
     * The top of a book's details (BookDetailModal): the series, the title
     * (the dialog's heading, `titleId`), the author, then badges: the
     * format, "On Kindle" when a copy is on the e-reader, the categories.
     */
    export default defineComponent({
        name: 'BookDetailHeader',

        components: { Badge },

        props: {
            book: { type: Object as PropType<Book>, required: true },
            /** Id of the title heading (the dialog is labelled by it). */
            titleId: { type: String, required: true },
            /** The e-reader's name when a copy of the book is on it ('' otherwise). */
            deviceName: { type: String, default: '' },
        },

        computed: {
            title(): string { return displayTitle(this.book, this.$t) },
            author(): string { return displayAuthor(this.book, this.$t) },
            series(): string | null { return seriesLabel(this.book) },
            formatTone(): BadgeTone {
                return FORMAT_TONES.includes(this.book.format) ? this.book.format as BadgeTone : 'neutral'
            },
        },

        methods: {
            categoryLabel,
        },
    })
</script>

<style lang="scss" scoped>
.detail-header {
    display: flex;
    flex-direction: column;
    min-width: 0;
}

.detail-header__series {
    margin: 0 0 var(--space-1);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--accent-text);
}

.detail-header__title {
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

.detail-header__author {
    margin: 0 0 var(--space-4);
    font-family: var(--font-serif);
    font-style: italic;
    font-size: calc(var(--font-size-lg) * var(--serif-scale));
    color: var(--text-secondary);
    overflow-wrap: anywhere;
}

.detail-header__rule {
    width: 140px;
    margin: 0 0 var(--space-4);
}

.detail-header__tags {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 5px;
}
</style>
