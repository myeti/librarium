<template>
    <section :id="id" class="settings-section" :aria-labelledby="`${id}-title`">
        <header class="settings-section__header">
            <h2 :id="`${id}-title`" class="settings-section__title" tabindex="-1">{{ $t(`settings.sections.${id}`) }}</h2>
            <p class="settings-section__intro">{{ $t(`settings.sectionIntros.${id}`) }}</p>
        </header>
        <div class="settings-section__rows">
            <slot />
        </div>
    </section>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import type { SettingsSectionId } from './sections'

    /**
     * One section of the Settings screen: its title (the target of the side
     * navigation, focusable so the view can move focus to it) and one line
     * about it, then its settings (SettingRow) in the default slot.
     */
    export default defineComponent({
        name: 'SettingsSection',

        props: {
            id: { type: String as PropType<SettingsSectionId>, required: true },
        },
    })
</script>

<style lang="scss" scoped>
.settings-section__header {
    margin-bottom: var(--space-5);
}

.settings-section__title {
    margin: 0 0 var(--space-1);
    font-family: var(--font-heading);
    font-size: 26px;
    font-weight: var(--heading-weight);
    letter-spacing: var(--heading-tracking);
    line-height: var(--line-height-tight);
    color: var(--text-primary);
    outline: none;
}

.settings-section__intro {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--text-secondary);
}

.settings-section__rows {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
}
</style>
