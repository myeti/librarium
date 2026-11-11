<template>
    <FormField
        :label="label"
        :control-id="ids.controlId"
        :hint-id="ids.hintId"
        :error-id="ids.errorId"
        :hint="hint"
        :error="error"
        :required="required"
        :disabled="disabled"
        :hide-label="hideLabel"
        :class="$attrs.class"
        :style="$attrs.style"
    >
        <select
            :id="ids.controlId"
            class="ui-select"
            :class="{ '-invalid': !!error }"
            :value="selectedIndex === -1 ? '' : String(selectedIndex)"
            :required="required || undefined"
            :disabled="disabled || undefined"
            :aria-invalid="error ? 'true' : undefined"
            :aria-describedby="ariaDescribedby"
            v-bind="controlAttrs"
            @change="onChange"
        >
            <option v-if="placeholder" value="" disabled>{{ placeholder }}</option>
            <option
                v-for="(option, i) in options"
                :key="String(option.value)"
                :value="String(i)"
                :disabled="option.disabled || undefined"
            >
                {{ option.label }}
            </option>
        </select>
    </FormField>
</template>

<script lang="ts">
    /**
     * A labelled native <select> (keyboard, screen readers and type-to-find
     * work as everywhere else on the platform).
     *
     * `options` are `{ value, label, disabled? }`; `v-model` holds the
     * selected option's `value` with its own type (a number stays a number).
     * `placeholder` adds a first, unselectable "Choose…" entry shown while
     * the value matches no option.
     */
    import { defineComponent, type PropType } from 'vue'
    import FormField from './FormField.vue'
    import { describedBy, fieldIds, type FieldIds } from './field'
    import type { SelectOption } from './types'

    export default defineComponent({
        name: 'SelectField',

        components: { FormField },

        inheritAttrs: false,

        props: {
            modelValue:  { type: [String, Number] as PropType<string | number | null>, default: null },
            label:       { type: String, required: true },
            options:     { type: Array as PropType<SelectOption[]>, required: true },
            placeholder: { type: String, default: '' },
            hint:        { type: String, default: '' },
            error:       { type: String, default: '' },
            required:    { type: Boolean, default: false },
            disabled:    { type: Boolean, default: false },
            hideLabel:   { type: Boolean, default: false },
            id:          { type: String, default: '' },
        },

        emits: {
            'update:modelValue': (_value: string | number) => true,
        },

        data(): { ids: FieldIds } {
            return { ids: fieldIds('select', this.id) }
        },

        computed: {
            selectedIndex(): number {
                return this.options.findIndex(o => o.value === this.modelValue)
            },

            ariaDescribedby(): string | undefined {
                return describedBy(this.ids, this.hint, this.error)
            },

            controlAttrs(): Record<string, unknown> {
                const { class: _class, style: _style, ...rest } = this.$attrs
                return rest
            },
        },

        methods: {
            onChange(e: Event) {
                // Option values are indexes: map back to the real value (and type).
                const option = this.options[Number((e.target as HTMLSelectElement).value)]
                if (option) { this.$emit('update:modelValue', option.value) }
            },
        },
    })
</script>

<style lang="scss" scoped>
.ui-select {
    width: 100%;
    height: var(--control-height);
    padding: 0 30px 0 12px;
    background-color: var(--bg-input);
    background-image: var(--select-arrow);
    background-repeat: no-repeat;
    background-position: right 10px center;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-family: inherit;
    font-size: var(--font-size-sm);
    outline: none;
    appearance: none;
    cursor: pointer;
    transition: border-color var(--transition), box-shadow var(--transition);

    &:focus,
    &:focus-visible {
        border-color: var(--focus-ring-color);
        box-shadow: 0 0 0 1px var(--focus-ring-color), 0 0 0 4px color-mix(in srgb, var(--accent) 12%, transparent);
    }

    &.-invalid { border-color: var(--danger); }

    &:disabled { opacity: 0.6; cursor: not-allowed; }
}
</style>
