<template>
    <div ref="root" class="filter-popover-wrap" @focusout="onFocusOut">
        <IconButton
            ref="trigger"
            icon="sliders"
            :label="triggerLabel"
            :class="{ '-active': activeCount > 0 || isOpen }"
            :aria-expanded="String(isOpen)"
            :aria-controls="isOpen ? panelId : undefined"
            @click="toggle"
        >
            <span v-if="activeCount > 0" class="filter-count" aria-hidden="true">{{ activeCount }}</span>
        </IconButton>

        <div
            v-if="isOpen"
            :id="panelId"
            ref="panel"
            class="filter-panel"
            role="dialog"
            :aria-label="$t('filter.filters')"
            @keydown.esc.stop.prevent="close(true)"
        >
            <div class="filter-panel__grid">
                <SelectField
                    :model-value="filters.authors[0] ?? ''"
                    :label="$t('filter.labelAuthor')"
                    :options="withAll(libraryStore.allAuthors)"
                    @update:model-value="set('authors', $event)"
                />
                <SelectField
                    v-if="libraryStore.allSeries.length"
                    :model-value="filters.series[0] ?? ''"
                    :label="$t('filter.labelSeries')"
                    :options="withAll(libraryStore.allSeries)"
                    @update:model-value="set('series', $event)"
                />
                <SelectField
                    :model-value="filters.categories[0] ?? ''"
                    :label="$t('filter.labelCategory')"
                    :options="withAll(libraryStore.allCategories, categoryLabel)"
                    @update:model-value="set('categories', $event)"
                />
                <SelectField
                    :model-value="filters.formats[0] ?? ''"
                    :label="$t('filter.labelFormat')"
                    :options="formatOptions"
                    @update:model-value="set('formats', $event)"
                />
                <SelectField
                    v-if="libraryStore.allLanguages.length"
                    :model-value="filters.languages[0] ?? ''"
                    :label="$t('filter.labelLanguage')"
                    :options="withAll(libraryStore.allLanguages)"
                    @update:model-value="set('languages', $event)"
                />
                <!-- With an e-reader: where the book is ("not sent yet" is "Only in the library"). -->
                <SelectField
                    v-if="deviceStore.isConnected || filters.presence.length > 0"
                    :model-value="filters.presence[0] ?? ''"
                    :label="$t('filter.labelWhere')"
                    :options="presenceOptions"
                    @update:model-value="set('presence', $event)"
                />
            </div>
            <div v-if="activeCount > 0" class="filter-panel__footer">
                <BaseButton variant="ghost" size="sm" icon="x" @click="clear">{{ $t('filter.clearFilters') }}</BaseButton>
            </div>
        </div>
    </div>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { useLibraryStore } from '@/stores/library'
    import { useDeviceStore } from '@/stores/device'
    import { deviceLabel } from '@/utils/device'
    import { categoryLabel } from '@/i18n'
    import { uniqueId } from '@/utils/dom'
    import IconButton from '@/components/ui/IconButton.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import SelectField from '@/components/ui/SelectField.vue'
    import type { SelectOption } from '@/components/ui/types'
    import type { BookFormat, FilterConfig, Presence } from '@/types'

    type FilterKey = 'authors' | 'series' | 'categories' | 'formats' | 'languages' | 'presence'
    const FILTER_KEYS: readonly FilterKey[] = ['authors', 'series', 'categories', 'formats', 'languages', 'presence']
    const FORMATS: readonly BookFormat[] = ['epub', 'pdf', 'mobi', 'azw3', 'cbz']

    /**
     * The library's filters (author, series, category, format, language,
     * and with an e-reader connected, where the book is), in a popover under
     * a button that shows how many are active.
     *
     * Not a DropdownMenu: it holds form fields, which need the arrow keys
     * and Tab for themselves. It is a non-modal disclosure instead: the
     * button has `aria-expanded`, opening focuses the first field, Escape
     * closes it and puts focus back on the button, and it closes when focus
     * or a click goes elsewhere.
     */
    export default defineComponent({
        name: 'FilterPopover',

        components: { IconButton, BaseButton, SelectField },

        data() {
            return {
                isOpen: false,
                panelId: uniqueId('filters'),
            }
        },

        computed: {
            ...mapStores(useLibraryStore, useDeviceStore),

            filters(): FilterConfig {
                return this.libraryStore.filters
            },

            activeCount(): number {
                return FILTER_KEYS.filter(k => this.filters[k].length > 0).length
            },

            triggerLabel(): string {
                return this.activeCount > 0
                    ? this.$t('collection.filtersActive', { n: this.activeCount })
                    : this.$t('filter.filters')
            },

            presenceOptions(): SelectOption[] {
                const device = deviceLabel(this.deviceStore.device)
                return [
                    { value: '', label: this.$t('filter.all') },
                    { value: 'libraryOnly', label: this.$t('shelf.libraryOnly') },
                    { value: 'both', label: this.$t('shelf.both', { device }) },
                    { value: 'deviceOnly', label: this.$t('shelf.deviceOnlyFilter', { device }) },
                ]
            },

            formatOptions(): SelectOption[] {
                return [{ value: '', label: this.$t('filter.all') }, ...FORMATS.map(f => ({ value: f, label: f.toUpperCase() }))]
            },
        },

        beforeUnmount() {
            document.removeEventListener('pointerdown', this.onDocumentPointerdown, true)
        },

        methods: {
            categoryLabel,

            /** "All", then the values (labelled by `label`). */
            withAll(values: readonly string[], label: (v: string) => string = v => v): SelectOption[] {
                return [{ value: '', label: this.$t('filter.all') }, ...values.map(v => ({ value: v, label: label(v) }))]
            },

            /** One value per filter ("All" clears it). */
            set(key: FilterKey, value: string | number) {
                const v = String(value)
                const values = v ? [v] : []
                const patch: Partial<FilterConfig> = key === 'formats'
                    ? { formats: values as BookFormat[] }
                    : key === 'presence'
                        ? { presence: values as Presence[] }
                        : { [key]: values }
                this.libraryStore.setFilters(patch)
            },

            clear() {
                this.libraryStore.setFilters({ formats: [], authors: [], series: [], languages: [], categories: [], presence: [] })
                this.focusFirstField()
            },

            toggle() {
                if (this.isOpen) { this.close(false) } else { this.open() }
            },

            open() {
                this.isOpen = true
                document.addEventListener('pointerdown', this.onDocumentPointerdown, true)
                void this.$nextTick(() => this.focusFirstField())
            },

            /** Closes; `restoreFocus` puts focus back on the button (Escape). */
            close(restoreFocus: boolean) {
                if (!this.isOpen) { return }
                this.isOpen = false
                document.removeEventListener('pointerdown', this.onDocumentPointerdown, true)
                if (restoreFocus) {
                    ((this.$refs.trigger as { $el?: HTMLElement } | undefined)?.$el)?.focus()
                }
            },

            focusFirstField() {
                (this.$refs.panel as HTMLElement | undefined)?.querySelector<HTMLElement>('select')?.focus()
            },

            onDocumentPointerdown(e: Event) {
                const root = this.$refs.root as HTMLElement | undefined
                if (root && e.target instanceof Node && root.contains(e.target)) { return }
                this.close(false)
            },

            onFocusOut(e: FocusEvent) {
                const root = this.$refs.root as HTMLElement | undefined
                const next = e.relatedTarget
                if (root && next instanceof Node && !root.contains(next)) { this.close(false) }
            },
        },
    })
</script>

<style lang="scss" scoped>
.filter-popover-wrap {
    position: relative;
    flex-shrink: 0;
}

.filter-count {
    position: absolute;
    top: -4px;
    right: -4px;
    min-width: 16px;
    padding: 2px 4px;
    border-radius: var(--radius-full);
    background: var(--accent-strong);
    color: var(--on-accent);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-bold);
    line-height: 1;
    text-align: center;
}

.filter-panel {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: var(--z-popover);
    width: min(380px, calc(100vw - 32px));
    padding: var(--space-4);
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow-popover);
}

.filter-panel__grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
}

.filter-panel__footer {
    display: flex;
    justify-content: flex-end;
    margin-top: var(--space-3);
    padding-top: var(--space-3);
    border-top: 1px solid var(--border);
}
</style>
