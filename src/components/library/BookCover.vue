<template>
    <div class="book-cover" :class="[`-${variant}`, `cover-${format}`]">
        <img
            v-if="src"
            :src="src"
            alt=""
            :loading="lazy ? 'lazy' : undefined"
            decoding="async"
            @error="onError"
        />
        <!-- Generated cover: what a card shows until (or instead of) a real cover,
             dressed like the theme's book covers: cloth, a gilt frame and
             corners, the title in the theme's lettering, its emblem. -->
        <div v-else-if="variant !== 'row'" class="cover-gen" :style="{ '--cover-cloth': cloth }" aria-hidden="true">
            <span class="cover-gen-frame"></span>
            <span v-for="corner in CORNERS" :key="corner" class="ornament -corner cover-gen-corner" :class="`-${corner}`"></span>
            <p v-if="series" class="cover-gen-series">{{ series }}</p>
            <p class="cover-gen-title">{{ title }}</p>
            <span class="ornament -rule cover-gen-rule"></span>
            <span class="ornament -art cover-gen-art"></span>
            <p class="cover-gen-author">{{ author }}</p>
        </div>
        <div v-else class="cover-placeholder" aria-hidden="true">
            <vue-feather type="book" size="16" />
        </div>
        <!-- Overlays drawn on the cover (selection checkbox, reading progress). -->
        <slot />
    </div>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import type { Book, BookFormat, DeviceBook } from '@/types'
    import { useLibraryStore } from '@/stores/library'
    import { displayAuthor, displayTitle, seriesLabel } from '@/utils/bookDisplay'
    import { fileFormat } from '@/services/shelf'
    import { coverColor } from '@/utils/coverArt'
    import { watchVisibility } from '@/utils/visibility'

    /** The four corner ornaments of a generated cover. */
    const CORNERS = Object.freeze(['tl', 'tr', 'bl', 'br'] as const)

    /**
     * A book's cover (a library book, or a `file` only on the e-reader,
     * which never has one), or a placeholder while it has none: a generated cover
     * (series, title, author, on one of the theme's cover cloths, see
     * utils/coverArt.ts) on cards and in the detail panel, a book icon on
     * list rows. Cards and the panel's cover look like books: a spine
     * shading on the left edge (and on cards, a foil glint when hovered:
     * BookCard).
     *
     * Covers load lazily. While the library store has no cover for the book,
     * a `lazy` cover watches its own visibility and asks the store for it
     * when it scrolls into view, and withdraws the request if it scrolls
     * away (or is unmounted: the virtual grid unmounts cards that leave the
     * screen) before the request started. A cover that isn't lazy (the
     * detail panel) asks at once. Only this book's entry of the store's
     * covers is read, so a cover arriving for another book doesn't re-render
     * this one.
     *
     * The image is decorative (`alt=""`): the title is always written next
     * to it.
     */
    export default defineComponent({
        name: 'BookCover',

        props: {
            /** The library book; null for a file only on the e-reader (then `file`). */
            book: { type: Object as PropType<Book | null>, default: null },
            /** A file on the e-reader no library book matches: it gets a generated cover. */
            file: { type: Object as PropType<DeviceBook | null>, default: null },
            variant: { type: String as PropType<'card' | 'row' | 'panel'>, default: 'card' },
            /** Ask for the cover only once visible (cards and rows), or at once (the panel). */
            lazy: { type: Boolean, default: true },
        },

        data(): { stopWatch: (() => void) | null, holdsRequest: boolean, CORNERS: typeof CORNERS } {
            return {
                CORNERS,
                /** Stops the visibility watch started for a missing cover. */
                stopWatch: null,
                /** Whether this cover registered interest in a queued request. */
                holdsRequest: false,
            }
        },

        computed: {
            ...mapStores(useLibraryStore),

            /** The library book's id: covers are only looked for for library books. */
            bookId(): string | null { return this.book?.id ?? null },

            src(): string | null {
                return this.bookId === null ? null : this.libraryStore.coverUrlOf(this.bookId)
            },

            hasCover(): boolean {
                return this.bookId !== null && this.libraryStore.coverOf(this.bookId) !== null
            },

            format(): BookFormat {
                if (this.book) { return this.book.format }
                return this.file ? fileFormat(this.file.filename) : 'unknown'
            },

            title(): string {
                if (this.book) { return displayTitle(this.book, this.$t) }
                const file = this.file
                return file ? (file.title.trim() || file.filename.replace(/\.[^.]+$/, '')) : ''
            },
            author(): string { return displayAuthor(this.book ?? this.file ?? { title: '', author: '' }, this.$t) },
            series(): string | null { return this.book ? seriesLabel(this.book) : null },
            /** The generated cover's cloth: the same for the book (and its series) on every launch. */
            cloth(): string {
                if (this.book) { return coverColor(this.book) }
                return coverColor({ id: this.file?.path ?? '' })
            },
        },

        watch: {
            bookId(_: string | null, oldId: string | null) {
                if (oldId !== null) { this.release(oldId) }
                this.sync()
            },
            // Set: the request is done. Cleared (cover deleted, book reset): ask again.
            hasCover() {
                this.holdsRequest = false
                this.sync()
            },
        },

        mounted() {
            this.sync()
        },

        beforeUnmount() {
            this.stopWatching()
            // Scrolled out of the virtual window, or filtered out: no need to load it.
            if (this.bookId !== null) { this.release(this.bookId) }
        },

        methods: {
            sync() {
                this.stopWatching()
                const bookId = this.bookId
                if (bookId === null || this.hasCover) { return }
                if (!this.lazy) {
                    this.libraryStore.requestCover(bookId)
                    return
                }
                this.stopWatch = watchVisibility(this.$el as Element, visible => {
                    if (visible && !this.holdsRequest) {
                        this.holdsRequest = this.libraryStore.requestCover(bookId)
                    } else if (!visible) {
                        this.release(bookId)
                    }
                })
            },

            stopWatching() {
                this.stopWatch?.()
                this.stopWatch = null
            },

            /** Withdraws this cover's interest in a request that hasn't started. */
            release(bookId: string) {
                if (!this.holdsRequest) { return }
                this.holdsRequest = false
                this.libraryStore.cancelCoverRequest(bookId)
            },

            /** The file didn't load (ignored if another view of the same cover already reported it). */
            onError() {
                if (this.src && this.bookId !== null) { this.libraryStore.coverFailedToLoad(this.bookId) }
            },
        },
    })
</script>

<style lang="scss" scoped>
.book-cover {
    position: relative;
    width: 100%;
    aspect-ratio: var(--cover-ratio);
    overflow: hidden;
    background: var(--bg-cover-placeholder);
    // The generated cover's lettering is sized by the cover's own width.
    container-type: inline-size;

    img {
        position: absolute;
        inset: 0;
        width: 100%;
        height: 100%;
        object-fit: cover;
        object-position: top;
    }

    // A book, not a tile: square spine, slightly rounded fore-edge, the
    // shading of the hinge along the left and a soft light from the top.
    &.-card,
    &.-panel {
        border-radius: 2px var(--radius-cover) var(--radius-cover) 2px;

        &::after {
            content: "";
            position: absolute;
            inset: 0;
            pointer-events: none;
            background:
                linear-gradient(90deg,
                    rgba(0,0,0,.28) 0%, rgba(255,255,255,.16) 1.6%, rgba(0,0,0,.10) 3.4%,
                    rgba(255,255,255,.05) 5.5%, transparent 9%),
                linear-gradient(180deg, rgba(255,255,255,.07), transparent 22%, transparent 78%, rgba(0,0,0,.14));
            box-shadow: inset 0 0 0 1px rgba(255,255,255,.06);
        }
    }
    &.-row { border-radius: var(--radius-xs); }
}

.cover-epub    { color: var(--fmt-epub); }
.cover-pdf     { color: var(--fmt-pdf); }
.cover-mobi,
.cover-azw3    { color: var(--fmt-mobi); }
.cover-cbz     { color: var(--fmt-cbz); }
.cover-unknown { color: var(--text-muted); }

.cover-placeholder {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    gap: var(--space-2);
    padding: 2px;
    text-align: center;
}

// ── The generated cover ──────────────────────────────────────────
// Sizes are fractions of the cover's width (cqi), so a wide card and a
// narrow one look like the same book. It is a picture (aria-hidden): the
// card writes the same series, title and author below it in full size, so
// the small capitals here may go under the app's 12px minimum.
.cover-gen {
    --ornament-paint: var(--cover-foil);
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 15cqi 13cqi 11cqi;
    text-align: center;
    color: var(--cover-ink);
    background:
        radial-gradient(120% 70% at 50% 0%, rgba(255,255,255,.08), transparent 60%),
        var(--cover-cloth, var(--cover-1));

    // The lettering keeps its size; only the emblem gives way to a long title.
    > * { flex-shrink: 0; }
}

// A gilt rule inset from the edge, and an ornament in each corner.
.cover-gen-frame {
    position: absolute;
    inset: 5.5cqi;
    border: 1px solid color-mix(in srgb, var(--cover-ink-soft) 38%, transparent);
    border-radius: 1px;
    pointer-events: none;
}

.cover-gen-corner {
    position: absolute;
    width: 17cqi;
    opacity: .85;

    &.-tl { top: 3cqi; left: 3cqi; }
    &.-tr { top: 3cqi; right: 3cqi; transform: scaleX(-1); }
    &.-bl { bottom: 3cqi; left: 3cqi; transform: scaleY(-1); }
    &.-br { bottom: 3cqi; right: 3cqi; transform: scale(-1); }
}

.cover-gen-series {
    max-width: 100%;
    margin-bottom: 4cqi;
    overflow: hidden;
    font-size: clamp(10px, 5.4cqi, 12px);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.16em;
    text-transform: uppercase;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--cover-ink-soft);
}

.cover-gen-title {
    display: -webkit-box;
    -webkit-line-clamp: 5;
    -webkit-box-orient: vertical;
    line-clamp: 5;
    max-width: 100%;
    overflow: hidden;
    overflow-wrap: anywhere;
    font-family: var(--font-display);
    font-weight: var(--display-weight);
    font-size: clamp(13px, 9.6cqi, 22px);
    letter-spacing: calc(var(--display-tracking) * 0.45);
    line-height: 1.18;
    text-transform: uppercase;
    text-wrap: balance;
    color: var(--cover-ink);
    text-shadow: 0 1px 2px rgba(0,0,0,.35);
}

.cover-gen-rule {
    width: 46cqi;
    margin-top: 5cqi;
}

// The theme's emblem (a key, a window on the stars, an arch, a moon),
// filling what the title leaves.
.cover-gen-art {
    flex: 1 1 auto;
    flex-shrink: 1;
    min-height: 0;
    width: 62cqi;
    margin: 4cqi 0;
    aspect-ratio: auto;
    opacity: .9;
}

.cover-gen-author {
    max-width: 100%;
    overflow: hidden;
    font-size: clamp(10px, 5.6cqi, 13px);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--cover-ink-soft);
}
</style>
