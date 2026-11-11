<template>
    <BaseModal
        size="screen"
        bare
        :labelledby="titleId"
        initial-focus=".reader-shell"
        @close="$emit('close')"
        @keydown="onKey"
    >
        <div class="reader-shell" tabindex="-1">
            <!-- Header -->
            <div class="reader-header">
                <h2 :id="titleId" class="reader-title truncate">{{ displayTitle(book, $t) }}</h2>
                <IconButton icon="x" :label="$t('reader.close')" class="reader-close" @click="$emit('close')" />
            </div>

            <!-- Loading -->
            <EmptyState
                v-if="state === 'loading'"
                loading
                heading-tag="p"
                :title="isConverting ? $t('reader.converting') : $t('reader.loading')"
                class="reader-state"
            />

            <!-- Error -->
            <EmptyState
                v-else-if="state === 'error'"
                alert
                icon="alert-circle"
                :icon-size="28"
                heading-tag="p"
                :title="$t('reader.openFailed')"
                :description="errorMsg"
                class="reader-state"
            />

            <!-- EPUB -->
            <div v-else-if="readerFormat === 'epub'" class="reader-epub-area">
                <div class="reader-device">
                    <div class="device-screen-wrap">
                        <div ref="epubContainer" class="reader-epub"></div>
                    </div>
                    <div class="device-nav">
                        <IconButton icon="chevron-left" :label="$t('reader.prev')" class="device-btn" @click="prevPage" />
                        <span class="reader-pager" aria-live="polite">{{ pagerText }}</span>
                        <IconButton icon="chevron-right" :label="$t('reader.next')" class="device-btn -right" @click="nextPage" />
                    </div>
                </div>
            </div>

            <!-- PDF: the webview's own viewer reads the file from its asset URL -->
            <iframe
                v-else-if="readerFormat === 'pdf'"
                class="reader-pdf"
                :title="displayTitle(book, $t)"
                :src="pdfUrl"
            ></iframe>
        </div>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent, markRaw, type PropType } from 'vue'
    import { mapActions } from 'pinia'
    import ePub from 'epubjs'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import IconButton from '@/components/ui/IconButton.vue'
    import EmptyState from '@/components/ui/EmptyState.vue'
    import { useLibraryStore } from '@/stores/library'
    import { needsConversion, openBook } from '@/services/reader'
    import { displayTitle } from '@/utils/bookDisplay'
    import { uniqueId } from '@/utils/dom'
    import { errorMessage } from '@/utils/errors'
    import type { Book } from '@/types'

    type State = 'loading' | 'ready' | 'error'

    /** Where the reader is, as epub.js reports it in `relocated` and `currentLocation()`. */
    interface ReaderLocation {
        start: { cfi: string, displayed: { page: number, total: number } }
    }

    /**
     * The parts of epub.js the reader uses. Its bundled typings declare no
     * `on()` and type `Locations.locationFromCfi` as returning an object (it
     * returns a number), so the objects are seen through this interface.
     */
    interface Rendition {
        themes: { default(rules: Record<string, Record<string, string>>): void }
        on(event: 'relocated', listener: (location: ReaderLocation) => void): void
        /** Keys pressed while the focus is inside the book's iframe (they don't reach this page). */
        on(event: 'keydown', listener: (e: KeyboardEvent) => void): void
        display(): Promise<void>
        prev(): Promise<void>
        next(): Promise<void>
        destroy(): void
        /** `undefined`, or `{}` before anything is displayed. */
        currentLocation(): Partial<ReaderLocation> | undefined
    }

    interface Locations {
        total: number
        generate(chars: number): Promise<string[]>
        locationFromCfi(cfi: string): number
    }

    /**
     * Previews a book, full screen: EPUBs with epub.js (MOBI/AZW3 are
     * converted by the backend first, and cached), PDFs in the webview's own
     * viewer. The file is loaded from its asset URL, never sent over IPC
     * (`services/reader`).
     *
     * A BaseModal of size `screen`: the page behind is inert, and its keys
     * stay here (the library's arrow-key navigation doesn't move behind the
     * book). ← / → turn pages and Escape closes, also while the focus is
     * inside the book (epub.js forwards the iframe's keys).
     */
    export default defineComponent({
        name: 'ReaderModal',

        components: { BaseModal, IconButton, EmptyState },

        props: {
            book: {
                type: Object as PropType<Book>,
                required: true,
            },
        },

        emits: ['close'],

        data(): {
            titleId: string
            state: State
            /** How the book is displayed: 'epub' or 'pdf' ('' while loading). */
            readerFormat: '' | 'epub' | 'pdf'
            /** The PDF's asset URL. */
            pdfUrl: string
            errorMsg: string
            /** MOBI/AZW3: the backend converts first (the first time), which takes a while. */
            isConverting: boolean
            /** Not reactive (markRaw): epub.js objects must not be wrapped in proxies. */
            rendition: Rendition | null
            currentPage: number
            totalPages: number
            /** The dialog went away while the book was loading: drop the result. */
            closed: boolean
        } {
            return {
                titleId: uniqueId('reader-title'),
                state: 'loading',
                readerFormat: '',
                pdfUrl: '',
                errorMsg: '',
                isConverting: false,
                rendition: null,
                currentPage: 0,
                totalPages: 0,
                closed: false,
            }
        },

        computed: {
            pagerText(): string {
                return this.totalPages > 0 ? `${this.currentPage} / ${this.totalPages}` : ''
            },
        },

        async mounted() {
            await this.load()
        },

        beforeUnmount() {
            this.closed = true
            this.rendition?.destroy()
        },

        methods: {
            ...mapActions(useLibraryStore, ['requireLibrary']),
            displayTitle,

            async load() {
                this.isConverting = needsConversion(this.book.format)
                this.state = 'loading'
                try {
                    const { libraryPath } = this.requireLibrary()
                    const opened = await openBook(libraryPath, this.book)
                    if (this.closed) { return }
                    this.readerFormat = opened.format
                    if (opened.format === 'pdf') {
                        this.pdfUrl = opened.url
                        this.state = 'ready'
                    } else {
                        this.state = 'ready'
                        await this.$nextTick()
                        await this.renderEpub(opened.data)
                    }
                } catch (e) {
                    this.errorMsg = errorMessage(e)
                    this.state = 'error'
                }
            },

            async renderEpub(data: ArrayBuffer) {
                const container = this.$refs.epubContainer as HTMLElement | undefined
                if (!container) { return }
                const book = ePub(data)

                const rendition = markRaw(book.renderTo(container, {
                    width: container.clientWidth,
                    height: container.clientHeight,
                    spread: 'none',
                }) as unknown as Rendition)
                this.rendition = rendition

                // The book is in an iframe: the page's CSS variables don't
                // reach it, so the theme's colours are passed as values.
                const style = getComputedStyle(document.documentElement)
                const bg = style.getPropertyValue('--bg').trim() || '#faf7f2'
                const fg = style.getPropertyValue('--text-primary').trim() || '#1a1a1a'
                const secondary = style.getPropertyValue('--text-secondary').trim() || '#555'
                const link = style.getPropertyValue('--accent-text').trim() || '#a64616'

                rendition.themes.default({
                    body: {
                        'background': `${bg} !important`,
                        'color': `${fg} !important`,
                        'font-family': "Baskerville, 'Book Antiqua', Georgia, Palatino, serif !important",
                        'line-height': '1.75 !important',
                        'font-size': '15px !important',
                    },
                    p: { 'margin': '0 0 0.9em !important', 'text-align': 'justify !important' },
                    h1: { 'color': `${fg} !important` },
                    h2: { 'color': `${fg} !important` },
                    h3: { 'color': `${secondary} !important` },
                    a: { 'color': `${link} !important` },
                })

                const locs = book.locations as unknown as Locations

                rendition.on('relocated', location => {
                    if (locs.total > 0) {
                        this.currentPage = locs.locationFromCfi(location.start.cfi) + 1
                        this.totalPages  = locs.total
                    } else {
                        // Locations not yet generated; show chapter-local page as fallback.
                        this.currentPage = location.start.displayed.page
                        this.totalPages  = location.start.displayed.total
                    }
                })
                rendition.on('keydown', e => this.onKey(e))

                await rendition.display()

                // Generate book-wide locations in the background (1024 chars ≈ one page).
                // Once done, refresh the pager with the true total. Without
                // them the pager keeps chapter-local pages: nothing to report.
                locs.generate(1024).then(() => {
                    this.totalPages = locs.total
                    const loc = this.rendition?.currentLocation()
                    if (loc?.start?.cfi) {
                        this.currentPage = locs.locationFromCfi(loc.start.cfi) + 1
                    }
                }).catch((e: unknown) => console.warn('[reader] could not paginate the whole book', e))
            },

            prevPage() { void this.rendition?.prev() },
            nextPage() { void this.rendition?.next() },

            /**
             * Keys from the dialog (BaseModal passes on what it doesn't
             * handle, only while the reader is the top dialog) and from the
             * book's iframe (Escape included: BaseModal never sees those).
             */
            onKey(e: KeyboardEvent) {
                if (e.key === 'ArrowLeft') {
                    e.preventDefault()
                    this.prevPage()
                } else if (e.key === 'ArrowRight') {
                    e.preventDefault()
                    this.nextPage()
                } else if (e.key === 'Escape') {
                    // Only from the iframe: BaseModal handles its own Escape.
                    this.$emit('close')
                }
            },
        },
    })
</script>

<style lang="scss" scoped>
.reader-shell {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background: var(--bg);
    outline: none;
}

// ── Header ────────────────────────────────────────────────────────────────────

.reader-header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-shrink: 0;
    height: 52px;
    padding: 0 20px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-elevated);
}

.reader-title {
    flex: 1;
    min-width: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
}

.reader-header .reader-close {
    border-radius: 50%;
    background: var(--bg-hover);

    &:hover {
        background: var(--accent-muted);
        color: var(--accent-text);
    }
}

// ── Loading / error state ─────────────────────────────────────────────────────

.reader-state { flex: 1; }

// ── EPUB Device ───────────────────────────────────────────────────────────────

.reader-epub-area {
    flex: 1;
    min-height: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background:
        linear-gradient(rgba(0, 0, 0, 0.72), rgba(0, 0, 0, 0.72)),
        url('@/assets/images/reading-bg.jpg') center / cover no-repeat;
    padding: 20px 32px;
}

// The e-reader drawn around the page keeps its own (device) colours in every theme.
.reader-device {
    height: 100%;
    max-height: 860px;
    width: 100%;
    max-width: 560px;
    background: linear-gradient(160deg, #242428 0%, #1a1a1d 100%);
    border-radius: 32px;
    padding: 20px 20px 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    position: relative;
    box-shadow:
        0 0 0 1.5px rgba(255, 255, 255, 0.07),
        0 0 0 3px rgba(0, 0, 0, 0.6),
        0 40px 100px rgba(0, 0, 0, 0.8),
        0 12px 32px rgba(0, 0, 0, 0.5),
        inset 0 1px 0 rgba(255, 255, 255, 0.1),
        inset 0 -1px 0 rgba(0, 0, 0, 0.4);

    // Side power button
    &::before {
        content: '';
        position: absolute;
        right: -4px;
        top: 68px;
        width: 4px;
        height: 32px;
        background: linear-gradient(180deg, #2e2e32 0%, #252528 100%);
        border-radius: 0 3px 3px 0;
        box-shadow: 0 40px 0 #252528;
    }
}

.device-screen-wrap {
    flex: 1;
    min-height: 0;
    border-radius: 10px;
    overflow: hidden;
    background: var(--bg);
    box-shadow:
        inset 0 0 0 1px rgba(0, 0, 0, 0.6),
        inset 0 2px 8px rgba(0, 0, 0, 0.3),
        0 0 0 1px rgba(255, 255, 255, 0.03);
}

.reader-epub {
    width: 100%;
    height: 100%;

    // epub.js styles its iframe inline; only !important wins over that.
    :deep(iframe) {
        display: block !important;
    }
}

.device-nav {
    flex-shrink: 0;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    height: 34px;
    padding: 0 4px;
}

.device-nav .device-btn {
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.06);
    color: rgba(255, 255, 255, 0.6);

    &.-right { justify-self: end; }

    &:hover {
        background: rgba(255, 255, 255, 0.13);
        color: rgba(255, 255, 255, 0.92);
    }
}

.reader-pager {
    font-size: var(--font-size-xs);
    color: rgba(255, 255, 255, 0.55);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.08em;
    text-align: center;
}

// ── PDF ───────────────────────────────────────────────────────────────────────

.reader-pdf {
    flex: 1;
    border: none;
    min-height: 0;
}
</style>
