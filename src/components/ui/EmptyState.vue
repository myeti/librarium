<template>
    <div class="ui-empty" :role="alert ? 'alert' : undefined">
        <Spinner v-if="loading" :size="32" />
        <!-- The icon in a gilt medallion, over the theme's emblem. -->
        <span v-else-if="icon" class="ui-empty__seal" aria-hidden="true">
            <span class="ornament -art ui-empty__emblem"></span>
            <vue-feather :type="icon" :size="String(iconSize)" class="ui-empty__icon" />
        </span>
        <component :is="headingTag" v-if="title" class="ui-empty__title">{{ title }}</component>
        <span v-if="title && !loading" class="ornament -rule ui-empty__rule" aria-hidden="true"></span>
        <p v-if="description || $slots.default" class="ui-empty__text">
            <slot>{{ description }}</slot>
        </p>
        <div v-if="$slots.actions" class="ui-empty__actions">
            <slot name="actions" />
        </div>
    </div>
</template>

<script lang="ts">
    /**
     * What a screen shows when it has nothing to list: an empty library, no
     * search results, a device that isn't connected, a loading state.
     *
     * - `title` (a short heading) and `description` (or the default slot);
     * - `actions` slot for the way out ("Import books", "Retry");
     * - `loading` replaces the icon with the spinner;
     * - `alert` for failures ("Library not reachable"): announced at once.
     */
    import { defineComponent, type PropType } from 'vue'
    import Spinner from './Spinner.vue'

    export default defineComponent({
        name: 'EmptyState',

        components: { Spinner },

        props: {
            /** Feather icon name. */
            icon:        { type: String, default: '' },
            iconSize:    { type: Number, default: 40 },
            title:       { type: String, default: '' },
            description: { type: String, default: '' },
            loading:     { type: Boolean, default: false },
            alert:       { type: Boolean, default: false },
            /** Heading level of the title, to fit the page outline. */
            headingTag:  { type: String as PropType<'h2' | 'h3' | 'p'>, default: 'h2' },
        },
    })
</script>

<style lang="scss" scoped>
.ui-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    height: 100%;
    min-height: 240px;
    padding: var(--space-5);
    color: var(--text-muted);
    text-align: center;
}

.ui-empty__seal {
    position: relative;
    display: grid;
    place-items: center;
    width: 84px;
    aspect-ratio: 1;
    margin-bottom: var(--space-2);
    border-radius: 50%;
    color: var(--accent-text);
    background: radial-gradient(circle, color-mix(in srgb, var(--accent) 14%, transparent), transparent 70%);
    box-shadow:
        inset 0 0 0 1px var(--hairline),
        inset 0 0 0 6px color-mix(in srgb, var(--bg) 40%, transparent),
        inset 0 0 0 7px color-mix(in srgb, var(--hairline) 50%, transparent);
}

.ui-empty__emblem {
    position: absolute;
    width: 120px;
    opacity: .12;
}

.ui-empty__icon { position: relative; }

.ui-empty__title {
    font-family: var(--font-heading);
    font-size: 22px;
    font-weight: var(--heading-weight);
    letter-spacing: var(--heading-tracking);
    color: var(--text-primary);
}

.ui-empty__rule {
    width: 140px;
    opacity: .8;
}

.ui-empty__text {
    max-width: 520px;
    font-size: var(--font-size-sm);
    color: var(--text-secondary);
    word-break: break-word;
}

.ui-empty__actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
}
</style>
