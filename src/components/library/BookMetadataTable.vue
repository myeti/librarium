<template>
    <dl class="metadata-table">
        <div v-for="row in rows" :key="row.key" class="metadata-table__row">
            <dt class="metadata-table__label">{{ row.label }}</dt>
            <dd class="metadata-table__value">
                <a
                    v-if="row.href"
                    :href="row.href"
                    class="metadata-table__link"
                    target="_blank"
                    rel="noopener"
                >{{ row.value }}</a>
                <template v-else>{{ row.value }}</template>
            </dd>
        </div>
    </dl>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import type { Book } from '@/types'
    import { fmtDate, fmtLanguage, fmtSize } from '@/utils/format'
    import { seriesLabel } from '@/utils/bookDisplay'

    interface MetadataRow {
        key: string
        label: string
        value: string
        href?: string
    }

    /**
     * The facts about a book in its details, as a description list:
     * language, series, ISBN, size, date added, reading progress on the
     * connected e-reader, Goodreads link. Rows without a value are left out
     * (language and size always show).
     */
    export default defineComponent({
        name: 'BookMetadataTable',

        props: {
            book: { type: Object as PropType<Book>, required: true },
            /** Reading progress on the connected e-reader, 0–100. */
            progress: { type: Number as PropType<number | undefined>, default: undefined },
        },

        computed: {
            rows(): MetadataRow[] {
                const m = this.book.metadata
                const rows: MetadataRow[] = [
                    { key: 'language', label: this.$t('detail.language'), value: fmtLanguage(m.language) },
                ]
                const series = seriesLabel(this.book)
                if (series) { rows.push({ key: 'series', label: this.$t('detail.series'), value: series }) }
                if (m.isbn) { rows.push({ key: 'isbn', label: this.$t('detail.isbn'), value: m.isbn }) }
                rows.push(
                    { key: 'size', label: this.$t('detail.size'), value: fmtSize(this.book.sizeBytes) },
                    { key: 'added', label: this.$t('detail.added'), value: fmtDate(this.book.dateAdded) },
                )
                if (this.progress !== undefined) {
                    rows.push({ key: 'progress', label: this.$t('collection.colProgress'), value: `${this.progress}%` })
                }
                if (m.goodreadsUrl) {
                    rows.push({ key: 'goodreads', label: this.$t('detail.goodreads'), value: this.$t('detail.goodreadsView'), href: m.goodreadsUrl })
                }
                return rows
            },
        },
    })
</script>

<style lang="scss" scoped>
// The facts side by side, label over value, as many per line as fit;
// between two gilt lines.
.metadata-table {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: var(--space-3) var(--space-4);
    margin: 0;
    padding: var(--space-4) 0;
    background:
        linear-gradient(90deg, var(--hairline), transparent 85%) top / 100% 1px no-repeat,
        linear-gradient(90deg, var(--hairline), transparent 85%) bottom / 100% 1px no-repeat;
}

.metadata-table__row {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
}

.metadata-table__label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-muted);
}

.metadata-table__value {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--text-primary);
    overflow-wrap: anywhere;
}

.metadata-table__link {
    color: var(--accent-text);
    text-decoration: underline;
    text-underline-offset: 2px;
}
</style>
