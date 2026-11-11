<template>
    <div class="book-card" :class="{ '-selected': selected, '-active': active, '-selecting': selectionMode }">
        <BookCover :book="item.book" :file="file" variant="card">
            <SelectCheckbox class="book-card__check" :checked="selected" :label="$t('collection.select', { title })" />
            <FavoriteStar v-if="item.book" class="book-card__star" :book="item.book" />
            <!-- Where the book is, when it is on the e-reader: in both places, or only there. -->
            <span v-if="item.presence !== 'libraryOnly'" class="book-card__where" :class="`-${item.presence}`" :title="whereText">
                <vue-feather v-if="item.presence === 'both'" type="book" size="11" aria-hidden="true" />
                <vue-feather type="tablet" size="11" aria-hidden="true" />
                <span class="sr-only">{{ whereText }}</span>
            </span>
            <!-- The option's name says it too ("42% read"): not read out twice. -->
            <span v-if="item.progress !== undefined" class="book-card__percent" aria-hidden="true">{{ item.progress }}%</span>
        </BookCover>
        <div class="book-card__info">
            <p v-if="series" class="book-card__series truncate" :title="series">{{ series }}</p>
            <p class="book-card__title" :title="title">{{ title }}</p>
            <p class="book-card__author truncate" :title="author">{{ author }}</p>
        </div>
    </div>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import type { DeviceBook } from '@/types'
    import { fileOf, itemAuthor, itemSeries, itemTitle, type ShelfItem } from '@/services/shelf'
    import BookCover from '@/components/library/BookCover.vue'
    import FavoriteStar from '@/components/library/FavoriteStar.vue'
    import SelectCheckbox from '@/components/library/SelectCheckbox.vue'

    /**
     * One item of the library grid: a library book, or a file only on the
     * e-reader. Its cover (or a generated one), and under it the series, the
     * title (two lines at most) and the author, always visible. Text that
     * doesn't fit ends with "…" and shows in full as a tooltip.
     *
     * On the cover: the selection checkbox (top left), the star (top
     * right: always shown on a favorite, on hover and focus otherwise; a
     * click stars or unstars the book), where the book is when it is on the
     * e-reader (bottom left: a book and a tablet when it is in both places,
     * a tablet alone when it is only on the device), and the reading
     * progress the e-reader reports ("42%", bottom right).
     *
     * Purely visual: the grid (BookCollection) owns focus, selection and
     * clicks, and gives each card its `role="option"` wrapper.
     *
     * The selection checkbox shows on hover, on keyboard focus, when the
     * book is selected, and on every card while some book is selected
     * (`selectionMode`).
     */
    export default defineComponent({
        name: 'BookCard',

        components: { BookCover, FavoriteStar, SelectCheckbox },

        props: {
            item: { type: Object as PropType<ShelfItem>, required: true },
            selected: { type: Boolean, default: false },
            /** The grid's current book (keyboard focus, or shown in the panel). */
            active: { type: Boolean, default: false },
            /** Some book is selected: every checkbox shows. */
            selectionMode: { type: Boolean, default: false },
            /** The connected e-reader's name ("Kindle"), for the location badge. */
            deviceName: { type: String, default: '' },
        },

        computed: {
            file(): DeviceBook | null { return fileOf(this.item) },
            title(): string { return itemTitle(this.item, this.$t) },
            author(): string { return itemAuthor(this.item, this.$t) },
            series(): string | null { return itemSeries(this.item) },
            whereText(): string {
                return this.item.presence === 'both'
                    ? this.$t('shelf.inBoth', { device: this.deviceName })
                    : this.$t('shelf.onlyOnDevice', { device: this.deviceName })
            },
        },
    })
</script>

<style lang="scss" scoped>
// The grid gives every card the same height (LibraryGrid's GRID_LAYOUT:
// the cover's height plus `infoHeight`); the text block takes what the cover
// leaves, which fits a series line, two title lines and an author line.
//
// No box around a card: the cover is a book standing on the page, with its
// shadow; hovering lifts it off the shelf and runs a foil glint across it.
.book-card {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 0;
    border-radius: var(--radius-sm);
    cursor: pointer;
    contain: layout style;

    // The tint behind a selected or current book.
    &::before {
        content: "";
        position: absolute;
        inset: -7px -7px -2px;
        border-radius: var(--radius);
        background: transparent;
        border: 1px solid transparent;
        transition: background var(--transition), border-color var(--transition);
        pointer-events: none;
    }

    :deep(.book-cover) {
        box-shadow:
            0 1px 1px rgba(0,0,0,.18),
            0 6px 14px -4px rgba(0,0,0,.35),
            0 18px 30px -18px rgba(0,0,0,.5);
        transition:
            transform var(--duration-slow) var(--ease-lift),
            box-shadow var(--duration-slow) var(--ease-out);
    }

    // The glint: a band of light across the cover, once per hover.
    :deep(.book-cover)::before {
        content: "";
        position: absolute;
        inset: 0;
        z-index: 1;
        pointer-events: none;
        background: linear-gradient(105deg, transparent 35%, rgba(255,255,255,.22) 47%, rgba(255,255,255,.04) 55%, transparent 65%);
        transform: translateX(-120%);
    }

    &:hover :deep(.book-cover) {
        transform: translateY(-6px) rotate(-0.4deg);
        box-shadow: var(--glow-card);
    }

    &:hover :deep(.book-cover)::before {
        transform: translateX(120%);
        transition: transform var(--duration-sheen) var(--ease-out);
    }

    &.-selected::before {
        background: var(--accent-muted);
        border-color: color-mix(in srgb, var(--hairline) 70%, transparent);
    }

    &.-active :deep(.book-cover) {
        box-shadow: 0 0 0 2px var(--accent), var(--glow-card);
    }

    @media (prefers-reduced-motion: reduce) {
        &:hover :deep(.book-cover) { transform: none; }
        :deep(.book-cover)::before { display: none; }
    }
}

.book-card__check {
    position: absolute;
    top: var(--space-2);
    left: var(--space-2);
    z-index: 2;
    opacity: 0;
    transition: opacity var(--transition);
}

// Shown on hover, on keyboard focus (the option around the card), when
// selected, and while any book is selected.
.book-card:hover .book-card__check,
.book-card.-selected .book-card__check,
.book-card.-selecting .book-card__check,
:focus-visible > .book-card .book-card__check {
    opacity: 1;
}

// The star, top right: always on a favorite, else like the checkbox.
.book-card__star {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    z-index: 2;
    opacity: 0;
    transition: opacity var(--transition);
}

.book-card:hover .book-card__star,
.book-card__star.-on,
:focus-visible > .book-card .book-card__star {
    opacity: 1;
}

// Where the book is: a small glass plate in the bottom-left corner, the
// same size, shape and distance from the edge as the progress tag across.
.book-card__where {
    position: absolute;
    bottom: var(--space-2);
    left: var(--space-2);
    z-index: 2;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    height: 15px;
    padding: 0 3px;
    border-radius: var(--radius-xs);
    color: var(--accent-text);
    background: color-mix(in srgb, var(--bg) 78%, transparent);
    border: 1px solid var(--hairline);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    box-shadow: 0 2px 6px rgba(0,0,0,.3);
}

// The reading progress: a "42%" plate in the cover's bottom-right corner.
.book-card__percent {
    position: absolute;
    right: var(--space-2);
    bottom: var(--space-2);
    z-index: 2;
    padding: 0 4px;
    border-radius: var(--radius-xs);
    // 12px: the smallest text the app uses anywhere.
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-bold);
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.01em;
    line-height: 15px;
    color: var(--on-accent);
    background: linear-gradient(135deg, var(--accent-strong), var(--accent-dark));
    box-shadow: 0 2px 6px rgba(0,0,0,.35);
}


.book-card__info {
    position: relative;
    display: flex;
    flex-direction: column;
    flex: 1 1 auto;
    min-height: 0;
    padding: 11px 2px 4px;
    overflow: hidden;
}

.book-card__series {
    font-size: var(--font-size-xs);
    // font-weight: var(--font-weight-semibold);
    line-height: 17px;
    // letter-spacing: 0.1em;
    // text-transform: uppercase;
    color: var(--accent-text);
}

.book-card__title {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    line-clamp: 2;
    overflow: hidden;
    overflow-wrap: anywhere;
    font-family: var(--font-serif);
    font-size: calc(var(--font-size-md) * var(--serif-scale));
    font-weight: var(--serif-weight);
    line-height: 19px;
    color: var(--text-primary);
}

.book-card__author {
    margin-top: 2px;
    font-size: var(--font-size-xs);
    line-height: 17px;
    color: var(--text-secondary);
}
</style>
