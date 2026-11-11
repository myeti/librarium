<template>
    <BaseModal
        size="lg"
        :title="$t('duplicates.title')"
        :dismissible="!busy"
        initial-focus="input.dup-radio:checked"
        @close="close"
    >
        <p ref="intro" class="dup-intro">{{ $t('duplicates.intro', { n: groups.length }) }}</p>

        <!-- Copies that couldn't be removed: their groups stay listed, with this reason. -->
        <p v-if="failure" class="dup-notice" role="alert">
            <vue-feather type="alert-circle" size="14" aria-hidden="true" />
            <span>{{ failure }}</span>
        </p>

        <EmptyState
            v-if="groups.length === 0"
            icon="check-circle"
            :icon-size="32"
            heading-tag="p"
            :title="$t('duplicates.none')"
            class="dup-empty"
        />

        <section
            v-for="(group, gi) in groups"
            :key="group.key"
            class="dup-group"
            :class="{ '-skipped': keptByGroup.get(group.key) === null }"
            :aria-labelledby="`${uid}-g${gi}`"
        >
            <header class="dup-group-header">
                <h3 :id="`${uid}-g${gi}`" class="dup-label">{{ $t('duplicates.group', { n: gi + 1, total: groups.length }) }}</h3>
                <span class="dup-reason">{{ groupReason(group) }}</span>
                <span class="dup-decision">{{ decisionLabel(group) }}</span>
            </header>

            <!-- One choice per group: a copy to keep, or keep them all (skip). Native radios: arrows move between them. -->
            <div role="radiogroup" class="dup-books" :aria-labelledby="`${uid}-g${gi}`">
                <label
                    v-for="book in group.books"
                    :key="book.id"
                    class="dup-book"
                    :class="{ '-kept': isKept(group, book.id), '-extra': isExtra(group, book.id) }"
                >
                    <input
                        type="radio"
                        class="dup-radio"
                        :name="`${uid}-${gi}`"
                        :checked="isKept(group, book.id)"
                        :disabled="busy"
                        @change="keep(group, book.id)"
                    />
                    <span class="radio-dot" aria-hidden="true"></span>

                    <span class="dup-cover">
                        <img
                            v-if="coverSrc(book)"
                            :src="coverSrc(book) ?? undefined"
                            alt=""
                            loading="lazy"
                            decoding="async"
                        />
                        <vue-feather v-else type="book" size="14" aria-hidden="true" />
                    </span>

                    <span class="dup-info">
                        <span class="dup-title-row">
                            <span class="dup-title">{{ displayTitle(book, $t) }}</span>
                            <Badge v-if="suggestions.get(group.key)?.bookId === book.id" tone="accent" icon="star" class="dup-suggested">
                                {{ $t('duplicates.suggested') }}
                            </Badge>
                            <span v-if="suggestions.get(group.key)?.bookId === book.id && suggestionReason(group, book)" class="dup-why">
                                {{ suggestionReason(group, book) }}
                            </span>
                        </span>
                        <span class="dup-author">{{ displayAuthor(book, $t) }}</span>
                        <span class="dup-facts">
                            <Badge :tone="formatTone(book.format)">{{ book.format.toUpperCase() }}</Badge>
                            <span>{{ fmtSize(book.sizeBytes) }}</span>
                            <span aria-hidden="true">·</span>
                            <span>{{ $t('duplicates.added', { date: fmtDate(book.dateAdded) }) }}</span>
                            <span aria-hidden="true">·</span>
                            <span>{{ $t('duplicates.details', { filled: factsOf(book).filled, total: factsOf(book).total }) }}</span>
                        </span>
                        <span v-if="factsOf(book).missing.length" class="dup-missing">
                            {{ $t('duplicates.missing', { fields: missingLabel(book) }) }}
                        </span>
                        <span class="dup-path" :title="book.path">{{ shortPath(book.path) }}</span>
                    </span>

                    <!-- What happens to this copy: text and icon, not colour alone. -->
                    <Badge v-if="isKept(group, book.id)" tone="success" icon="check" class="dup-fate">{{ $t('duplicates.keep') }}</Badge>
                    <Badge v-else-if="isExtra(group, book.id)" tone="danger" icon="minus-circle" class="dup-fate">{{ $t('duplicates.extra') }}</Badge>
                </label>

                <label class="dup-book -skip" :class="{ '-kept': keptByGroup.get(group.key) === null }">
                    <input
                        type="radio"
                        class="dup-radio"
                        :name="`${uid}-${gi}`"
                        :checked="keptByGroup.get(group.key) === null"
                        :disabled="busy"
                        @change="skip(group)"
                    />
                    <span class="radio-dot" aria-hidden="true"></span>
                    <span class="dup-skip-text">{{ $t('duplicates.keepAll') }}</span>
                </label>
            </div>
        </section>

        <ConfirmDialog
            v-if="pending"
            :danger="pending.action === 'trash'"
            :title="pending.action === 'trash'
                ? $t('duplicates.trashTitle', { n: pending.ids.length })
                : $t('duplicates.forgetTitle', { n: pending.ids.length })"
            :message="pending.action === 'trash' ? $t('library.trashConfirmBody') : $t('library.forgetConfirmBody')"
            :details="pending.action === 'trash' ? $t('library.trashConfirmDetails') : $t('library.forgetConfirmDetails')"
            :confirm-label="pending.action === 'trash' ? $t('library.trashConfirm') : $t('library.forgetConfirm')"
            :busy="busy"
            :error="confirmError"
            @confirm="run"
            @cancel="pending = null"
        />

        <template #footer>
            <div class="dup-footer">
                <p class="dup-summary" aria-live="polite">{{ summary }}</p>
                <div class="dup-actions">
                    <BaseButton variant="ghost" :disabled="busy" @click="close">{{ $t('common.close') }}</BaseButton>
                    <BaseButton :disabled="toRemove.length === 0 || busy" class="dup-forget" @click="ask('forget')">
                        {{ $t('duplicates.forget', { n: toRemove.length }) }}
                    </BaseButton>
                    <BaseButton variant="danger" icon="trash-2" :disabled="toRemove.length === 0 || busy" class="dup-trash" @click="ask('trash')">
                        {{ $t('duplicates.trash', { n: toRemove.length }) }}
                    </BaseButton>
                </div>
            </div>
        </template>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import Badge from '@/components/ui/Badge.vue'
    import EmptyState from '@/components/ui/EmptyState.vue'
    import ConfirmDialog from '@/components/ConfirmDialog.vue'
    import type { BadgeTone } from '@/components/ui/types'
    import { useLibraryStore } from '@/stores/library'
    import { useNotificationsStore } from '@/stores/notifications'
    import type { DuplicateGroup } from '@/services/duplicates'
    import {
        copiesToRemove, copyFacts, suggestKeeper,
        type CompletenessField, type CopyFacts, type Suggestion,
    } from '@/services/duplicateReview'
    import { reportError } from '@/services/librarium-api'
    import type { TextKey } from '@/i18n'
    import { displayAuthor, displayTitle } from '@/utils/bookDisplay'
    import { uniqueId } from '@/utils/dom'
    import { errorMessage } from '@/utils/errors'
    import { fmtDate, fmtSize } from '@/utils/format'
    import type { Book } from '@/types'

    type Action = 'trash' | 'forget'

    const FORMAT_TONES: readonly string[] = ['epub', 'pdf', 'mobi', 'azw3', 'cbz']

    /** How each missing detail is named ("Missing: ISBN, cover"). */
    const FIELD_LABELS: Record<CompletenessField, TextKey> = {
        author:      'metadata.author',
        series:      'metadata.series',
        language:    'metadata.language',
        isbn:        'metadata.isbn',
        description: 'metadata.description',
        categories:  'metadata.categories',
        cover:       'tidyup.cover',
    }

    interface DuplicatesData {
        uid: string
        /** Groups listed when the dialog opened: those start on the suggested copy. */
        initialKeys: Set<string>
        /** The user's choice per group key: the copy to keep, or null for "keep all". */
        choices: Record<string, string | null>
        /** The action waiting for confirmation, with the copies it applies to. */
        pending: { action: Action, ids: string[] } | null
        busy: boolean
        /** Why the confirmed action failed (shown in the confirmation, which stays open). */
        confirmError: string
        /** Copies the last action couldn't handle (shown at the top). */
        failure: string
    }

    /**
     * The Duplicates review: groups of probable duplicates (same ISBN, or
     * same title and author), each copy with what tells them apart (format,
     * size, date added, cover, how complete its details are), and the copy
     * Librarium suggests keeping (services/duplicateReview.ts).
     *
     * In each group the user keeps one copy, or keeps them all (skips the
     * group: nothing happens to it). The extra copies of every decided group
     * can then be moved to the Trash, or removed from the library with their
     * files left on disk; both are confirmed first and run here, through the
     * library store. The groups are the store's live `duplicateGroups`:
     * handled copies disappear, and the dialog closes itself when no group
     * is left. Copies that failed stay, with the reason at the top.
     *
     * Choices are kept by group key and checked against the group's current
     * books, so a recomputed group can't make a choice point at another
     * book. Groups the dialog opened with start on the suggested copy; a
     * group that shows up later starts skipped (nothing is removed from a
     * group the user hasn't seen).
     */
    export default defineComponent({
        name: 'DuplicatesModal',

        components: { BaseModal, BaseButton, Badge, EmptyState, ConfirmDialog },

        props: {
            groups: {
                type: Array as PropType<DuplicateGroup[]>,
                required: true,
            },
        },

        emits: ['close'],

        data(): DuplicatesData {
            return {
                uid: uniqueId('dup'),
                initialKeys: new Set(this.groups.map(g => g.key)),
                choices: {},
                pending: null,
                busy: false,
                confirmError: '',
                failure: '',
            }
        },

        computed: {
            ...mapStores(useLibraryStore, useNotificationsStore),

            /** The suggested copy of each group (live: it follows covers as they load). */
            suggestions(): Map<string, Suggestion | null> {
                const hasCover = (id: string) => this.libraryStore.coverOf(id) !== null
                return new Map(this.groups.map(g => [g.key, suggestKeeper(g, hasCover)]))
            },

            /** The copy kept in each group, or null when the group is skipped. */
            keptByGroup(): Map<string, string | null> {
                const kept = new Map<string, string | null>()
                for (const group of this.groups) {
                    let id: string | null = null
                    if (group.key in this.choices) {
                        id = this.choices[group.key]
                    } else if (this.initialKeys.has(group.key)) {
                        id = this.suggestions.get(group.key)?.bookId ?? null
                    }
                    // A choice whose book left the group means nothing any more.
                    kept.set(group.key, id && group.books.some(b => b.id === id) ? id : null)
                }
                return kept
            },

            toRemove(): string[] {
                const keptIds = new Set([...this.keptByGroup.values()].filter((id): id is string => id !== null))
                return copiesToRemove(this.groups, keptIds)
            },

            skippedCount(): number {
                return [...this.keptByGroup.values()].filter(id => id === null).length
            },

            /** "3 extra copies selected · 1 group skipped". */
            summary(): string {
                if (this.groups.length === 0) { return '' }
                const parts = [this.$t('duplicates.summaryCopies', { n: this.toRemove.length })]
                if (this.skippedCount > 0) { parts.push(this.$t('duplicates.summarySkipped', { n: this.skippedCount })) }
                return parts.join(' · ')
            },

            /** Facts per book id, computed once per render pass. */
            facts(): Map<string, CopyFacts> {
                return new Map(this.groups.flatMap(g => g.books).map(b => [b.id, copyFacts(b, this.libraryStore.coverOf(b.id) !== null)]))
            },
        },

        mounted() {
            // Covers help tell the copies apart; most were never on screen.
            this.groups.flatMap(g => g.books).forEach(book => this.libraryStore.requestCover(book.id))
        },

        methods: {
            displayTitle,
            displayAuthor,
            fmtDate,
            fmtSize,

            factsOf(book: Book): CopyFacts {
                return this.facts.get(book.id) ?? copyFacts(book, false)
            },

            missingLabel(book: Book): string {
                return this.factsOf(book).missing.map(f => this.$t(FIELD_LABELS[f])).join(', ')
            },

            formatTone(format: string): BadgeTone {
                return (FORMAT_TONES.includes(format) ? format : 'neutral') as BadgeTone
            },

            coverSrc(book: Book): string | null {
                return this.libraryStore.coverUrlOf(book.id)
            },

            isKept(group: DuplicateGroup, bookId: string): boolean {
                return this.keptByGroup.get(group.key) === bookId
            },

            /** An extra copy: its group keeps another one. */
            isExtra(group: DuplicateGroup, bookId: string): boolean {
                const kept = this.keptByGroup.get(group.key)
                return !!kept && kept !== bookId
            },

            keep(group: DuplicateGroup, bookId: string) {
                this.choices[group.key] = bookId
            },

            skip(group: DuplicateGroup) {
                this.choices[group.key] = null
            },

            groupReason(group: DuplicateGroup): string {
                if (group.reason === 'isbn' && group.isbn) { return this.$t('duplicates.sameIsbn', { isbn: group.isbn }) }
                return this.$t('duplicates.sameTitleAuthor')
            },

            decisionLabel(group: DuplicateGroup): string {
                const kept = this.keptByGroup.get(group.key)
                if (!kept) { return this.$t('duplicates.skipped') }
                return this.$t('duplicates.extraCount', { n: group.books.length - 1 })
            },

            suggestionReason(group: DuplicateGroup, book: Book): string {
                switch (this.suggestions.get(group.key)?.reason) {
                    case 'complete': return this.$t('duplicates.why.complete')
                    case 'format':   return this.$t('duplicates.why.format', { format: book.format.toUpperCase() })
                    case 'cover':    return this.$t('duplicates.why.cover')
                    case 'oldest':   return this.$t('duplicates.why.oldest')
                    default:         return ''
                }
            },

            shortPath(path: string): string {
                const parts = path.split(/[\\/]/)
                return parts.slice(-2).join('/')
            },

            close() {
                if (!this.busy) { this.$emit('close') }
            },

            /** Opens the confirmation for the extra copies chosen right now. */
            ask(action: Action) {
                if (this.toRemove.length === 0 || this.busy) { return }
                this.confirmError = ''
                this.pending = { action, ids: [...this.toRemove] }
            },

            /** The confirmed action. The confirmation stays open (with the reason) if nothing could be done. */
            async run() {
                if (!this.pending || this.busy) { return }
                const { action, ids } = this.pending
                this.busy = true
                this.confirmError = ''
                try {
                    if (action === 'trash') {
                        const result = await this.libraryStore.deleteBooks(ids)
                        this.report(result.deleted.length, result.failures, action, result.libraryTrash)
                    } else {
                        const result = await this.libraryStore.forgetBooks(ids)
                        this.report(result.forgotten.length, result.failures, action, null)
                    }
                    this.pending = null
                } catch (e) {
                    reportError(action === 'trash' ? 'DELETE_FAILED' : 'FORGET_FAILED', action === 'trash' ? 'duplicates.trash' : 'duplicates.forget', e)
                    this.confirmError = action === 'trash'
                        ? this.$t('errors.deleteFailed', { error: errorMessage(e) })
                        : this.$t('errors.forgetFailed', { error: errorMessage(e) })
                    return
                } finally {
                    this.busy = false
                }
                // The groups are recomputed from the library: once none is left, the review is over.
                await this.$nextTick()
                if (this.groups.length === 0) {
                    this.$emit('close')
                } else {
                    this.keepFocusInside()
                }
            },

            /**
             * The confirmation gave focus back to the button that opened it,
             * which may now be disabled (nothing left to remove): focus is
             * then lost to <body>. Put it back on a choice in this dialog.
             */
            keepFocusInside() {
                const panel = (this.$refs.intro as HTMLElement | undefined)?.closest<HTMLElement>('.ui-modal__panel')
                if (!panel || panel.contains(document.activeElement)) { return }
                const target = panel.querySelector<HTMLElement>('input.dup-radio:checked') ?? panel
                target.focus()
            },

            /** A toast for what was done; what failed stays here, at the top, with its groups. */
            report(done: number, failures: { error: string }[], action: Action, libraryTrash: string | null) {
                if (done > 0) {
                    let message = action === 'trash'
                        ? this.$t('library.trashDone', { n: done })
                        : this.$t('library.forgetDone', { n: done })
                    if (libraryTrash) { message += ' ' + this.$t('library.trashFallback', { path: libraryTrash }) }
                    this.notificationsStore.push('success', message, libraryTrash ? 15_000 : undefined)
                }
                this.failure = failures.length === 0 ? '' : action === 'trash'
                    ? this.$t('library.trashFailures', { n: failures.length, error: failures[0].error })
                    : this.$t('library.forgetFailures', { n: failures.length, error: failures[0].error })
            },
        },
    })
</script>

<style lang="scss" scoped>
.dup-intro {
    margin-bottom: 20px;
    font-size: var(--font-size-sm);
    line-height: 1.55;
    color: var(--text-secondary);
}

.dup-notice {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin-bottom: var(--space-4);
    padding: 10px var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--danger-muted);
    font-size: var(--font-size-sm);
    color: var(--danger);
    word-break: break-word;

    svg { flex-shrink: 0; margin-top: 2px; }
}

.dup-empty { min-height: 160px; }

.dup-group {
    margin-bottom: var(--space-5);

    &:last-of-type { margin-bottom: 0; }
}

.dup-group-header {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
}

.dup-label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-bold);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--accent-text);
}

.dup-reason {
    font-size: var(--font-size-xs);
    color: var(--text-muted);
}

.dup-decision {
    margin-left: auto;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--text-secondary);
}

.dup-books {
    display: flex;
    flex-direction: column;
    gap: 6px;
}

.dup-book {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 10px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: border-color var(--transition), background var(--transition);

    &:hover {
        border-color: var(--border-hover);
        background: var(--bg-hover);
    }

    &.-kept {
        border-color: color-mix(in srgb, var(--accent) 60%, transparent);
        background: var(--accent-muted);
    }

    &.-extra .dup-cover,
    &.-extra .dup-info { opacity: 0.75; }

    &.-skip {
        padding-top: var(--space-2);
        padding-bottom: var(--space-2);
        border-style: dashed;
    }
}

// The native radio stays in the page (focus, arrows, screen readers), drawn by the dot.
.dup-radio {
    position: absolute;
    width: 16px;
    height: 16px;
    margin: 0;
    opacity: 0;
    cursor: inherit;
}

.radio-dot {
    display: flex;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border: 2px solid var(--text-muted);
    border-radius: 50%;
    transition: border-color var(--transition);
}

.dup-radio:checked + .radio-dot {
    border-color: var(--accent-strong);

    &::after {
        content: '';
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--accent-strong);
    }
}

.dup-radio:focus-visible + .radio-dot {
    outline: var(--focus-ring-width) solid var(--focus-ring-color);
    outline-offset: var(--focus-ring-offset);
}

.dup-cover {
    display: flex;
    flex-shrink: 0;
    align-items: center;
    justify-content: center;
    width: 40px;
    aspect-ratio: var(--cover-ratio);
    overflow: hidden;
    border-radius: var(--radius-xs);
    background: var(--bg-cover-placeholder);
    color: var(--text-muted);
    transition: opacity var(--transition);

    img {
        display: block;
        width: 100%;
        height: 100%;
        object-fit: cover;
    }
}

.dup-info {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    transition: opacity var(--transition);
}

.dup-title-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
}

.dup-title {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
}

.dup-suggested { flex-shrink: 0; }

.dup-why {
    flex-shrink: 0;
    font-size: var(--font-size-xs);
    color: var(--accent-text);
}

.dup-author {
    font-size: var(--font-size-xs);
    color: var(--text-secondary);
}

.dup-facts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin-top: 2px;
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-secondary);
}

.dup-missing,
.dup-path {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--font-size-xs);
    color: var(--text-muted);
}

.dup-path { font-family: var(--font-mono); }

.dup-fate { flex-shrink: 0; }

.dup-skip-text {
    font-size: var(--font-size-sm);
    color: var(--text-secondary);
}

// ── Footer ────────────────────────────────────────────────────────────────────

.dup-footer {
    display: flex;
    flex: 1;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    min-width: 0;
}

.dup-summary {
    font-size: var(--font-size-xs);
    color: var(--text-secondary);
}

.dup-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-left: auto;
}
</style>
