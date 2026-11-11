<template>
    <BaseModal :title="$t('shortcuts.dialogTitle')" :width="560" close-on-backdrop @close="$emit('close')">
        <section v-for="group in groups" :key="group.id" class="shortcuts__group">
            <h3 class="shortcuts__heading">{{ $t(`shortcuts.groups.${group.id}`) }}</h3>
            <dl class="shortcuts__list">
                <div v-for="row in group.rows" :key="row.id" class="shortcuts__row">
                    <dt class="shortcuts__what">{{ $t(row.description) }}</dt>
                    <dd class="shortcuts__keys">
                        <template v-for="(combo, i) in row.combos" :key="combo">
                            <span v-if="i > 0" class="shortcuts__or">/</span>
                            <kbd>{{ combo }}</kbd>
                        </template>
                    </dd>
                </div>
            </dl>
        </section>

        <!-- Not global: what the keys do in the grid or list of books (BookCollection). -->
        <section class="shortcuts__group">
            <h3 class="shortcuts__heading">{{ $t('shortcuts.booksTitle') }}</h3>
            <dl class="shortcuts__list">
                <div v-for="row in bookRows" :key="row.description" class="shortcuts__row">
                    <dt class="shortcuts__what">{{ $t(row.description) }}</dt>
                    <dd class="shortcuts__keys">
                        <template v-for="(key, i) in row.keys" :key="key">
                            <span v-if="i > 0" class="shortcuts__or">/</span>
                            <kbd>{{ key }}</kbd>
                        </template>
                    </dd>
                </div>
            </dl>
        </section>

        <p v-if="isMac" class="shortcuts__note">{{ $t('shortcuts.menuNote') }}</p>

        <template #footer>
            <BaseButton variant="primary" data-autofocus @click="$emit('close')">{{ $t('common.close') }}</BaseButton>
        </template>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import { useShortcutsStore } from '@/stores/shortcuts'
    import {
        formatCombo, SHORTCUT_GROUPS, SHORTCUT_IDS, SHORTCUTS,
        type ShortcutDefinition, type ShortcutGroup, type ShortcutId,
    } from '@/constants/shortcuts'
    import type { TextKey } from '@/i18n'

    interface ShortcutRow {
        id: ShortcutId
        description: TextKey
        /** The key combos, as the platform writes them (⌘O, Ctrl+O). */
        combos: string[]
    }

    /** The keys of the books' grid and list (utils/collectionNav.ts), described rather than declared. */
    const BOOK_ROWS: { description: TextKey, keys: string[] }[] = [
        { description: 'shortcuts.books.move', keys: ['← → ↑ ↓', 'Home', 'End'] },
        { description: 'shortcuts.books.select', keys: ['Space'] },
        { description: 'shortcuts.books.extend', keys: ['⇧ ← → ↑ ↓'] },
        { description: 'shortcuts.books.open', keys: ['Enter'] },
        { description: 'shortcuts.books.menu', keys: ['⇧F10'] },
    ]

    /**
     * Help › Keyboard shortcuts (⌘/): every command of constants/shortcuts.ts
     * that has keys, grouped, with its keys as this platform writes them;
     * then the keys of the books' grid and list. Built from the same table
     * the shortcuts store and the menu bar use, so it can't drift.
     */
    export default defineComponent({
        name: 'ShortcutsDialog',

        components: { BaseModal, BaseButton },

        emits: ['close'],

        computed: {
            ...mapStores(useShortcutsStore),

            isMac(): boolean { return this.shortcutsStore.isMac },

            groups(): { id: ShortcutGroup, rows: ShortcutRow[] }[] {
                return SHORTCUT_GROUPS.map(id => ({
                    id,
                    rows: SHORTCUT_IDS
                        .filter(sid => {
                            const def: ShortcutDefinition = SHORTCUTS[sid]
                            return def.group === id && def.keys.length > 0
                        })
                        .map(sid => ({
                            id: sid,
                            description: SHORTCUTS[sid].description,
                            combos: (SHORTCUTS[sid].keys as string[]).map(k => formatCombo(k, this.isMac)),
                        })),
                })).filter(group => group.rows.length > 0)
            },

            bookRows(): typeof BOOK_ROWS { return BOOK_ROWS },
        },
    })
</script>

<style lang="scss" scoped>
.shortcuts__group + .shortcuts__group {
    margin-top: var(--space-5);
}

.shortcuts__heading {
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-bold);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
}

.shortcuts__list {
    margin: 0;
}

.shortcuts__row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--border);

    &:last-child { border-bottom: none; }
}

.shortcuts__what {
    font-size: var(--font-size-sm);
    color: var(--text-primary);
}

.shortcuts__keys {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin: 0;
    flex-shrink: 0;
}

.shortcuts__or {
    font-size: var(--font-size-xs);
    color: var(--text-muted);
}

kbd {
    display: inline-block;
    min-width: 24px;
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: var(--radius-xs);
    background: var(--bg-elevated);
    font-family: var(--font-sans);
    font-size: var(--font-size-xs);
    text-align: center;
    color: var(--text-secondary);
}

.shortcuts__note {
    margin: var(--space-4) 0 0;
    font-size: var(--font-size-xs);
    color: var(--text-secondary);
}
</style>
