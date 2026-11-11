<template>
    <BaseModal
        :width="520"
        :dismissible="!applying"
        :describedby="subtitleId"
        @close="$emit('close')"
    >
        <template #title>
            {{ $t('tidyup.title') }}
            <Badge v-if="source" icon="database" class="tidyup-source">{{ sourceLabel }}</Badge>
        </template>

        <p :id="subtitleId" class="tidyup-subtitle">
            {{ $t('tidyup.subtitle', { title: displayTitle(book, $t) }) }}
        </p>
        <!-- The provider's title is quite different from the book's:
             it may be another book, so say so before anything is applied. -->
        <p v-if="!confident" class="tidyup-warning" role="note">
            <vue-feather type="alert-triangle" size="14" aria-hidden="true" />
            <span>{{ $t('tidyup.lowConfidence', { title: metadata.title ?? '' }) }}</span>
        </p>

        <!-- Cover row -->
        <div
            v-if="coverUrl && coverLoaded"
            class="tidyup-field tidyup-cover-row"
            :class="{ '-unchecked': !coverSelected }"
        >
            <Checkbox v-model="coverSelected" :disabled="applying">
                <span class="field-info">
                    <span class="field-label">{{ $t('tidyup.cover') }}</span>
                    <span v-if="!currentCoverSrc" class="field-current">{{ $t('tidyup.noCurrent') }}</span>
                </span>
                <span class="cover-preview-wrap">
                    <img
                        v-if="currentCoverSrc"
                        :src="currentCoverSrc"
                        class="cover-thumb -current"
                        :alt="$t('tidyup.currentCover')"
                    />
                    <span v-else class="cover-thumb -current -empty">
                        <vue-feather type="image" size="18" aria-hidden="true" />
                    </span>
                    <vue-feather type="arrow-right" size="14" class="cover-arrow" aria-hidden="true" />
                    <img
                        :src="coverUrl"
                        class="cover-thumb -new"
                        :alt="$t('tidyup.newCover')"
                        @error="coverLoaded = false"
                    />
                </span>
            </Checkbox>
        </div>

        <!-- Metadata fields -->
        <div class="tidyup-fields">
            <div
                v-for="field in visibleFields"
                :key="field.key"
                class="tidyup-field"
                :class="{ '-unchecked': !selected.has(field.key) }"
            >
                <Checkbox :model-value="selected.has(field.key)" :disabled="applying" @update:model-value="toggle(field.key)">
                    <span class="field-info">
                        <span class="field-label">{{ field.label }}</span>
                        <span class="field-new">{{ field.display }}</span>
                        <span v-if="field.same" class="field-current">{{ $t('tidyup.sameAsCurrent') }}</span>
                        <span v-else-if="field.current" class="field-current">{{ $t('tidyup.was', { value: field.current }) }}</span>
                    </span>
                </Checkbox>
            </div>

            <EmptyState
                v-if="visibleFields.length === 0 && (!coverUrl || !coverLoaded)"
                icon="search"
                :icon-size="28"
                heading-tag="p"
                :title="$t('tidyup.noInfo')"
                class="tidyup-empty"
            />
        </div>

        <!-- Saving failed: the review stays open with the choices, to retry or cancel. -->
        <p v-if="error" class="tidyup-error" role="alert">
            <vue-feather type="alert-circle" size="14" aria-hidden="true" />
            <span>{{ error }}</span>
        </p>

        <template #footer>
            <BaseButton variant="ghost" :disabled="applying" @click="$emit('close')">{{ $t('tidyup.cancel') }}</BaseButton>
            <BaseButton
                variant="primary"
                :disabled="selectedCount === 0"
                :loading="applying"
                @click="apply"
            >
                {{ $t('tidyup.apply', { n: selectedCount }) }}
            </BaseButton>
        </template>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import Badge from '@/components/ui/Badge.vue'
    import Checkbox from '@/components/ui/Checkbox.vue'
    import EmptyState from '@/components/ui/EmptyState.vue'
    import { useLibraryStore } from '@/stores/library'
    import { tidyupSourceLabel } from '@/services/tidyup'
    import { categoryLabel, type TextKey } from '@/i18n'
    import { displayTitle } from '@/utils/bookDisplay'
    import { uniqueId } from '@/utils/dom'
    import { errorMessage } from '@/utils/errors'
    import type { Book, BookMetadata, TidyupSubmit } from '@/types'

    /** The fields the review offers, with their translated names. Others in a result are ignored. */
    const FIELD_LABELS: Partial<Record<keyof BookMetadata, TextKey>> = {
        title: 'tidyup.fields.title',
        subtitle: 'tidyup.fields.subtitle',
        author: 'tidyup.fields.author',
        language: 'tidyup.fields.language',
        description: 'tidyup.fields.description',
        isbn: 'tidyup.fields.isbn',
        categories: 'tidyup.fields.categories',
        goodreadsUrl: 'tidyup.fields.goodreadsUrl',
    }

    /**
     * Fields listed even when the fetched value matches what the book already
     * has. Hiding an unchanged title made tidy-up look like it ignored the
     * field; listing it (unchecked) shows what the provider has to offer.
     */
    const ALWAYS_OFFERED: (keyof BookMetadata)[] = ['title']

    /** A metadata value: text, a number (series index, rating) or a list (categories). */
    type FieldValue = BookMetadata[keyof BookMetadata]

    interface ReviewField {
        key: keyof BookMetadata
        label: string
        display: string
        current: string
        same: boolean
    }

    function hasValue(v: FieldValue): boolean {
        return v !== undefined && v !== null && v !== '' && !(Array.isArray(v) && v.length === 0)
    }

    /** Comparable text of a value (categories in a stable order). */
    function valueText(value: FieldValue): string {
        if (Array.isArray(value)) { return [...value].sort().join(',') }
        return value === undefined ? '' : String(value).trim()
    }

    function displayValue(value: FieldValue): string {
        if (Array.isArray(value)) { return value.map(categoryLabel).join(', ') }
        return value === undefined ? '' : String(value)
    }

    function isDifferent(incoming: FieldValue, current: FieldValue): boolean {
        return valueText(incoming) !== valueText(current)
    }

    /**
     * Reviews the metadata found online for one book ("Find metadata
     * online"): each field the provider offers, with the book's current
     * value, ticked when it differs; and the proposed cover next to the
     * current one.
     *
     * Applying: with `submit`, the dialog runs it, shows a busy button, and
     * stays open with the reason if it fails (the parent closes it on
     * success, or it emits `close` itself once `submit` resolved). Without
     * `submit` it only emits `apply` with the ticked fields and the cover
     * URL, and the parent does the rest (the older contract).
     */
    export default defineComponent({
        name: 'TidyupReviewModal',

        components: { BaseModal, BaseButton, Badge, Checkbox, EmptyState },

        props: {
            book:     { type: Object as PropType<Book>, required: true },
            metadata: { type: Object as PropType<Partial<BookMetadata>>, required: true },
            source:   { type: String as PropType<string | null>, default: null },
            coverUrl: { type: String as PropType<string | null>, default: null },
            /** False when the result's title doesn't closely match the book's. */
            confident: { type: Boolean, default: true },
            /** Saves the choice (see TidyupSubmit). Optional: without it, `apply` is emitted. */
            submit:   { type: Function as PropType<TidyupSubmit | null>, default: null },
        },

        emits: {
            apply: (_metadata: Partial<BookMetadata>, _coverUrl: string | null) => true,
            close: () => true,
        },

        data() {
            // Only changed fields start checked — an unchanged title is offered
            // but not applied unless the user asks for it.
            const keys = (Object.keys(this.metadata) as (keyof BookMetadata)[])
                .filter(k => hasValue(this.metadata[k])
                    && k in FIELD_LABELS
                    && isDifferent(this.metadata[k], this.book.metadata[k]))
            return {
                selected: new Set<keyof BookMetadata>(keys),
                coverSelected: !!this.coverUrl,
                coverLoaded: !!this.coverUrl,
                applying: false,
                error: '',
                subtitleId: uniqueId('tidyup-subtitle'),
            }
        },

        computed: {
            ...mapStores(useLibraryStore),

            /** The book's current cover (the prop `coverUrl` is the proposed one). */
            currentCoverSrc(): string | null {
                return this.libraryStore.coverUrlOf(this.book.id)
            },

            sourceLabel(): string {
                return tidyupSourceLabel(this.source)
            },

            /** Ticked changes: the fields, plus the cover when it is offered and ticked. */
            selectedCount(): number {
                const coverCounts = this.coverSelected && !!this.coverUrl && this.coverLoaded
                return this.selected.size + (coverCounts ? 1 : 0)
            },

            visibleFields(): ReviewField[] {
                const fields: ReviewField[] = []
                for (const k of Object.keys(this.metadata) as (keyof BookMetadata)[]) {
                    const labelKey = FIELD_LABELS[k]
                    const incoming = this.metadata[k]
                    const current = this.book.metadata[k]
                    if (!labelKey || !hasValue(incoming)) { continue }
                    if (!ALWAYS_OFFERED.includes(k) && !isDifferent(incoming, current)) { continue }
                    fields.push({
                        key: k,
                        label: this.$t(labelKey),
                        display: displayValue(incoming),
                        current: hasValue(current) ? displayValue(current) : '',
                        same: !isDifferent(incoming, current),
                    })
                }
                return fields
            },
        },

        methods: {
            displayTitle,

            toggle(key: keyof BookMetadata) {
                if (this.selected.has(key)) {
                    this.selected.delete(key)
                } else {
                    this.selected.add(key)
                }
            },

            async apply() {
                if (this.applying || this.selectedCount === 0) { return }
                const result: Partial<BookMetadata> = {}
                for (const key of this.selected) {
                    (result as Record<string, unknown>)[key] = this.metadata[key]
                }
                // A proposed cover that failed to load isn't offered, so it isn't applied either.
                const cover = this.coverSelected && this.coverLoaded ? this.coverUrl : null
                if (!this.submit) {
                    this.$emit('apply', result, cover)
                    return
                }
                this.applying = true
                this.error = ''
                try {
                    await this.submit(result, cover)
                    this.applying = false
                    this.$emit('close')
                } catch (e) {
                    this.error = this.$t('errors.metadataSaveFailed', { error: errorMessage(e) })
                } finally {
                    this.applying = false
                }
            },
        },
    })
</script>

<style lang="scss" scoped>
.tidyup-source {
    margin-left: var(--space-1);
    font-weight: var(--font-weight-medium);
}

.tidyup-subtitle {
    margin-bottom: var(--space-4);
    font-size: var(--font-size-sm);
    line-height: 1.5;
    color: var(--text-secondary);
}

.tidyup-warning,
.tidyup-error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin: 0 0 14px;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    font-size: var(--font-size-xs);
    line-height: 1.5;

    svg { flex-shrink: 0; margin-top: 1px; }
}

.tidyup-warning {
    color: var(--warning);
    background: var(--warning-muted);
}

.tidyup-error {
    margin: 14px 0 0;
    color: var(--danger);
    background: var(--danger-muted);
    word-break: break-word;
}

.tidyup-fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
}

// A row: the whole bordered box is the checkbox's label (clickable).
.tidyup-field {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg);
    transition: background var(--transition), opacity var(--transition);

    &:hover { background: var(--bg-hover); }

    // Unticked fields fade, but stay readable (with the box, not colour alone).
    &.-unchecked .field-info,
    &.-unchecked .cover-preview-wrap { opacity: 0.6; }

    :deep(.ui-checkbox__row) {
        display: flex;
        align-items: flex-start;
        gap: var(--space-3);
        width: 100%;
        padding: 10px var(--space-3);
    }

    :deep(.ui-checkbox__input),
    :deep(.ui-checkbox__box) { margin-top: 2px; }

    :deep(.ui-checkbox__label) {
        display: flex;
        flex: 1;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-3);
        min-width: 0;
    }
}

.tidyup-cover-row {
    margin-bottom: var(--space-2);

    :deep(.ui-checkbox__row) { align-items: center; }
    :deep(.ui-checkbox__input),
    :deep(.ui-checkbox__box) { margin-top: 0; }
}

.cover-preview-wrap {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
}

.cover-thumb {
    width: 48px;
    height: 68px;
    object-fit: cover;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-hover);

    &.-empty {
        display: flex;
        align-items: center;
        justify-content: center;
        color: var(--text-muted);
    }

    &.-new { box-shadow: var(--shadow-sm); }
}

.cover-arrow {
    flex-shrink: 0;
    color: var(--text-muted);
}

.field-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    transition: opacity var(--transition);
}

.field-label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-bold);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    text-align: left;
    color: var(--text-muted);
}

.field-new {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    word-break: break-word;
    color: var(--text-primary);
}

.field-current {
    font-size: var(--font-size-xs);
    font-style: italic;
    color: var(--text-muted);
}

.tidyup-empty { min-height: 140px; }
</style>
