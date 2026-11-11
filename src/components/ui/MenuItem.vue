<template>
    <button
        type="button"
        class="ui-menu-item"
        :class="{ '-danger': danger }"
        :role="checked === null ? 'menuitem' : 'menuitemcheckbox'"
        :aria-checked="checked === null ? undefined : checked"
        :aria-disabled="disabled || undefined"
        tabindex="-1"
        @click="onClick"
    >
        <vue-feather v-if="checked === true" type="check" size="14" class="ui-menu-item__icon" aria-hidden="true" />
        <vue-feather v-else-if="icon" :type="icon" size="14" class="ui-menu-item__icon" aria-hidden="true" />
        <span v-else-if="checked === false" class="ui-menu-item__icon" aria-hidden="true"></span>
        <span class="ui-menu-item__label"><slot /></span>
        <span v-if="hint" class="ui-menu-item__hint">{{ hint }}</span>
    </button>
</template>

<script lang="ts">
    /**
     * An item of a DropdownMenu. The menu moves focus between items with the
     * arrow keys (they are not in the Tab order) and closes after a click.
     *
     * - `checked` (true/false) makes it a checkbox item (a sort order, a
     *   filter); leave it null for a plain action.
     * - `disabled` keeps it visible but skipped by the arrows and inert.
     * - `hint` is shown on the right (a shortcut, a count).
     * - Add `data-keep-open` to keep the menu open after a click.
     */
    import { defineComponent, type PropType } from 'vue'

    export default defineComponent({
        name: 'MenuItem',

        props: {
            icon:     { type: String, default: '' },
            danger:   { type: Boolean, default: false },
            disabled: { type: Boolean, default: false },
            checked:  { type: Boolean as PropType<boolean | null>, default: null },
            hint:     { type: String, default: '' },
        },

        emits: {
            click: (_e: MouseEvent) => true,
        },

        methods: {
            onClick(e: MouseEvent) {
                if (this.disabled) {
                    // The menu stays open: nothing happened.
                    e.stopPropagation()
                    return
                }
                this.$emit('click', e)
            },
        },
    })
</script>

<style lang="scss" scoped>
.ui-menu-item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: 7px 10px;
    border-radius: var(--radius-xs);
    font-size: var(--font-size-sm);
    line-height: var(--line-height-tight);
    color: var(--text-secondary);
    text-align: left;
    white-space: nowrap;

    &:hover,
    &:focus-visible {
        background: var(--bg-hover);
        color: var(--text-primary);
    }

    // The focus ring is drawn inside the item, so neighbours don't cover it.
    &:focus-visible { outline-offset: -2px; }

    &.-danger { color: var(--danger); }
    &.-danger:hover,
    &.-danger:focus-visible { background: var(--danger-muted); color: var(--danger); }

    &[aria-disabled='true'] {
        opacity: 0.45;
        cursor: not-allowed;
        &:hover { background: none; color: var(--text-secondary); }
    }
}

.ui-menu-item__icon {
    flex-shrink: 0;
    width: 14px;
    display: inline-flex;
}

.ui-menu-item__label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
}

.ui-menu-item__hint {
    margin-left: var(--space-4);
    font-size: var(--font-size-xs);
    color: var(--text-muted);
}
</style>
