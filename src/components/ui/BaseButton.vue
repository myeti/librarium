<template>
    <button
        :type="type"
        class="ui-button"
        :class="[`-${variant}`, `-${size}`, { '-block': block, '-loading': loading }]"
        :disabled="disabled || undefined"
        :aria-disabled="loading || undefined"
        :aria-busy="loading || undefined"
        @click="onClick"
    >
        <Spinner v-if="loading" :size="iconSize" class="ui-button__icon" />
        <vue-feather v-else-if="icon" :type="icon" :size="String(iconSize)" class="ui-button__icon" aria-hidden="true" />
        <span class="ui-button__label"><slot /></span>
    </button>
</template>

<script lang="ts">
    /**
     * A text button. Icon-only buttons use IconButton (it requires a label).
     *
     * - `variant`: `primary` (the main action of a screen or dialog, at most
     *   one), `secondary` (default), `ghost` (low emphasis: Cancel, toolbars),
     *   `danger` (destructive: Delete, Move to Trash).
     * - `loading`: shows a spinner and ignores clicks, but stays focusable
     *   (`aria-disabled`, not `disabled`), so focus isn't lost while the
     *   action runs and screen readers hear "busy".
     * - `disabled`: the native attribute (not focusable, not clickable).
     * - `type` defaults to `button`, so a button inside a <form> never
     *   submits it by accident; pass `type="submit"` for the one that should.
     *
     * Emits `click` only when the button is usable.
     */
    import { defineComponent, type PropType } from 'vue'
    import Spinner from './Spinner.vue'
    import type { ButtonSize, ButtonVariant } from './types'

    export default defineComponent({
        name: 'BaseButton',

        components: { Spinner },

        props: {
            variant: { type: String as PropType<ButtonVariant>, default: 'secondary' },
            size:    { type: String as PropType<ButtonSize>, default: 'md' },
            type:    { type: String as PropType<'button' | 'submit' | 'reset'>, default: 'button' },
            /** Feather icon name shown before the label. */
            icon:    { type: String, default: '' },
            disabled: { type: Boolean, default: false },
            loading:  { type: Boolean, default: false },
            /** Full width of its container. */
            block:    { type: Boolean, default: false },
        },

        emits: {
            click: (_e: MouseEvent) => true,
        },

        computed: {
            iconSize(): number {
                return this.size === 'sm' ? 12 : 14
            },
        },

        methods: {
            onClick(e: MouseEvent) {
                if (this.disabled || this.loading) {
                    e.preventDefault()
                    return
                }
                this.$emit('click', e)
            },
        },
    })
</script>

<style lang="scss" scoped>
.ui-button {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    overflow: hidden;
    border-radius: var(--radius-control);
    border: 1px solid var(--border);
    font-family: inherit;
    font-weight: var(--font-weight-medium);
    line-height: 1;
    white-space: nowrap;
    user-select: none;
    -webkit-user-select: none;
    cursor: pointer;
    transition: background var(--transition), color var(--transition), box-shadow var(--transition), border-color var(--transition);
    background: color-mix(in srgb, var(--bg-elevated) 80%, transparent);
    color: var(--text-secondary);
    box-shadow: var(--shadow-sm);

    &:hover {
        background: var(--bg-hover);
        color: var(--text-primary);
        border-color: var(--border-hover);
    }

    &:disabled {
        opacity: 0.45;
        cursor: not-allowed;
        pointer-events: none;
        box-shadow: none;
    }

    &.-loading { cursor: progress; }

    &.-block { display: flex; width: 100%; }
}

// Sizes
.-md { padding: 7px 14px; font-size: var(--font-size-sm); }
.-sm { padding: 4px 10px; font-size: var(--font-size-xs); gap: 5px; }

// Variants
// The primary action: a plate of the theme's metal, with a glint on hover.
.-primary {
    background: linear-gradient(135deg, var(--accent-strong) 0%, var(--accent-dark) 100%);
    color: var(--on-accent);
    border-color: transparent;
    font-weight: var(--font-weight-semibold);
    box-shadow: 0 4px 14px color-mix(in srgb, var(--accent-dark) 34%, transparent), inset 0 1px 0 rgba(255, 255, 255, .22);

    &::after {
        content: "";
        position: absolute;
        inset: 0;
        background: linear-gradient(105deg, transparent 30%, rgba(255,255,255,.32) 48%, transparent 66%);
        transform: translateX(-110%);
        pointer-events: none;
    }

    &:hover {
        background: linear-gradient(135deg, var(--accent-hover) 0%, var(--accent-dark) 100%);
        box-shadow: 0 6px 22px color-mix(in srgb, var(--accent-dark) 42%, transparent), inset 0 1px 0 rgba(255, 255, 255, .26);
        color: var(--on-accent);
        border-color: transparent;

        &::after {
            transform: translateX(110%);
            transition: transform var(--duration-sheen) var(--ease-out);
        }
    }
}

.-ghost {
    background: transparent;
    border-color: transparent;
    box-shadow: none;

    &:hover {
        background: var(--bg-hover);
        border-color: var(--border);
    }
}

.-danger {
    background: var(--danger-muted);
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 15%, transparent);

    &:hover {
        background: var(--danger-strong);
        color: var(--on-danger);
        border-color: transparent;
        box-shadow: 0 2px 8px color-mix(in srgb, var(--danger-strong) 30%, transparent);
    }
}

.ui-button__icon { flex-shrink: 0; }
.ui-button__label:empty { display: none; }
</style>
