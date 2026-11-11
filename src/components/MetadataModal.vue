<template>
    <BaseModal
        :title="isBatch ? $t('metadata.editBatch', { n: books.length }) : $t('metadata.editTitle')"
        :size="isBatch ? 'xl' : 'md'"
        :dirty="dirty"
        :dismissible="!saving"
        close-on-backdrop
        initial-focus=".metadata-form input, .metadata-form textarea"
        @close="close"
    >
        <!-- A batch: the books on the left, in an order the user can change
             (the volumes can be numbered in it), the form on the right. -->
        <div class="metadata-layout" :class="{ '-batch': isBatch }">
            <aside v-if="isBatch" class="metadata-books" :aria-label="$t('metadata.selectedBooks')">
                <h3 class="metadata-books__title">{{ $t('metadata.selectedBooks') }}</h3>
                <BatchOrderList
                    v-model="order"
                    :numbering="numbering"
                    :start="numberStart ?? 1"
                    :disabled="saving"
                />
            </aside>

            <div class="metadata-main">
                <p v-if="isBatch" class="batch-notice" role="note">
                    <vue-feather type="info" size="14" aria-hidden="true" />
                    {{ $t('metadata.batchNotice') }}
                </p>

                <!-- Cover editor — single book only. Buttons, not a hover overlay: reachable from the keyboard. -->
                <div v-if="!isBatch" class="cover-editor">
                    <div class="cover-preview" :class="{ '-removed': coverAction === 'delete' }">
                        <img v-if="shownCover" :src="shownCover" class="cover-img" :alt="$t('metadata.coverAlt')" />
                        <div v-else class="cover-placeholder">
                            <vue-feather type="image" size="28" aria-hidden="true" />
                        </div>
                    </div>
                    <div class="cover-actions">
                        <BaseButton size="sm" icon="upload" :disabled="saving" @click="pickCover">
                            {{ $t('metadata.coverChange') }}
                        </BaseButton>
                        <BaseButton
                            v-if="coverPreview && coverAction !== 'delete'"
                            size="sm"
                            variant="ghost"
                            icon="trash-2"
                            :disabled="saving"
                            @click="deleteCover"
                        >
                            {{ $t('metadata.coverDelete') }}
                        </BaseButton>
                        <BaseButton v-if="coverAction !== 'none'" size="sm" variant="ghost" icon="rotate-ccw" :disabled="saving" @click="resetCover">
                            {{ $t('metadata.coverUndo') }}
                        </BaseButton>
                        <p v-if="coverAction !== 'none'" class="cover-note">
                            {{ coverAction === 'delete' ? $t('metadata.coverWillRemove') : $t('metadata.coverWillReplace') }}
                        </p>
                        <p v-if="coverError" class="cover-note -error" role="alert">{{ coverError }}</p>
                        <input
                            ref="fileInput"
                            type="file"
                            accept="image/jpeg,image/png,image/webp"
                            class="cover-file-input"
                            tabindex="-1"
                            aria-hidden="true"
                            @change="onFileSelected"
                        />
                    </div>
                </div>

                <form id="metadata-form" class="metadata-form" novalidate @submit.prevent="save">
                    <TextField
                        v-if="!isBatch"
                        v-model="form.title"
                        :label="$t('metadata.title')"
                        :placeholder="$t('metadata.titlePh')"
                        :disabled="saving"
                    />

                    <div class="form-row">
                        <TextField
                            v-model="form.author"
                            :label="$t('metadata.author')"
                            :placeholder="isBatch ? $t('metadata.keepCurrent') : $t('metadata.authorPh')"
                            :disabled="saving"
                        />
                        <TextField
                            v-model="form.language"
                            :label="$t('metadata.language')"
                            :placeholder="isBatch ? $t('metadata.keepCurrent') : 'en'"
                            :hint="$t('metadata.languageHint')"
                            :disabled="saving"
                        />
                    </div>

                    <div class="form-row">
                        <TextField
                            v-model="form.series"
                            :label="$t('metadata.series')"
                            :placeholder="isBatch ? $t('metadata.keepCurrent') : $t('metadata.seriesPh')"
                            :disabled="saving"
                        />
                        <TextField
                            v-if="!isBatch"
                            v-model="form.seriesIndex"
                            :label="$t('metadata.volume')"
                            inputmode="decimal"
                            placeholder="1"
                            :error="seriesIndexError"
                            :disabled="saving"
                        />
                    </div>

                    <!-- Batch, with a series: each book's volume from its place in the list. -->
                    <div v-if="isBatch" class="numbering">
                        <Checkbox
                            v-model="numberVolumes"
                            :label="$t('metadata.numberVolumes')"
                            :hint="hasSeries ? $t('metadata.numberVolumesHint') : $t('metadata.numberVolumesNeedsSeries')"
                            :disabled="saving || !hasSeries"
                        />
                        <TextField
                            v-if="numbering"
                            v-model="numberFrom"
                            class="numbering__start"
                            :label="$t('metadata.numberFrom')"
                            inputmode="decimal"
                            placeholder="1"
                            :error="numberFromError"
                            :disabled="saving"
                        />
                    </div>

                    <TextField
                        v-if="!isBatch"
                        v-model="form.isbn"
                        :label="$t('metadata.isbn')"
                        placeholder="978-…"
                        :disabled="saving"
                    />

                    <fieldset class="categories" :disabled="saving">
                        <legend class="categories-legend">
                            {{ $t('metadata.categories') }}
                            <span v-if="selectedCategories.length" class="label-count">{{ $t('metadata.categoriesCount', { n: selectedCategories.length }) }}</span>
                        </legend>
                        <div class="category-grid">
                            <!-- The <label> wraps its checkbox: they are linked without ids. -->
                            <label
                                v-for="cat in sortedCategories"
                                :key="cat"
                                class="cat-check"
                                :class="{ active: selectedCategories.includes(cat) }"
                            >
                                <input v-model="selectedCategories" type="checkbox" :value="cat" />
                                {{ categoryLabel(cat) }}
                            </label>
                        </div>
                    </fieldset>

                    <TextField
                        v-if="!isBatch"
                        v-model="form.description"
                        multiline
                        :rows="4"
                        :label="$t('metadata.description')"
                        :disabled="saving"
                    />
                </form>
            </div>
        </div>

        <template #footer>
            <div class="metadata-footer">
                <!-- The dialog stays open until the save succeeded, so a failure
                     (file locked, NAS offline…) is shown here and nothing typed is lost. -->
                <p v-if="saveError" class="save-error" role="alert">
                    <vue-feather type="alert-circle" size="14" aria-hidden="true" />
                    <span>{{ saveError }}</span>
                </p>
                <div class="metadata-actions">
                    <BaseButton variant="ghost" :disabled="saving" @click="close">{{ $t('metadata.cancel') }}</BaseButton>
                    <BaseButton type="submit" form="metadata-form" variant="primary" :loading="saving">
                        {{ saving ? $t('metadata.saving') : $t('metadata.save') }}
                    </BaseButton>
                </div>
            </div>
        </template>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import type { PropType } from 'vue'
    import { mapStores } from 'pinia'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import TextField from '@/components/ui/TextField.vue'
    import Checkbox from '@/components/ui/Checkbox.vue'
    import BatchOrderList from '@/components/library/BatchOrderList.vue'
    import type { Book, BookMetadata, CoverChange } from '@/types'
    import { CATEGORIES } from '@/constants/categories'
    import { categoryLabel } from '@/i18n'
    import { useLibraryStore } from '@/stores/library'
    import { errorMessage } from '@/utils/errors'
    import { compareText } from '@/utils/collator'

    export type { CoverChange } from '@/types'

    /**
     * Saves the edit. Provided by the parent, which knows whether this is a
     * single or a batch edit; it must reject (with a readable message) when
     * anything failed, so the dialog can stay open and show why. A batch's
     * `bookIds` are in the order the user chose; `seriesIndexes` (book id →
     * volume) is set when the volumes are numbered in that order.
     */
    export type MetadataSubmit = (
        bookIds: string[],
        metadata: Partial<BookMetadata>,
        cover: CoverChange,
        seriesIndexes?: Record<string, number>,
    ) => Promise<void>

    /** The text fields as typed (the volume too: '' is "none", parsed on save). */
    interface MetadataForm {
        title: string
        author: string
        series: string
        seriesIndex: string
        language: string
        isbn: string
        description: string
    }

    interface MetadataModalData {
        form: MetadataForm
        /** The values the dialog opened with: anything else is an unsaved edit. */
        initialForm: MetadataForm
        selectedCategories: string[]
        initialCategories: string[]
        /** The book's current cover, or a picked file (blob: URL). */
        coverPreview: string | null
        coverAction: CoverChange['action']
        coverBase64: string | null
        coverError: string
        saving: boolean
        saveError: string
        /** A batch's books, in the order the user chose. */
        order: Book[]
        /** Number the volumes in that order (a batch with a series). */
        numberVolumes: boolean
        /** The first volume number, as typed. */
        numberFrom: string
    }

    /**
     * The order a batch opens in: series by name, each by volume, then the
     * books outside a series by title. Numbering a series already in order
     * is one click.
     */
    function initialOrder(books: readonly Book[]): Book[] {
        const series = (b: Book) => b.metadata.series?.trim() ?? ''
        const volume = (b: Book) => b.metadata.seriesIndex ?? Number.POSITIVE_INFINITY
        return [...books].sort((a, b) => {
            const sa = series(a)
            const sb = series(b)
            if (!sa !== !sb) { return sa ? -1 : 1 } // books outside a series last
            return compareText(sa, sb) || volume(a) - volume(b) || compareText(a.metadata.title, b.metadata.title)
        })
    }

    function formOf(metadata: BookMetadata | null): MetadataForm {
        return {
            title:       metadata?.title ?? '',
            author:      metadata?.author ?? '',
            series:      metadata?.series ?? '',
            seriesIndex: metadata?.seriesIndex === undefined || metadata?.seriesIndex === null ? '' : String(metadata.seriesIndex),
            language:    metadata?.language ?? '',
            isbn:        metadata?.isbn ?? '',
            description: metadata?.description ?? '',
        }
    }

    /** '' → undefined; a number (0 and decimals allowed: "1.5"); NaN when it isn't one. */
    function parseIndex(text: string): number | undefined {
        const trimmed = text.trim().replace(',', '.')
        return trimmed === '' ? undefined : Number(trimmed)
    }

    /** Reads a picked image as base64 (what `set_book_cover` takes). */
    async function fileToBase64(file: File): Promise<string> {
        const bytes = new Uint8Array(await file.arrayBuffer())
        let binary = ''
        const CHUNK = 0x8000
        for (let i = 0; i < bytes.length; i += CHUNK) {
            binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK))
        }
        return btoa(binary)
    }

    /**
     * Edits one book's metadata and cover, or the shared fields of several
     * books (batch: only the fields filled in are applied). Saving is the
     * parent's `submit`; the dialog stays open on failure and shows why.
     *
     * Unsaved edits are never lost by accident: Escape, × and a click
     * outside ask "Discard your changes?" (BaseModal's `dirty` guard).
     * Cancel discards on purpose. Enter in a field saves.
     */
    export default defineComponent({
        name: 'MetadataModal',

        components: { BaseModal, BaseButton, TextField, Checkbox, BatchOrderList },

        props: {
            books: {
                type: Array as PropType<Book[]>,
                required: true,
            },
            submit: {
                type: Function as PropType<MetadataSubmit>,
                required: true,
            },
        },

        emits: ['close'],

        data(): MetadataModalData {
            const single = this.books.length === 1 ? this.books[0] : null
            const form = formOf(single?.metadata ?? null)
            const categories = [...(single?.metadata.categories ?? [])]
            return {
                form,
                initialForm: { ...form },
                selectedCategories: categories,
                initialCategories: [...categories],
                coverPreview: null,
                coverAction: 'none',
                coverBase64: null,
                coverError: '',
                saving: false,
                saveError: '',
                order: initialOrder(this.books),
                numberVolumes: false,
                numberFrom: '1',
            }
        },

        computed: {
            ...mapStores(useLibraryStore),

            isBatch(): boolean {
                return this.books.length > 1
            },

            sortedCategories(): string[] {
                const counts: Record<string, number> = {}
                for (const book of this.libraryStore.books) {
                    for (const cat of book.metadata.categories ?? []) {
                        counts[cat] = (counts[cat] ?? 0) + 1
                    }
                }
                return [...CATEGORIES].sort((a, b) => (counts[b] ?? 0) - (counts[a] ?? 0))
            },

            /** What the preview shows: nothing once the cover is marked for removal. */
            shownCover(): string | null {
                return this.coverAction === 'delete' ? null : this.coverPreview
            },

            /** A batch with a series typed in: its volumes can be numbered. */
            hasSeries(): boolean {
                return this.isBatch && this.form.series.trim() !== ''
            },

            /** The volumes are numbered on save. */
            numbering(): boolean {
                return this.hasSeries && this.numberVolumes
            },

            /** The first volume number (undefined when not a number ≥ 0). */
            numberStart(): number | undefined {
                const start = parseIndex(this.numberFrom)
                return start !== undefined && Number.isFinite(start) && start >= 0 ? start : undefined
            },

            numberFromError(): string {
                return this.numbering && this.numberStart === undefined ? this.$t('metadata.volumeInvalid') : ''
            },

            seriesIndexError(): string {
                const index = parseIndex(this.form.seriesIndex)
                return index !== undefined && (!Number.isFinite(index) || index < 0) ? this.$t('metadata.volumeInvalid') : ''
            },

            /** Something differs from what the dialog opened with. */
            dirty(): boolean {
                const fieldsChanged = (Object.keys(this.form) as (keyof MetadataForm)[])
                    .some(k => this.form[k].trim() !== this.initialForm[k].trim())
                const before = new Set(this.initialCategories)
                const categoriesChanged = this.selectedCategories.length !== before.size
                    || this.selectedCategories.some(c => !before.has(c))
                return fieldsChanged || categoriesChanged || this.coverAction !== 'none' || this.numbering
            },
        },

        mounted() {
            if (!this.isBatch && this.books[0]) {
                this.coverPreview = this.libraryStore.coverUrlOf(this.books[0].id)
            }
        },

        beforeUnmount() {
            this.revokePreview()
        },

        methods: {
            categoryLabel,

            close() {
                if (!this.saving) { this.$emit('close') }
            },

            payload(): Partial<BookMetadata> {
                const f = this.form
                const payload: Partial<BookMetadata> = {}
                if (this.isBatch) {
                    // Batch: an empty field means "don't touch".
                    if (f.author.trim())   { payload.author = f.author.trim() }
                    if (f.series.trim())   { payload.series = f.series.trim() }
                    if (f.language.trim()) { payload.language = f.language.trim() }
                    if (this.selectedCategories.length) { payload.categories = [...this.selectedCategories] }
                    return payload
                }
                // Single book: always write what's in the form, so clearing a field clears it.
                const series = f.series.trim()
                payload.title       = f.title.trim()
                payload.author      = f.author.trim()
                payload.series      = series || undefined
                payload.seriesIndex = parseIndex(f.seriesIndex) ?? (series ? 1 : undefined)
                payload.language    = f.language.trim() || undefined
                payload.isbn        = f.isbn.trim() || undefined
                payload.description = f.description.trim() || undefined
                payload.categories  = [...this.selectedCategories]
                return payload
            },

            /** With numbering: each book's volume, from its place in the list. */
            seriesIndexes(): Record<string, number> | undefined {
                const start = this.numberStart
                if (!this.numbering || start === undefined) { return undefined }
                return Object.fromEntries(this.order.map((book, i) => [book.id, start + i]))
            },

            async save() {
                if (this.saving || this.seriesIndexError || this.numberFromError) { return }
                this.saving = true
                this.saveError = ''
                const ids = this.isBatch ? this.order.map(b => b.id) : this.books.map(b => b.id)
                try {
                    await this.submit(ids, this.payload(), {
                        action: this.coverAction,
                        base64: this.coverBase64,
                    }, this.seriesIndexes())
                    this.saving = false
                    this.$emit('close')
                } catch (e) {
                    this.saveError = errorMessage(e) || this.$t('errors.generic')
                } finally {
                    this.saving = false
                }
            },

            pickCover() {
                (this.$refs.fileInput as HTMLInputElement | undefined)?.click()
            },

            /** Marks the book's cover for removal on save (a picked file, if any, is dropped). */
            deleteCover() {
                const current = this.libraryStore.coverUrlOf(this.books[0].id)
                if (!current) {
                    // Only a picked file to drop: the book has no cover to remove.
                    this.resetCover()
                    return
                }
                this.revokePreview()
                this.coverPreview = current
                this.coverAction = 'delete'
                this.coverBase64 = null
            },

            /** Back to the book's current cover. */
            resetCover() {
                this.revokePreview()
                this.coverPreview = this.libraryStore.coverUrlOf(this.books[0].id)
                this.coverAction = 'none'
                this.coverBase64 = null
                this.coverError = ''
            },

            /** Releases the object URL of a picked image (they live until revoked). */
            revokePreview() {
                if (this.coverPreview?.startsWith('blob:')) { URL.revokeObjectURL(this.coverPreview) }
            },

            async onFileSelected(e: Event) {
                const input = e.target as HTMLInputElement
                const file = input.files?.[0]
                input.value = ''   // picking the same file again still fires `change`
                if (!file) { return }
                this.coverError = ''
                try {
                    this.coverBase64 = await fileToBase64(file)
                } catch (err) {
                    this.coverError = this.$t('metadata.coverReadFailed', { error: errorMessage(err) })
                    return
                }
                this.revokePreview()
                this.coverPreview = URL.createObjectURL(file)
                this.coverAction = 'replace'
            },
        },
    })
</script>

<style lang="scss" scoped>
// ── Layout ────────────────────────────────────────────────────────────────────

// A batch: the books (their order) on the left, the form on the right.
.metadata-layout.-batch {
    display: grid;
    grid-template-columns: minmax(240px, 320px) minmax(0, 1fr);
    gap: var(--space-5);
    align-items: start;
}

.metadata-books {
    position: sticky;
    top: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: min(62vh, 560px);
    min-height: 0;
    padding-right: var(--space-4);
    border-right: 1px solid var(--border);
}

.metadata-books__title {
    font-family: var(--font-heading);
    font-size: var(--font-size-md);
    font-weight: var(--heading-weight);
    letter-spacing: var(--heading-tracking);
    color: var(--text-primary);
}

.metadata-main { min-width: 0; }

// Narrow window: the list above the form.
@media (max-width: 900px) {
    .metadata-layout.-batch { grid-template-columns: 1fr; }
    .metadata-books { position: static; max-height: 260px; padding-right: 0; border-right: none; }
}

.numbering {
    display: flex;
    align-items: flex-start;
    gap: var(--space-4);
    margin: var(--space-1) 0 var(--space-4);
}

.numbering__start {
    flex: 0 0 120px;
}

.batch-notice {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin-bottom: 20px;
    padding: 10px 14px;
    border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
    border-radius: var(--radius-sm);
    background: var(--accent-muted);
    font-size: var(--font-size-xs);
    color: var(--text-secondary);

    svg { flex-shrink: 0; margin-top: 1px; color: var(--accent-text); }
}

// ── Cover ─────────────────────────────────────────────────────────────────────

.cover-editor {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-4);
    margin-bottom: var(--space-5);
}

.cover-preview {
    position: relative;
    flex-shrink: 0;
    width: 100px;
    height: 140px;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-cover-placeholder);

    &.-removed { border-style: dashed; }
}

.cover-img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
}

.cover-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    color: var(--text-muted);
}

.cover-actions {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    max-width: 220px;
}

.cover-note {
    font-size: var(--font-size-xs);
    color: var(--text-muted);

    &.-error { color: var(--danger); }
}

.cover-file-input { display: none; }

// ── Form ──────────────────────────────────────────────────────────────────────

.metadata-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
}

.form-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
}

.categories {
    min-width: 0;
    margin: 0;
    padding: 0;
    border: none;
}

.categories-legend {
    margin-bottom: 6px;
    padding: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: .06em;
    text-transform: uppercase;
    color: var(--text-secondary);
}

.label-count {
    margin-left: 6px;
    font-weight: var(--font-weight-regular);
    letter-spacing: 0;
    text-transform: none;
    color: var(--accent-text);
}

.category-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 5px;
    max-height: 200px;
    overflow-y: auto;
    padding: 2px;
}

.cat-check {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-size: var(--font-size-xs);
    color: var(--text-secondary);
    cursor: pointer;
    user-select: none;
    transition: border-color var(--transition), background var(--transition), color var(--transition);

    input[type="checkbox"] {
        flex-shrink: 0;
        width: 13px;
        height: 13px;
        accent-color: var(--accent-strong);
        cursor: pointer;
    }

    &:hover {
        border-color: var(--border-hover);
        color: var(--text-primary);
    }

    &.active {
        border-color: color-mix(in srgb, var(--accent) 55%, transparent);
        background: var(--accent-muted);
        color: var(--accent-text);
        font-weight: var(--font-weight-medium);
    }
}

// ── Footer ────────────────────────────────────────────────────────────────────

.metadata-footer {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
}

.metadata-actions {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
}

.save-error {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
    padding: var(--space-2) 10px;
    border-radius: var(--radius-sm);
    background: var(--danger-muted);
    font-size: var(--font-size-xs);
    color: var(--danger);
    word-break: break-word;

    svg { flex-shrink: 0; margin-top: 1px; }
}
</style>
