<template>
    <div class="discover-view">
        <header class="discover-header">
            <div class="discover-header__top">
                <div>
                    <h1 class="discover-title">{{ $t('discover.title') }}</h1>
                    <p class="discover-intro">{{ $t('discover.intro') }}</p>
                </div>
                <RouterLink :to="{ path: '/settings', hash: '#catalogs' }" class="discover-manage">
                    <vue-feather type="sliders" size="14" aria-hidden="true" />
                    {{ $t('discover.manageCatalogs') }}
                </RouterLink>
            </div>

            <SearchInput
                ref="search"
                v-model="query"
                class="discover-search"
                :label="$t('discover.searchLabel')"
                :placeholder="$t('discover.searchPlaceholder')"
                :debounce="DEBOUNCE_MS"
            />

            <!-- Each catalog, and how its search is going (said in words, not by colour). -->
            <ul class="discover-catalogs" :aria-label="$t('discover.catalogsLabel')">
                <li v-for="p in allProviders" :key="p.id">
                    <Badge v-if="!p.isAvailable()" tone="neutral" icon="lock" :title="$t('discover.catalogLocked', { name: p.label })">{{ p.label }}</Badge>
                    <Badge v-else-if="providerLoading.has(p.id)" tone="accent" icon="loader" :title="$t('discover.catalogSearching', { name: p.label })">{{ p.label }}</Badge>
                    <Badge v-else-if="providerErrors[p.id]" tone="danger" icon="alert-circle" :title="$t('discover.catalogFailed', { name: p.label })">{{ p.label }}</Badge>
                    <Badge v-else tone="success" icon="check">{{ p.label }}</Badge>
                </li>
            </ul>

            <!-- A catalog that failed is reported as such, not as "no results". -->
            <ul v-if="failedProviders.length" class="discover-errors" role="status">
                <li v-for="p in failedProviders" :key="p.id">
                    <vue-feather type="alert-circle" size="14" aria-hidden="true" />
                    <span>
                        {{ $t('discover.catalogUnreachable', { name: p.label }) }}
                        <span class="discover-errors__detail">{{ providerErrors[p.id] }}</span>
                    </span>
                </li>
            </ul>
        </header>

        <div class="discover-body">
            <EmptyState
                v-if="!hasSearched"
                icon="compass"
                :title="$t('discover.title')"
                :description="$t('discover.searchHint')"
            />

            <!-- Every catalog failed: that's an outage, not an empty search. -->
            <EmptyState
                v-else-if="!isLoading && results.length === 0 && allSearchedFailed"
                alert
                icon="wifi-off"
                :title="$t('discover.allUnreachable')"
                :description="$t('discover.allUnreachableBody')"
            />

            <EmptyState
                v-else-if="!isLoading && results.length === 0"
                icon="inbox"
                :title="$t('discover.noResults')"
                :description="$t('discover.noResultsBody')"
            />

            <EmptyState v-else-if="results.length === 0" loading :description="$t('discover.searching')" />

            <template v-else>
                <p class="discover-count" role="status">{{ $t('discover.results', { n: results.length }) }}</p>
                <ul class="discover-grid">
                    <li v-for="book in results" :key="book.id" class="discover-card">
                        <div class="discover-card__cover">
                            <div class="discover-card__placeholder" aria-hidden="true">{{ initials(titleOf(book)) }}</div>
                            <img
                                v-if="book.coverUrl && !brokenCovers.has(book.id)"
                                :src="book.coverUrl"
                                class="discover-card__image"
                                :alt="$t('discover.coverAlt', { title: titleOf(book) })"
                                loading="lazy"
                                @error="brokenCovers.add(book.id)"
                            />
                            <div class="discover-card__badges">
                                <Badge :tone="book.format">{{ book.format.toUpperCase() }}</Badge>
                                <Badge v-if="book.language && book.language !== 'en'" tone="neutral">{{ book.language.toUpperCase() }}</Badge>
                            </div>
                        </div>

                        <div class="discover-card__info">
                            <p class="discover-card__title" :title="titleOf(book)">{{ titleOf(book) }}</p>
                            <p class="discover-card__author" :title="authorOf(book)">{{ authorOf(book) }}</p>
                            <p class="discover-card__source">{{ $t('discover.source', { name: sourceLabel(book.source) }) }}</p>
                        </div>

                        <div class="discover-card__action">
                            <!-- Said, not a dead button. -->
                            <p v-if="isInLibrary(book)" class="discover-card__owned">
                                <Badge tone="success" icon="check-circle">{{ $t('discover.inLibrary') }}</Badge>
                            </p>
                            <BaseButton v-else-if="downloading.has(book.id)" size="sm" block loading>
                                {{ $t('discover.downloading') }}
                            </BaseButton>
                            <BaseButton v-else-if="downloadErrors.has(book.id)" size="sm" block variant="danger" icon="rotate-ccw" @click="downloadBook(book)">
                                {{ $t('discover.downloadError') }}
                            </BaseButton>
                            <BaseButton v-else size="sm" block variant="primary" icon="download" @click="downloadBook(book)">
                                {{ $t('discover.download') }}
                            </BaseButton>
                        </div>
                    </li>
                </ul>
            </template>
        </div>

        <!-- Shows its own success toast (EpubConvertModal): nothing to add here. -->
        <EpubConvertModal
            v-if="pendingConversionBook"
            :books="[pendingConversionBook]"
            @converted="pendingConversionBook = null"
            @close="pendingConversionBook = null"
        />
    </div>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { RouterLink } from 'vue-router'
    import * as api from '@/services/api'
    import { STORE_PROVIDERS } from '@/services/store'
    import type { StoreBook, StoreProvider } from '@/services/store'
    import { mergeResults } from '@/services/store/merge'
    import { createOpdsCatalogProvider } from '@/services/store/providers/opdsCatalog'
    import { HttpFormatError, HttpStatusError } from '@/services/http'
    import { useLibraryStore, CONVERTIBLE_FORMATS } from '@/stores/library'
    import { useOpdsSourcesStore } from '@/stores/opdsSources'
    import { useNotificationsStore } from '@/stores/notifications'
    import { analytics, Analytics } from '@/services/analytics'
    import { bucketCount } from '@/services/privacy'
    import { errorMessage } from '@/utils/errors'
    import { normalizeForMatch } from '@/utils/text'
    import SearchInput from '@/components/ui/SearchInput.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import Badge from '@/components/ui/Badge.vue'
    import EmptyState from '@/components/ui/EmptyState.vue'
    import EpubConvertModal from '@/components/EpubConvertModal.vue'
    import type { Book } from '@/types'

    function initials(title: string): string {
        return title
            .split(/\s+/)
            .slice(0, 2)
            .map(w => w[0]?.toUpperCase() ?? '')
            .join('')
    }

    /** How a catalog result and a library book are compared ("In library"): title and author, case and accents ignored. */
    function libraryKey(title: string, author: string): string {
        return `${normalizeForMatch(title)}\u0000${normalizeForMatch(author)}`
    }

    /** Each search queries several remote catalogs: wait for a real pause in typing. */
    const DEBOUNCE_MS = 400

    /**
     * Discover (`/discover`; `/store` redirects here): searches free-ebook
     * catalogs (Project Gutenberg, Open Library, the OPDS catalogs added in
     * Settings) and downloads books into the current library.
     *
     * Each catalog answers on its own; their results are merged as they
     * arrive (services/store/merge.ts: one card per book, best matches
     * first, the same order whatever answered first). A catalog that fails
     * is listed with the reason, in the UI's language, and the others'
     * results still show; when all fail it is said as such, not as "no
     * results".
     */
    export default defineComponent({
        name: 'DiscoverView',

        components: { RouterLink, SearchInput, BaseButton, Badge, EmptyState, EpubConvertModal },

        data(): {
            /** A downloaded MOBI/AZW3 the user may convert to EPUB. */
            pendingConversionBook: Book | null
            query: string
            /** The search the results are for. */
            searchedQuery: string
            hasSearched: boolean
            /** Everything the catalogs answered for `searchedQuery` (merged for display by `results`). */
            rawResults: StoreBook[]
            /** Ids of the providers still searching. */
            providerLoading: Set<string>
            /** Provider id → why its last search failed (only failed providers are listed). */
            providerErrors: Record<string, string>
            /** Providers queried by the last search. */
            searchedIds: string[]
            /** Ids of the results being downloaded, and of those whose download failed. */
            downloading: Set<string>
            downloadErrors: Set<string>
            /** Results whose cover image failed to load (the placeholder shows). */
            brokenCovers: Set<string>
            abortController: AbortController | null
        } {
            return {
                pendingConversionBook: null,
                query:           '',
                searchedQuery:   '',
                hasSearched:     false,
                rawResults:      [],
                providerLoading: new Set(),
                providerErrors:  {},
                searchedIds:     [],
                downloading:     new Set(),
                downloadErrors:  new Set(),
                brokenCovers:    new Set(),
                abortController: null,
            }
        },

        computed: {
            ...mapStores(useLibraryStore, useOpdsSourcesStore, useNotificationsStore),

            DEBOUNCE_MS: () => DEBOUNCE_MS,

            allProviders(): StoreProvider[] {
                return [
                    ...STORE_PROVIDERS,
                    ...this.opdsSourcesStore.sources.map(createOpdsCatalogProvider),
                ]
            },

            /** One card per book, best matches first (stable while catalogs answer). */
            results(): StoreBook[] {
                return mergeResults(this.rawResults, this.searchedQuery, this.allProviders.map(p => p.id))
            },

            failedProviders(): StoreProvider[] {
                return this.allProviders.filter(p => this.providerErrors[p.id])
            },

            allSearchedFailed(): boolean {
                return this.searchedIds.length > 0 && this.searchedIds.every(id => this.providerErrors[id])
            },

            isLoading(): boolean {
                return this.providerLoading.size > 0
            },

            /** The library's books, keyed like `libraryKey`, built once per change of the library. */
            libraryKeys(): Set<string> {
                return new Set(this.libraryStore.books.map(b => libraryKey(b.metadata.title, b.metadata.author)))
            },
        },

        watch: {
            // SearchInput debounces typing (and commits on Enter).
            query(value: string) {
                if (value.trim()) {
                    void this.search(value)
                } else {
                    this.reset()
                }
            },
        },

        beforeUnmount() {
            this.abortController?.abort()
        },

        methods: {
            initials,

            titleOf(book: StoreBook): string {
                return book.title.trim() || this.$t('discover.unknownTitle')
            },

            authorOf(book: StoreBook): string {
                return book.author.trim() || this.$t('discover.unknownAuthor')
            },

            sourceLabel(sourceId: string): string {
                return this.allProviders.find(p => p.id === sourceId)?.label ?? sourceId
            },

            /** Why a catalog failed, in the UI's language (the details stay as the catalog gave them). */
            failureReason(e: unknown): string {
                if (e instanceof HttpFormatError) { return this.$t('discover.unsupportedFormat') }
                if (e instanceof HttpStatusError) { return this.$t('discover.requestFailed', { error: `HTTP ${e.status}` }) }
                return this.$t('discover.requestFailed', { error: errorMessage(e) || this.$t('errors.generic') })
            },

            reset() {
                this.abortController?.abort()
                this.abortController = null
                this.rawResults = []
                this.searchedQuery = ''
                this.hasSearched = false
                this.providerLoading = new Set()
                this.providerErrors = {}
                this.searchedIds = []
            },

            async search(text: string) {
                const query = text.trim()
                if (!query) { return }

                this.abortController?.abort()
                const controller = new AbortController()
                this.abortController = controller
                const { signal } = controller

                this.rawResults = []
                this.searchedQuery = query
                this.hasSearched = true
                this.providerErrors = {}
                this.downloadErrors = new Set()
                this.brokenCovers = new Set()

                const providers = this.allProviders.filter(p => p.isAvailable())
                this.searchedIds = providers.map(p => p.id)
                this.providerLoading = new Set(this.searchedIds)

                await Promise.allSettled(providers.map(async (p: StoreProvider) => {
                    try {
                        const books = await p.search(query, signal)
                        if (signal.aborted) { return }
                        this.rawResults = [...this.rawResults, ...books]
                    } catch (e: unknown) {
                        const aborted = e instanceof DOMException && e.name === 'AbortError'
                        if (!aborted && !signal.aborted) {
                            console.error(`[discover] ${p.id} error:`, e)
                            analytics.track('error', { component: 'store_search', error_type: 'provider_failed' })
                            this.providerErrors = { ...this.providerErrors, [p.id]: this.failureReason(e) }
                        }
                    } finally {
                        if (!signal.aborted) {
                            this.providerLoading = new Set([...this.providerLoading].filter(id => id !== p.id))
                        }
                    }
                }))

                if (signal.aborted) { return }
                analytics.track('store_search', {
                    provider_count_bucket: bucketCount(providers.length),
                    has_opds_source: this.opdsSourcesStore.sources.length > 0,
                    result_count_bucket: Analytics.bucketResultCount(this.results.length),
                })
            },

            isInLibrary(book: StoreBook): boolean {
                return this.libraryKeys.has(libraryKey(book.title, book.author))
            },

            async downloadBook(book: StoreBook) {
                const libraryPath = this.libraryStore.libraryPath
                if (!libraryPath) { return }
                // The download is for the library open now; if the user switches
                // meanwhile, the book stays in that library and isn't listed in the new one.
                const session = this.libraryStore.session
                this.downloading = new Set([...this.downloading, book.id])
                this.downloadErrors = new Set([...this.downloadErrors].filter(id => id !== book.id))

                const safe = (s: string) => s.replace(/[/\\:*?"<>|]/g, '_').trim()
                const filename = `${safe(this.authorOf(book))} - ${safe(this.titleOf(book))}.${book.format}`

                try {
                    const imported = await api.downloadAndImport(book.downloadUrl, filename, libraryPath)
                    // A user catalog is sent as 'opds', not 'opds-<its id>': that id is
                    // random and persistent, so it would tie events together.
                    analytics.track('store_download', { format: book.format, source: book.source.startsWith('opds-') ? 'opds' : book.source })
                    if (session === this.libraryStore.session) {
                        this.libraryStore.addBooks([imported])
                        if (CONVERTIBLE_FORMATS.includes(imported.format.toLowerCase())) {
                            this.pendingConversionBook = imported
                        }
                    }
                } catch (e) {
                    console.error('[discover] download failed:', e)
                    analytics.track('error', { component: 'store_download', error_type: 'download_failed' })
                    this.downloadErrors = new Set([...this.downloadErrors, book.id])
                    this.notificationsStore.push('error', this.$t('discover.downloadFailed', { title: this.titleOf(book), error: errorMessage(e) }))
                } finally {
                    this.downloading = new Set([...this.downloading].filter(id => id !== book.id))
                }
            },
        },
    })
</script>

<style lang="scss" scoped>
.discover-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
}

// ── Header ────────────────────────────────────────────────────────────────────

.discover-header {
    flex-shrink: 0;
    margin: 20px 28px 0;
    padding: 22px 26px 18px;
    border: 1px solid var(--hairline);
    border-radius: var(--radius-lg);
    background:
        radial-gradient(80% 120% at 0% 0%, color-mix(in srgb, var(--accent) 10%, transparent), transparent 70%),
        var(--glass);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
}

.discover-header__top {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
}

.discover-title {
    margin: 0;
    font-family: var(--font-display);
    font-size: 26px;
    font-weight: var(--display-weight);
    letter-spacing: var(--display-tracking);
    text-transform: uppercase;
    line-height: 1.1;
    color: var(--text-primary);
}

.discover-intro {
    margin: var(--space-1) 0 0;
    font-size: var(--font-size-sm);
    color: var(--text-secondary);
}

.discover-manage {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    flex-shrink: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    color: var(--accent-text);

    &:hover { text-decoration: underline; }
}

.discover-catalogs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;

    // The searching badge's icon turns.
    :deep(.-accent svg) { animation: librarium-spin 1s linear infinite; }
}

.discover-errors {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--font-size-xs);
    color: var(--danger);

    li { display: flex; align-items: flex-start; gap: 6px; }
    svg { flex-shrink: 0; margin-top: 2px; }
}

.discover-errors__detail {
    color: var(--text-secondary);
}

// ── Body ──────────────────────────────────────────────────────────────────────

.discover-body {
    flex: 1;
    overflow-y: auto;
    padding: 20px 32px 32px;
}

.discover-count {
    margin: 0 0 var(--space-3);
    font-size: var(--font-size-xs);
    color: var(--text-secondary);
}

// As many columns as fit: no fixed count to fill with placeholders.
.discover-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--space-4);
    margin: 0;
    padding: 0;
    list-style: none;
}

.discover-card {
    display: flex;
    flex-direction: column;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
    transition: box-shadow var(--transition), border-color var(--transition);

    &:hover { box-shadow: var(--shadow); border-color: var(--border-hover); }
}

.discover-card__cover {
    position: relative;
    aspect-ratio: 2 / 3;
    background: var(--bg-cover-placeholder);
    overflow: hidden;
    flex-shrink: 0;
}

.discover-card__image {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
}

.discover-card__placeholder {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: var(--font-serif);
    font-size: 28px;
    font-weight: var(--font-weight-bold);
    color: color-mix(in srgb, var(--accent-text) 45%, transparent);
}

.discover-card__badges {
    position: absolute;
    bottom: 6px;
    left: 6px;
    display: flex;
    gap: 4px;
}

.discover-card__info {
    flex: 1;
    padding: 10px 12px 6px;
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
}

.discover-card__title {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    line-height: var(--line-height-tight);
    color: var(--text-primary);
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
}

.discover-card__author {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--text-secondary);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
}

.discover-card__source {
    margin: 2px 0 0;
    font-size: var(--font-size-xs);
    color: var(--text-muted);
}

.discover-card__action {
    padding: 8px 10px;
    border-top: 1px solid var(--border);
}

.discover-card__owned {
    display: flex;
    justify-content: center;
    align-items: center;
    min-height: var(--control-height-sm);
    margin: 0;
}
</style>
