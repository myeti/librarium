<template>
    <div class="ui-checkbox" :class="{ '-disabled': disabled }">
        <label class="ui-checkbox__row">
            <input
                :id="ids.controlId"
                ref="input"
                type="checkbox"
                class="ui-checkbox__input"
                :checked="modelValue"
                :disabled="disabled || undefined"
                :aria-describedby="ariaDescribedby"
                v-bind="$attrs"
                @change="onChange"
            />
            <span class="ui-checkbox__box" aria-hidden="true">
                <vue-feather v-if="indeterminate" type="minus" size="12" />
                <vue-feather v-else-if="modelValue" type="check" size="12" />
            </span>
            <span class="ui-checkbox__label"><slot>{{ label }}</slot></span>
        </label>
        <p v-if="hint" :id="ids.hintId" class="ui-checkbox__hint">{{ hint }}</p>
    </div>
</template>

<script lang="ts">
    /**
     * A checkbox with its label (text in `label`, or markup in the slot).
     *
     * A real <input type="checkbox"> sits under the drawn box: Space toggles
     * it, the whole label is clickable, screen readers announce the state.
     * `indeterminate` shows a dash ("some selected") until the user clicks.
     * `hint` is a line under the label, read with the checkbox.
     *
     * Attributes (name, value, data-*) go on the <input>.
     */
    import { defineComponent } from 'vue'
    import { describedBy, fieldIds, type FieldIds } from './field'

    export default defineComponent({
        name: 'Checkbox',

        inheritAttrs: false,

        props: {
            modelValue:    { type: Boolean, default: false },
            label:         { type: String, default: '' },
            hint:          { type: String, default: '' },
            disabled:      { type: Boolean, default: false },
            indeterminate: { type: Boolean, default: false },
            id:            { type: String, default: '' },
        },

        emits: {
            'update:modelValue': (_value: boolean) => true,
        },

        data(): { ids: FieldIds } {
            return { ids: fieldIds('checkbox', this.id) }
        },

        computed: {
            ariaDescribedby(): string | undefined {
                return describedBy(this.ids, this.hint, '')
            },
        },

        watch: {
            indeterminate() { this.syncIndeterminate() },
        },

        mounted() {
            this.syncIndeterminate()
        },

        methods: {
            // `indeterminate` is a DOM property only (no attribute for it).
            syncIndeterminate() {
                const input = this.$refs.input as HTMLInputElement | undefined
                if (input) { input.indeterminate = this.indeterminate }
            },

            onChange(e: Event) {
                this.$emit('update:modelValue', (e.target as HTMLInputElement).checked)
            },
        },
    })
</script>

<style lang="scss" scoped>
.ui-checkbox {
    display: flex;
    flex-direction: column;
    gap: 2px;
}

.ui-checkbox__row {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--font-size-sm);
    color: var(--text-primary);
    cursor: pointer;
}

// The native input stays in the page (focus, Space, screen readers) but is
// drawn by the box next to it.
.ui-checkbox__input {
    position: absolute;
    width: 16px;
    height: 16px;
    margin: 0;
    opacity: 0;
    cursor: inherit;
}

.ui-checkbox__box {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    border: 1.5px solid var(--text-muted);
    border-radius: var(--radius-xs);
    background: var(--bg-input);
    color: var(--on-accent);
    transition: background var(--transition), border-color var(--transition);
}

.ui-checkbox__input:checked + .ui-checkbox__box,
.ui-checkbox__input:indeterminate + .ui-checkbox__box {
    background: var(--accent-strong);
    border-color: var(--accent-strong);
}

.ui-checkbox__input:focus-visible + .ui-checkbox__box {
    outline: var(--focus-ring-width) solid var(--focus-ring-color);
    outline-offset: var(--focus-ring-offset);
}

.ui-checkbox__hint {
    margin-left: calc(16px + var(--space-2));
    font-size: var(--font-size-xs);
    color: var(--text-muted);
}

.-disabled .ui-checkbox__row {
    opacity: 0.55;
    cursor: not-allowed;
}
</style>
