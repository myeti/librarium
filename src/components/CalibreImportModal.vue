<template>
    <BaseModal
        :width="680"
        :title="$t('settings.calibre.title')"
        :close-on-backdrop="step === 'pick' || step === 'done'"
        class="calibre-modal"
        :class="`-${step}`"
        @close="maybeClose"
    >
        <!-- What went wrong, where it happened (the dialog stays on the step to retry). -->
        <p v-if="error" class="notice -bad" role="alert">
            <vue-feather type="alert-circle" size="14" aria-hidden="true" />
            <span>{{ error }}</span>
        </p>

        <!-- STEP: pick -->
        <template v-if="step === 'pick'">
            <div v-if="detected" class="auto-detect-row">
                <span class="auto-detect-label">{{ $t('settings.calibre.autoDetected') }}</span>
                <span class="auto-detect-path truncate" :title="detected">{{ detected }}</span>
                <BaseButton size="sm" variant="primary" class="use-detected" @click="usePath(detected)">
                    {{ $t('settings.calibre.use') }}
                </BaseButton>
            </div>
            <p v-else class="hint">{{ $t('settings.calibre.notFound') }}</p>
            <BaseButton variant="ghost" icon="folder" @click="browseFolder">
                {{ $t('settings.calibre.browse') }}
            </BaseButton>
        </template>

        <!-- STEP: loading -->
        <EmptyState
            v-else-if="step === 'loading'"
            loading
            heading-tag="p"
            :title="$t('settings.calibre.loading')"
            class="step-state"
        />

        <!-- STEP: browse -->
        <template v-else-if="step === 'browse'">
            <div class="browse-toolbar">
                <SearchInput
                    v-model="query"
                    :label="$t('settings.calibre.search')"
                    class="browse-search"
                />
                <span class="book-count">{{ $t('settings.calibre.booksFound', { n: filteredBooks.length }) }}</span>
                <BaseButton variant="ghost" size="sm" class="toggle-all" @click="toggleAll">
                    {{ allSelected ? $t('settings.calibre.deselectAll') : $t('settings.calibre.selectAll') }}
                </BaseButton>
            </div>

            <ul class="book-list">
                <li
                    v-for="book in filteredBooks"
                    :key="book.id"
                    class="book-row"
                    :class="{ '-checked': selected.has(book.id) }"
                >
                    <Checkbox :model-value="selected.has(book.id)" @update:model-value="toggle(book.id)">
                        <span class="book-info">
                            <span class="book-title">{{ displayTitle(book, $t) }}</span>
                            <span class="book-author">{{ displayAuthor(book, $t) }}</span>
                            <span v-if="book.series" class="book-series">{{ book.series }}<template v-if="book.seriesIndex !== null"> #{{ book.seriesIndex }}</template></span>
                        </span>
                        <Badge v-if="book.bestFormat" :tone="formatTone(book.bestFormat)" class="book-format">{{ book.bestFormat.toUpperCase() }}</Badge>
                        <Badge v-else tone="warning" icon="alert-triangle" class="book-format">{{ $t('settings.calibre.noSupportedFile') }}</Badge>
                    </Checkbox>
                </li>
            </ul>
            <EmptyState
                v-if="filteredBooks.length === 0"
                icon="search"
                :icon-size="28"
                heading-tag="p"
                :title="$t('settings.calibre.noMatch')"
                class="step-state"
            />
        </template>

        <!-- STEP: importing -->
        <div v-else-if="step === 'importing'" class="import-progress">
            <ProgressBar
                :value="calibreStore.progress ? calibreStore.progress.done : null"
                :max="progressTotal"
                size="md"
                :label="$t('settings.calibre.importing')"
                :value-text="progressText"
            />
            <p class="progress-count" aria-hidden="true">{{ progressText }}</p>
            <p v-if="currentTitle !== null" class="progress-title truncate">{{ currentTitle }}</p>
        </div>

        <!-- STEP: done -->
        <template v-else-if="step === 'done' && result">
            <p v-if="result.cancelled" class="notice -warn" role="status">
                {{ $t('settings.calibre.cancelled', { n: result.remaining }) }}
            </p>
            <p v-if="result.error" class="notice -bad" role="alert">
                {{ $t('settings.calibre.stopped', { error: result.error }) }}
                <template v-if="result.remaining > 0">{{ $t('settings.calibre.notImported', { n: result.remaining }) }}</template>
            </p>
            <ul class="summary">
                <li class="summary-row -good">
                    <vue-feather type="check-circle" size="16" aria-hidden="true" />
                    {{ $t('settings.calibre.imported', { n: result.imported.length }) }}
                </li>
                <li v-if="result.skipped.length" class="summary-row -warn">
                    <vue-feather type="skip-forward" size="16" aria-hidden="true" />
                    {{ $t('settings.calibre.skipped', { n: result.skipped.length }) }}
                </li>
                <li v-if="result.failed.length" class="summary-row -bad">
                    <vue-feather type="alert-circle" size="16" aria-hidden="true" />
                    {{ $t('settings.calibre.failed', { n: result.failed.length }) }}
                </li>
            </ul>

            <details v-if="result.skipped.length" class="details-list">
                <summary>{{ $t('settings.calibre.skippedList') }}</summary>
                <ul>
                    <li v-for="(b, i) in result.skipped" :key="i" class="skipped-book">
                        {{ displayTitle(b, $t) }} <span class="muted">— {{ displayAuthor(b, $t) }} ({{ skipReason(b) }})</span>
                    </li>
                </ul>
            </details>

            <details v-if="result.failed.length" class="details-list" open>
                <summary>{{ $t('settings.calibre.failedList') }}</summary>
                <ul>
                    <li v-for="(b, i) in result.failed" :key="i" class="failed-book">
                        {{ b.title || $t('book.unknownTitle') }} <span class="muted">— {{ b.reason }}</span>
                    </li>
                </ul>
            </details>
        </template>

        <template #footer>
            <BaseButton v-if="step === 'pick' || step === 'loading'" variant="ghost" @click="maybeClose">
                {{ $t('settings.calibre.close') }}
            </BaseButton>
            <template v-else-if="step === 'browse'">
                <BaseButton variant="ghost" icon="arrow-left" class="back" @click="back">{{ $t('settings.calibre.back') }}</BaseButton>
                <BaseButton
                    variant="primary"
                    class="start-import"
                    :disabled="selected.size === 0"
                    @click="startImport"
                >
                    {{ $t('settings.calibre.import', { n: selected.size }) }}
                </BaseButton>
            </template>
            <BaseButton
                v-else-if="step === 'importing'"
                variant="ghost"
                class="cancel-import"
                :disabled="calibreStore.cancelling"
                @click="cancelImport"
            >
                {{ calibreStore.cancelling ? $t('settings.calibre.cancelling') : $t('settings.calibre.cancelImport') }}
            </BaseButton>
            <BaseButton v-else variant="primary" data-autofocus @click="maybeClose">{{ $t('settings.calibre.close') }}</BaseButton>
        </template>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { open } from '@tauri-apps/plugin-dialog'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import Badge from '@/components/ui/Badge.vue'
    import Checkbox from '@/components/ui/Checkbox.vue'
    import EmptyState from '@/components/ui/EmptyState.vue'
    import ProgressBar from '@/components/ui/ProgressBar.vue'
    import SearchInput from '@/components/ui/SearchInput.vue'
    import type { BadgeTone } from '@/components/ui/types'
    import * as api from '@/services/api'
    import { useCalibreStore } from '@/stores/calibre'
    import { reportError } from '@/services/librarium-api'
    import { displayAuthor, displayTitle } from '@/utils/bookDisplay'
    import { errorCode, errorMessage } from '@/utils/errors'
    import type { CalibreBook, CalibreImportResult, CalibreSkippedBook } from '@/types'

    type Step = 'pick' | 'loading' | 'browse' | 'importing' | 'done'

    const FORMAT_TONES: readonly string[] = ['epub', 'pdf', 'mobi', 'azw3', 'cbz']

    interface CalibreImportData {
        step: Step
        /** The Calibre library found on this computer, if any. */
        detected: string | null
        calibrePath: string | null
        books: CalibreBook[]
        /** Ids of the Calibre books to import. */
        selected: Set<number>
        query: string
        result: CalibreImportResult | null
        /** Why the last action failed, shown at the top of the dialog. */
        error: string
        /** The user closed the modal during the import: it was cancelled, close once it has stopped. */
        closeWhenDone: boolean
    }

    /**
     * Imports books from a Calibre library: pick the library (auto-detected,
     * or browse), choose books, import (with progress and Cancel). The
     * backend skips books the library already holds and says why; the result
     * lists imported, skipped and failed books, and whether the import was
     * cancelled or stopped early.
     *
     * Failures (an unreadable library, an import that couldn't start) are
     * shown in the dialog, on the step where they happened, so the user can
     * retry. The import itself runs in the Calibre store, which also brings
     * the library's book list up to date. Closing the modal during an import
     * (Escape, ×) cancels it; the modal closes once the backend has stopped
     * (after the book it is copying), so the imported books are still
     * reported. A click outside closes only on the first and last steps,
     * never in the middle of choosing books.
     */
    export default defineComponent({
        name: 'CalibreImportModal',

        components: { BaseModal, BaseButton, Badge, Checkbox, EmptyState, ProgressBar, SearchInput },

        emits: ['close', 'imported'],

        data(): CalibreImportData {
            return {
                step: 'pick',
                detected: null,
                calibrePath: null,
                books: [],
                selected: new Set(),
                query: '',
                result: null,
                error: '',
                closeWhenDone: false,
            }
        },

        computed: {
            ...mapStores(useCalibreStore),

            /** The count of books requested, while the first progress event is on its way. */
            progressTotal(): number {
                return this.calibreStore.progress?.total ?? this.selected.size
            },

            /** "12 of 340 books". */
            progressText(): string {
                return this.$t('settings.calibre.progress', { n: this.progressTotal, done: this.calibreStore.progress?.done ?? 0 })
            },

            /** The book being imported (`null` between books). */
            currentTitle(): string | null {
                const title = this.calibreStore.progress?.currentTitle
                return title === null || title === undefined ? null : displayTitle({ title, author: '' }, this.$t)
            },

            filteredBooks(): CalibreBook[] {
                const q = this.query.trim().toLowerCase()
                if (!q) { return this.books }
                return this.books.filter(b =>
                    b.title.toLowerCase().includes(q) ||
                    b.author.toLowerCase().includes(q),
                )
            },

            allSelected(): boolean {
                return this.filteredBooks.length > 0 &&
                    this.filteredBooks.every(b => this.selected.has(b.id))
            },
        },

        async mounted() {
            try {
                this.detected = await api.detectCalibreLibrary()
            } catch (e) {
                reportError('CALIBRE_DETECT_FAILED', 'calibre.detect', e)
            }
        },

        methods: {
            displayTitle,
            displayAuthor,

            formatTone(format: string): BadgeTone {
                return (FORMAT_TONES.includes(format) ? format : 'neutral') as BadgeTone
            },

            skipReason(book: CalibreSkippedBook): string {
                switch (book.reason) {
                    case 'alreadyInLibrary':
                        return book.existingFilename
                            ? this.$t('settings.calibre.skipReasonAlreadyInLibraryAs', { file: book.existingFilename })
                            : this.$t('settings.calibre.skipReasonAlreadyInLibrary')
                    case 'noFile':
                        return this.$t('settings.calibre.skipReasonNoFile')
                    default:
                        return this.$t('settings.calibre.skipReasonUnsupportedFormat')
                }
            },

            /** Closes, except during an import: that is cancelled, and the modal closes once it has stopped. */
            maybeClose() {
                if (this.step !== 'importing') {
                    this.$emit('close')
                    return
                }
                this.closeWhenDone = true
                void this.cancelImport()
            },

            back() {
                this.error = ''
                this.step = 'pick'
            },

            async cancelImport() {
                this.error = ''
                try {
                    await this.calibreStore.cancel()
                } catch (e) {
                    this.closeWhenDone = false
                    this.error = this.$t('settings.calibre.cancelFailed', { error: errorMessage(e) })
                }
            },

            async browseFolder() {
                const path = await open({ directory: true, multiple: false, title: this.$t('settings.calibre.browseTitle') })
                if (path && typeof path === 'string') { await this.usePath(path) }
            },

            async usePath(path: string) {
                this.calibrePath = path
                this.error = ''
                this.step = 'loading'
                try {
                    this.books = await api.listCalibreBooks(path)
                    this.selected = new Set(this.books.map(b => b.id))
                    this.query = ''
                    this.step = 'browse'
                } catch (e) {
                    this.error = this.$t('settings.calibre.readFailed', { error: errorMessage(e) })
                    this.step = 'pick'
                }
            },

            toggle(id: number) {
                if (this.selected.has(id)) { this.selected.delete(id) } else { this.selected.add(id) }
            },

            toggleAll() {
                if (this.allSelected) {
                    this.filteredBooks.forEach(b => this.selected.delete(b.id))
                } else {
                    this.filteredBooks.forEach(b => this.selected.add(b.id))
                }
            },

            /**
             * Runs the import. Whatever the outcome, the imported books are
             * reported (`imported`; the store has already added them to the
             * library) — also when it was cancelled or stopped early.
             */
            async startImport() {
                const calibrePath = this.calibrePath
                if (!calibrePath || this.selected.size === 0) { return }
                this.error = ''
                this.step = 'importing'
                this.closeWhenDone = false
                try {
                    const result = await this.calibreStore.importBooks(calibrePath, Array.from(this.selected))
                    this.result = result
                    if (result.imported.length > 0) {
                        this.$emit('imported', result.imported)
                    }
                    this.step = 'done'
                } catch (e) {
                    this.error = errorCode(e) === 'CONFLICT'
                        ? this.$t('settings.calibre.alreadyRunning')
                        : this.$t('settings.calibre.importFailed', { error: errorMessage(e) })
                    this.step = 'browse'
                    // Nothing was imported: stay open with the reason, even if closing was asked.
                    this.closeWhenDone = false
                }
                if (this.closeWhenDone) {
                    this.$emit('close')
                }
            },
        },
    })
</script>

<style lang="scss" scoped>
// The browse step's list scrolls inside the body, under a fixed toolbar.
// (Global: the body is BaseModal's element, outside this component's scope.)
:global(.calibre-modal.-browse .ui-modal__body) {
    display: flex;
    flex-direction: column;
    padding: 0;
}

.notice {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin-bottom: var(--space-4);
    padding: 10px var(--space-3);
    border-radius: var(--radius-sm);
    font-size: var(--font-size-sm);
    word-break: break-word;

    svg { flex-shrink: 0; margin-top: 2px; }

    &.-warn { color: var(--warning); background: var(--warning-muted); }
    &.-bad  { color: var(--danger); background: var(--danger-muted); }

    .-browse & { margin: var(--space-3) 20px 0; }
}

.step-state { min-height: 200px; }

// ── Pick step ─────────────────────────────────────────────────────────────────

.auto-detect-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: var(--space-4);
    padding: 14px var(--space-4);
    border-radius: var(--radius-sm);
    background: var(--bg-hover);
}

.auto-detect-label {
    white-space: nowrap;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    color: var(--text-muted);
}

.auto-detect-path {
    flex: 1;
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
    color: var(--text-secondary);
}

.hint {
    margin-bottom: var(--space-4);
    font-size: var(--font-size-sm);
    color: var(--text-muted);
}

// ── Browse step ───────────────────────────────────────────────────────────────

.browse-toolbar {
    display: flex;
    flex-shrink: 0;
    align-items: center;
    gap: 10px;
    padding: var(--space-3) 20px;
    border-bottom: 1px solid var(--border);
}

.browse-search { flex: 1; }

.book-count {
    white-space: nowrap;
    font-size: var(--font-size-xs);
    color: var(--text-muted);
}

.book-list {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
}

.book-row {
    border-bottom: 1px solid var(--border);
    transition: background var(--transition);

    &:hover { background: var(--bg-hover); }
    &.-checked { background: color-mix(in srgb, var(--accent) 4%, transparent); }

    // The whole row is the checkbox's label.
    :deep(.ui-checkbox__row) {
        display: flex;
        gap: var(--space-3);
        width: 100%;
        padding: 9px 20px;
    }

    :deep(.ui-checkbox__label) {
        display: flex;
        flex: 1;
        align-items: center;
        gap: var(--space-3);
        min-width: 0;
    }
}

.book-info {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
}

.book-title {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--text-primary);
}

.book-author {
    font-size: var(--font-size-xs);
    color: var(--text-secondary);
}

.book-series {
    font-size: var(--font-size-xs);
    font-style: italic;
    color: var(--text-muted);
}

.book-format { flex-shrink: 0; }

// ── Importing step ────────────────────────────────────────────────────────────

.import-progress {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
    max-width: 420px;
    margin: 0 auto;
    padding: var(--space-6) 0;
    text-align: center;
}

.progress-count {
    font-variant-numeric: tabular-nums;
    color: var(--text-secondary);
}

.progress-title {
    font-style: italic;
    color: var(--text-muted);
}

// ── Done step ─────────────────────────────────────────────────────────────────

.summary {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 0 0 20px;
    padding: 0;
    list-style: none;
}

.summary-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-medium);

    &.-good { color: var(--success); }
    &.-warn { color: var(--text-secondary); }
    &.-bad  { color: var(--danger); }
}

.details-list {
    margin-top: var(--space-3);
    font-size: var(--font-size-sm);

    summary {
        margin-bottom: var(--space-2);
        font-weight: var(--font-weight-semibold);
        color: var(--text-secondary);
        cursor: pointer;
    }

    ul {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
        margin: 0;
        padding: 0;
        list-style: none;
    }

    li {
        padding: var(--space-1) 0;
        border-bottom: 1px solid var(--border);
        color: var(--text-secondary);
    }
}

.muted { color: var(--text-muted); }
</style>
