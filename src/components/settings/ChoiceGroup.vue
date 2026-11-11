<template>
    <div class="choice-group" role="radiogroup" :aria-labelledby="labelledby || undefined" :aria-describedby="describedby || undefined">
        <label
            v-for="option in options"
            :key="option.value"
            class="choice-group__option"
            :class="{ '-checked': option.value === modelValue }"
        >
            <input
                type="radio"
                class="choice-group__input"
                :name="name"
                :value="option.value"
                :checked="option.value === modelValue"
                @change="$emit('update:modelValue', option.value)"
            />
            <vue-feather v-if="option.icon" :type="option.icon" size="14" aria-hidden="true" />
            <span>{{ option.label }}</span>
        </label>
    </div>
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import { uniqueId } from '@/utils/dom'
    import type { ChoiceOption } from './sections'

    /**
     * A few mutually exclusive choices side by side (Light / Dark / Match
     * system; Off / Subtle / On), drawn as one segmented control.
     *
     * They are real radio buttons: Tab reaches the group, the arrow keys
     * move between choices (and choose), screen readers announce "radio
     * button, 2 of 3". Label the group with `labelledby` (SettingRow's
     * `labelId`).
     */
    export default defineComponent({
        name: 'ChoiceGroup',

        props: {
            modelValue:  { type: String, required: true },
            options:     { type: Array as PropType<ChoiceOption[]>, required: true },
            labelledby:  { type: String, default: '' },
            describedby: { type: String, default: '' },
        },

        emits: {
            'update:modelValue': (_value: string) => true,
        },

        data() {
            return { name: uniqueId('choice') }
        },
    })
</script>

<style lang="scss" scoped>
.choice-group {
    display: inline-flex;
    padding: 2px;
    gap: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
}

.choice-group__option {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 4px 10px;
    border-radius: var(--radius-xs);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--text-secondary);
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--transition), color var(--transition);

    &:hover { color: var(--text-primary); }

    &.-checked {
        background: var(--accent-strong);
        color: var(--on-accent);
    }

    // The ring of the (visually hidden) radio, drawn on its segment.
    &:has(.choice-group__input:focus-visible) {
        outline: var(--focus-ring-width) solid var(--focus-ring-color);
        outline-offset: 1px;
    }
}

// Hidden but still focusable and clickable (through its label).
.choice-group__input {
    position: absolute;
    opacity: 0;
    width: 1px;
    height: 1px;
    margin: 0;
    pointer-events: none;
}
</style>
