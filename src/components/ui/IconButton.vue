<template>
    <button
        :type="type"
        class="ui-icon-button"
        :class="[`-${size}`, { '-active': pressed === true, '-danger': danger }]"
        :aria-label="label"
        :title="title || label"
        :aria-pressed="pressed === null ? undefined : pressed"
        :disabled="disabled || undefined"
        @click="onClick"
    >
        <vue-feather :type="icon" :size="String(size === 'sm' ? 14 : 16)" aria-hidden="true" />
        <!-- A count or dot over the icon (decorative: put the meaning in `label`). -->
        <slot />
    </button>
</template>

<script lang="ts">
    /**
     * A button that shows only an icon.
     *
     * `label` is required: it is the button's accessible name (aria-label)
     * and its tooltip (title), since there is no visible text. Write it as
     * the action ("Close", "Send to Kindle"), not the icon ("X").
     *
     * `pressed` makes it a toggle (aria-pressed): grid/list view switches,
     * filters. Leave it null for plain actions.
     */
    import { defineComponent, type PropType } from 'vue'

    export default defineComponent({
        name: 'IconButton',

        props: {
            /** Accessible name and tooltip. */
            label:    { type: String, required: true },
            /** Feather icon name. */
            icon:     { type: String, required: true },
            /** Tooltip, when it should say more than the label (a shortcut…). */
            title:    { type: String, default: '' },
            size:     { type: String as PropType<'sm' | 'md'>, default: 'md' },
            /** Toggle state; null (default) for a plain action button. */
            pressed:  { type: Boolean as PropType<boolean | null>, default: null },
            danger:   { type: Boolean, default: false },
            disabled: { type: Boolean, default: false },
            type:     { type: String as PropType<'button' | 'submit' | 'reset'>, default: 'button' },
        },

        emits: {
            click: (_e: MouseEvent) => true,
        },

        methods: {
            onClick(e: MouseEvent) {
                if (!this.disabled) { this.$emit('click', e) }
            },
        },
    })
</script>

<style lang="scss" scoped>
.ui-icon-button {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    background: transparent;
    transition: background var(--transition), color var(--transition);

    &.-md { width: 30px; height: 30px; }
    &.-sm { width: 24px; height: 24px; }

    &:hover {
        background: var(--bg-hover);
        color: var(--text-primary);
    }

    &.-active {
        background: var(--accent-muted);
        color: var(--accent-text);
    }

    &.-danger:hover {
        background: var(--danger-muted);
        color: var(--danger);
    }

    &:disabled {
        opacity: 0.45;
        cursor: not-allowed;
        pointer-events: none;
    }
}
</style>
