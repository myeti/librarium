<template>
    <div class="filter-bar">
        <!-- The view's title, lettered like the theme's covers (hidden when narrow). -->
        <h1 class="filter-bar__title display-title">{{ $t('nav.books') }}</h1>
        <!-- With an e-reader connected: all the books, the library's, or the device's. -->
        <div v-if="deviceStore.isConnected" class="filter-bar__scope">
            <span :id="scopeLabelId" class="sr-only">{{ $t('shelf.scopeLabel') }}</span>
            <ChoiceGroup
                :model-value="libraryStore.scope"
                :options="scopeOptions"
                :labelledby="scopeLabelId"
                @update:model-value="setScope"
            />
        </div>
        <div class="filter-bar__search">
            <SearchInput
                ref="search"
                v-model="search"
                :label="$t('collection.searchLabel')"
                :placeholder="$t('filter.search')"
                :debounce="searchDebounce"
            />
        </div>

        <FilterPopover />

        <DropdownMenu :label="$t('sort.sort')" placement="bottom-end">
            <template #trigger="{ toggle, attrs }">
                <IconButton
                    :icon="sort.order === 'asc' ? 'arrow-up' : 'arrow-down'"
                    :label="sortLabel"
                    v-bind="attrs"
                    @click="toggle"
                />
            </template>
            <MenuItem
                v-for="opt in sortOptions"
                :key="opt.value"
                :checked="sort.field === opt.value"
                data-keep-open
                @click="setSortField(opt.value)"
            >
                {{ opt.label }}
            </MenuItem>
            <MenuSeparator />
            <MenuItem :checked="sort.order === 'asc'" @click="setOrder('asc')">{{ orderLabel('asc') }}</MenuItem>
            <MenuItem :checked="sort.order === 'desc'" @click="setOrder('desc')">{{ orderLabel('desc') }}</MenuItem>
        </DropdownMenu>

        <span class="filter-bar__sep" aria-hidden="true"></span>

        <IconButton icon="grid" :label="$t('collection.gridView')" :pressed="viewMode === 'grid'" @click="libraryStore.setViewMode('grid')" />
        <IconButton icon="list" :label="$t('collection.listView')" :pressed="viewMode === 'list'" @click="libraryStore.setViewMode('list')" />
        <IconButton
            v-if="duplicateCount > 0"
            icon="copy"
            :label="$t('filter.duplicates', { n: duplicateCount })"
            @click="bookActionsStore.openDuplicates()"
        >
            <span class="filter-bar__count" aria-hidden="true">{{ duplicateCount }}</span>
        </IconButton>
    </div>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { useLibraryStore } from '@/stores/library'
    import { useBookActionsStore } from '@/stores/bookActions'
    import SearchInput from '@/components/ui/SearchInput.vue'
    import IconButton from '@/components/ui/IconButton.vue'
    import DropdownMenu from '@/components/ui/DropdownMenu.vue'
    import MenuItem from '@/components/ui/MenuItem.vue'
    import MenuSeparator from '@/components/ui/MenuSeparator.vue'
    import FilterPopover from '@/components/library/FilterPopover.vue'
    import ChoiceGroup from '@/components/settings/ChoiceGroup.vue'
    import type { ChoiceOption } from '@/components/settings/sections'
    import { useDeviceStore } from '@/stores/device'
    import { isShelfScope } from '@/services/shelf'
    import { deviceLabel } from '@/utils/device'
    import { uniqueId } from '@/utils/dom'
    import type { SortConfig, SortField } from '@/types'

    /** The search runs this long after the last keystroke (filtering a big library on every key lags). */
    const SEARCH_DEBOUNCE_MS = 150

    /**
     * The library's toolbar while nothing is selected: the scope (with an
     * e-reader connected: all the books, the library's, or the device's),
     * search, filters, sort (by reading progress too, with an e-reader),
     * grid/list view and (when there are some) the duplicates review.
     * Everything it changes lives in the library store.
     *
     * The search field (SearchInput) shows what is typed at once and
     * applies it SEARCH_DEBOUNCE_MS after the last keystroke, or at once on
     * Enter; when the store's search is reset elsewhere (another library
     * was opened), the field follows. ⌘F focuses it (`focusSearch`, called
     * by the view's shortcut).
     */
    export default defineComponent({
        name: 'FilterSortBar',

        components: { SearchInput, IconButton, DropdownMenu, MenuItem, MenuSeparator, FilterPopover, ChoiceGroup },

        data() {
            return { scopeLabelId: uniqueId('scope') }
        },

        computed: {
            ...mapStores(useLibraryStore, useBookActionsStore, useDeviceStore),

            /** All · Library · Kindle (the device's own name). */
            scopeOptions(): ChoiceOption[] {
                return [
                    { value: 'all', label: this.$t('shelf.scopeAll') },
                    { value: 'library', label: this.$t('shelf.scopeLibrary'), icon: 'book' },
                    { value: 'device', label: deviceLabel(this.deviceStore.device), icon: 'tablet' },
                ]
            },

            searchDebounce(): number { return SEARCH_DEBOUNCE_MS },

            search: {
                get(): string { return this.libraryStore.filters.search },
                set(value: string) { this.libraryStore.setFilters({ search: value }) },
            },

            sort(): SortConfig { return this.libraryStore.sort },
            viewMode(): string { return this.libraryStore.viewMode },
            duplicateCount(): number { return this.libraryStore.duplicateGroups.length },

            sortOptions(): { value: SortField, label: string }[] {
                return [
                    { value: 'dateAdded', label: this.$t('sort.dateAdded') },
                    { value: 'title',     label: this.$t('sort.title') },
                    { value: 'author',    label: this.$t('sort.author') },
                    { value: 'series',    label: this.$t('sort.series') },
                    { value: 'format',    label: this.$t('sort.format') },
                    { value: 'sizeBytes', label: this.$t('sort.size') },
                    // Only an e-reader reports reading progress.
                    ...(this.deviceStore.isConnected || this.sort.field === 'progress'
                        ? [{ value: 'progress' as const, label: this.$t('sort.progress') }]
                        : []),
                ]
            },

            /** "Sort: Title, A to Z": the sort button's name says the current order. */
            sortLabel(): string {
                const field = this.sortOptions.find(o => o.value === this.sort.field)?.label ?? ''
                return this.$t('collection.sortLabel', { field, order: this.orderLabel(this.sort.order) })
            },
        },

        methods: {
            setScope(value: string) {
                if (isShelfScope(value)) { this.libraryStore.setScope(value) }
            },

            setSortField(field: SortField) {
                this.libraryStore.setSort({ field, order: this.sort.order })
            },

            /**
             * The order in words that fit the field: "A to Z" for text,
             * "Newest first" for the date added, "Largest first" for the size.
             */
            orderLabel(order: 'asc' | 'desc'): string {
                const asc = order === 'asc'
                switch (this.sort.field) {
                    case 'dateAdded': return this.$t(asc ? 'collection.orderOldest' : 'collection.orderNewest')
                    case 'sizeBytes': return this.$t(asc ? 'collection.orderSmallest' : 'collection.orderLargest')
                    case 'progress':  return this.$t(asc ? 'collection.orderLeastRead' : 'collection.orderMostRead')
                    default:          return this.$t(asc ? 'collection.orderAZ' : 'collection.orderZA')
                }
            },

            setOrder(order: 'asc' | 'desc') {
                this.libraryStore.setSort({ field: this.sort.field, order })
            },

            /** Focuses the search field (⌘F). */
            focusSearch() {
                (this.$refs.search as { focus(): void } | undefined)?.focus()
            },
        },
    })
</script>

<style lang="scss" scoped>
.filter-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 100%;
    padding: 0 28px;
    container: filter-bar / inline-size;
}

.filter-bar__title {
    flex-shrink: 0;
    margin-right: var(--space-4);
    font-size: 24px;
    white-space: nowrap;
}

@container filter-bar (max-width: 620px) {
    .filter-bar__title { display: none; }
}

.filter-bar__scope {
    flex-shrink: 0;
}

.filter-bar__search {
    flex: 1;
    min-width: 160px;
    max-width: 560px;
    margin-left: auto;
}

.filter-bar__sep {
    flex-shrink: 0;
    width: 1px;
    height: 20px;
    margin: 0 var(--space-1);
    background: var(--border);
}

.filter-bar__count {
    position: absolute;
    top: -4px;
    right: -4px;
    min-width: 16px;
    padding: 2px 4px;
    border-radius: var(--radius-full);
    background: var(--danger-strong);
    color: var(--on-danger);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-bold);
    line-height: 1;
    text-align: center;
}
</style>
