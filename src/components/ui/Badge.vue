<template>
    <span class="ui-badge" :class="`-${tone}`">
        <vue-feather v-if="icon" :type="icon" size="11" aria-hidden="true" />
        <slot />
    </span>
</template>

<script lang="ts">
    /**
     * A small label: a book's format, a status ("On device", "Up to date"),
     * a count.
     *
     * Tones: `neutral`, `accent`, `success`, `warning`, `danger`, and one per
     * book format (`epub`, `pdf`, `mobi`, `azw3`, `cbz`). Every tone is
     * readable in every theme (themes.test.ts). The colour is never the only
     * signal: the badge always has text, and `icon` can reinforce it.
     */
    import { defineComponent, type PropType } from 'vue'
    import type { BadgeTone } from './types'

    export default defineComponent({
        name: 'Badge',

        props: {
            tone: { type: String as PropType<BadgeTone>, default: 'neutral' },
            /** Feather icon shown before the text. */
            icon: { type: String, default: '' },
        },
    })
</script>

<style lang="scss" scoped>
.ui-badge {
    --badge-color: var(--text-secondary);
    --badge-bg: color-mix(in srgb, var(--text-secondary) 10%, transparent);

    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 1px 7px;
    border-radius: var(--radius-full);
    border: 1px solid color-mix(in srgb, var(--badge-color) 18%, transparent);
    background: var(--badge-bg);
    color: var(--badge-color);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    line-height: 1.5;
    white-space: nowrap;
}

.-accent  { --badge-color: var(--accent-text); --badge-bg: var(--accent-muted); }
.-success { --badge-color: var(--success); --badge-bg: var(--success-muted); }
.-warning { --badge-color: var(--warning); --badge-bg: var(--warning-muted); }
.-danger  { --badge-color: var(--danger); --badge-bg: var(--danger-muted); }

// Formats: the same look as the .tag-<format> badges (uppercase, square-ish).
.-epub, .-pdf, .-mobi, .-azw3, .-cbz {
    --badge-bg: color-mix(in srgb, var(--badge-color) 10%, transparent);
    border-radius: 4px;
    font-weight: var(--font-weight-bold);
    letter-spacing: .04em;
    text-transform: uppercase;
}
.-epub { --badge-color: var(--fmt-epub); }
.-pdf  { --badge-color: var(--fmt-pdf); }
.-mobi { --badge-color: var(--fmt-mobi); }
.-azw3 { --badge-color: var(--fmt-azw3); }
.-cbz  { --badge-color: var(--fmt-cbz); }
</style>
