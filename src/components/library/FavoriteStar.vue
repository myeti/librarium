<template>
    <button
        type="button"
        class="favorite-star"
        :class="[`-${variant}`, { '-on': on }]"
        tabindex="-1"
        :aria-label="label"
        :title="label"
        @click.stop="bookActionsStore.toggleFavorite([book])"
    >
        <vue-feather type="star" :size="variant === 'card' ? '18' : '14'" aria-hidden="true" />
    </button>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import type { Book } from '@/types'
    import { useBookActionsStore } from '@/stores/bookActions'
    import { displayTitle } from '@/utils/bookDisplay'

    /**
     * The star of a book card (`card`: in the cover's corner, a shadow
     * keeping it visible on light and dark covers alike) or list row
     * (`row`: by the title): filled in the accent when the book
     * is a favorite. A click stars or unstars it; the click doesn't reach
     * the collection (it would open the book).
     *
     * Out of the Tab order, like the row buttons: from the keyboard, the
     * context menu and the detail panel do the same, and the book's name in
     * the collection says "favorite". Whether it shows when off (on hover,
     * on focus) is the parent's styling.
     */
    export default defineComponent({
        name: 'FavoriteStar',

        props: {
            book:    { type: Object as PropType<Book>, required: true },
            variant: { type: String as PropType<'card' | 'row'>, default: 'card' },
        },

        computed: {
            ...mapStores(useBookActionsStore),

            on(): boolean {
                return this.book.favorite === true
            },

            label(): string {
                const title = displayTitle(this.book, this.$t)
                return this.on ? this.$t('favorites.removeOne', { title }) : this.$t('favorites.addOne', { title })
            },
        },
    })
</script>

<style lang="scss" scoped>
.favorite-star {
    display: inline-grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--text-secondary);
    cursor: pointer;
    transition: color var(--transition), background var(--transition), transform var(--transition);

    &:hover { color: var(--accent-text); }
    &:active { transform: scale(.88); }

    // Filled when on.
    &.-on {
        color: var(--accent-text);

        :deep(svg) { fill: currentColor; }
    }

    // On a cover: the star alone, no frame; its shadow keeps it readable
    // on a pale cover. The box is only the click target.
    &.-card {
        width: 18px;
        height: 18px;
        color: #fff;
        filter: drop-shadow(0 1px 1.5px rgba(0,0,0,.65)) drop-shadow(0 0 4px rgba(0,0,0,.35));

        &.-on { color: var(--accent-text); }
    }

    &.-row {
        width: 20px;
        height: 20px;
        border-radius: var(--radius-xs);

        &:hover { background: var(--bg-hover); }
    }

    @media (prefers-reduced-motion: reduce) {
        &:active { transform: none; }
    }
}
</style>
