<template>
    <BaseModal
        :width="500"
        :title="$t('convert.title')"
        :dismissible="!running"
        @close="close"
    >
        <!-- Why EPUB -->
        <div class="why-box">
            <p class="why-title">
                <vue-feather type="info" size="14" aria-hidden="true" />
                {{ $t('convert.whyTitle') }}
            </p>
            <p class="why-body">{{ $t('convert.why') }}</p>
        </div>

        <!-- Book list, with each book's state -->
        <p class="book-count">{{ $t('convert.bookCount', { n: books.length }) }}</p>
        <ul class="book-list">
            <li v-for="book in books" :key="book.id" class="book-item" :class="`-${stateOf(book)}`">
                <Spinner v-if="stateOf(book) === 'converting'" :size="14" class="item-icon" />
                <vue-feather v-else :type="ICONS[stateOf(book)]" size="14" class="item-icon" aria-hidden="true" />
                <span class="item-name">{{ book.metadata.title || book.filename }}</span>
                <Badge :tone="formatTone(book.format)">{{ book.format.toUpperCase() }}</Badge>
                <span class="sr-only">{{ stateLabel(book) }}</span>
                <p v-if="errors[book.id]" class="item-error">
                    {{ $t('convert.errorPrefix') }} {{ errors[book.id] }}
                </p>
            </li>
        </ul>

        <p v-if="!finished" class="replace-warning">
            <vue-feather type="alert-triangle" size="13" aria-hidden="true" />
            {{ $t('convert.willReplace') }}
        </p>
        <!-- After a run with failures: the dialog stays open so they can be read (and retried). -->
        <p v-else-if="failedBooks.length > 0" class="result -bad" role="alert">
            {{ converted.length === 0
                ? $t('convert.allFailed', { n: failedBooks.length })
                : $t('convert.partial', { n: failedBooks.length, total: books.length }) }}
        </p>

        <template #footer>
            <template v-if="!finished">
                <BaseButton variant="ghost" :disabled="running" @click="close">{{ $t('convert.skip') }}</BaseButton>
                <BaseButton variant="primary" :loading="running" @click="convert(books)">
                    {{ running ? $t('convert.converting') : $t('convert.convert') }}
                </BaseButton>
            </template>
            <template v-else>
                <BaseButton v-if="failedBooks.length > 0" :loading="running" icon="refresh-cw" @click="convert(failedBooks)">
                    {{ $t('convert.retry') }}
                </BaseButton>
                <BaseButton variant="primary" :disabled="running" data-autofocus @click="close">
                    {{ converted.length > 0 ? $t('convert.done') : $t('common.close') }}
                </BaseButton>
            </template>
        </template>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import Badge from '@/components/ui/Badge.vue'
    import Spinner from '@/components/ui/Spinner.vue'
    import type { BadgeTone } from '@/components/ui/types'
    import { useLibraryStore } from '@/stores/library'
    import { useNotificationsStore } from '@/stores/notifications'
    import { analytics } from '@/services/analytics'
    import { bucketCount } from '@/services/privacy'
    import { errorMessage } from '@/utils/errors'
    import type { Book } from '@/types'

    type ConvertState = 'pending' | 'converting' | 'done' | 'error'

    /** The icon of each state (`converting` shows the Spinner instead). */
    const ICONS: Record<ConvertState, string> = {
        pending:    'file',
        converting: 'loader',
        done:       'check-circle',
        error:      'alert-circle',
    }

    const FORMAT_TONES: readonly string[] = ['epub', 'pdf', 'mobi', 'azw3', 'cbz']

    /**
     * Offers to convert MOBI/AZW/AZW3 books to EPUB (after an import, or from
     * the detail panel), one book at a time, each book's state on its line.
     *
     * Emits `converted` with the converted books once the user is done with
     * the dialog: right away when every book converted (with a toast saying
     * so: callers close the dialog on `converted`), or, when some failed,
     * only when the user closes it, so the failures stay on screen to be
     * read and retried until then. The library store already holds the
     * converted books in either case.
     */
    export default defineComponent({
        name: 'EpubConvertModal',

        components: { BaseModal, BaseButton, Badge, Spinner },

        props: {
            books: { type: Array as PropType<Book[]>, required: true },
        },

        emits: {
            close: () => true,
            converted: (_books: Book[]) => true,
        },

        data(): {
            states: Record<string, ConvertState>
            errors: Record<string, string>
            converted: Book[]
            running: boolean
            finished: boolean
            reported: boolean
        } {
            return {
                states: {},
                errors: {},
                converted: [],
                running: false,
                finished: false,
                /** `converted` was emitted (only once). */
                reported: false,
            }
        },

        computed: {
            ...mapStores(useLibraryStore, useNotificationsStore),

            failedBooks(): Book[] {
                return this.books.filter(b => this.states[b.id] === 'error')
            },

            ICONS: () => ICONS,
        },

        methods: {
            stateOf(book: Book): ConvertState {
                return this.states[book.id] ?? 'pending'
            },

            /** The state in words, for screen readers (on screen, the icon says it). */
            stateLabel(book: Book): string {
                switch (this.stateOf(book)) {
                    case 'converting': return this.$t('convert.converting')
                    case 'done':       return this.$t('convert.stateDone')
                    case 'error':      return this.$t('convert.stateFailed')
                    default:           return ''
                }
            },

            formatTone(format: string): BadgeTone {
                return (FORMAT_TONES.includes(format) ? format : 'neutral') as BadgeTone
            },

            async convert(books: Book[]) {
                if (this.running) { return }
                this.running = true
                const newlyConverted: Book[] = []
                for (const book of books) {
                    this.states[book.id] = 'converting'
                    delete this.errors[book.id]
                    try {
                        const result = await this.libraryStore.convertToEpub(book)
                        this.states[book.id] = 'done'
                        newlyConverted.push(result)
                    } catch (e) {
                        this.states[book.id] = 'error'
                        this.errors[book.id] = errorMessage(e)
                    }
                }
                this.running = false
                this.finished = true
                this.converted.push(...newlyConverted)

                if (newlyConverted.length > 0) {
                    const fromFormats = [...new Set(books.filter(b => this.states[b.id] === 'done').map(b => b.format))].join(',')
                    analytics.track('book_convert', { book_count_bucket: bucketCount(newlyConverted.length), from_formats: fromFormats })
                }
                if (this.failedBooks.length === 0) {
                    this.notificationsStore.push('success', this.$t('convert.doneToast', { n: this.converted.length }))
                    this.report()
                }
            },

            /** Tells the parent which books were converted (once). */
            report() {
                if (this.reported || this.converted.length === 0) { return }
                this.reported = true
                this.$emit('converted', this.converted)
            },

            close() {
                if (this.running) { return }
                this.report()
                this.$emit('close')
            },
        },
    })
</script>

<style lang="scss" scoped>
.why-box {
    margin-bottom: var(--space-4);
    padding: var(--space-3) 14px;
    border: 1px solid color-mix(in srgb, var(--accent) 22%, transparent);
    border-radius: var(--radius-sm);
    background: var(--accent-muted);
}

.why-title {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 6px;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-bold);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--accent-text);
}

.why-body {
    margin: 0;
    font-size: var(--font-size-sm);
    line-height: 1.55;
    color: var(--text-secondary);
}

.book-count {
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
}

.book-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0 0 14px;
    padding: 0;
    list-style: none;
}

.book-item {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    padding: 7px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg);
    font-size: var(--font-size-sm);

    &.-error { border-color: color-mix(in srgb, var(--danger) 35%, transparent); }
}

.item-icon {
    flex-shrink: 0;
    color: var(--text-muted);

    .-converting & { color: var(--accent-text); }
    .-done &       { color: var(--success); }
    .-error &      { color: var(--danger); }
}

.item-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: var(--text-primary);
}

.item-error {
    flex-basis: 100%;
    margin: 0;
    padding-left: 22px;
    font-size: var(--font-size-xs);
    color: var(--danger);
    word-break: break-word;
}

.replace-warning {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--text-muted);

    svg { flex-shrink: 0; color: var(--warning); }
}

.result {
    margin: 0;
    padding: var(--space-2) 10px;
    border-radius: var(--radius-sm);
    font-size: var(--font-size-xs);

    &.-bad { color: var(--danger); background: var(--danger-muted); }
}
</style>
