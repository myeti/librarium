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
        <textarea
            v-if="multiline"
            :id="ids.controlId"
            ref="control"
            class="ui-text-field"
            :class="{ '-invalid': !!error }"
            :value="modelValue"
            :rows="rows"
            :placeholder="placeholder || undefined"
            :required="required || undefined"
            :disabled="disabled || undefined"
            :readonly="readonly || undefined"
            :aria-invalid="error ? 'true' : undefined"
            :aria-describedby="ariaDescribedby"
            v-bind="controlAttrs"
            @input="onInput"
        ></textarea>
        <input
            v-else
            :id="ids.controlId"
            ref="control"
            class="ui-text-field"
            :class="{ '-invalid': !!error }"
            :type="type"
            :value="modelValue"
            :placeholder="placeholder || undefined"
            :required="required || undefined"
            :disabled="disabled || undefined"
            :readonly="readonly || undefined"
            :autocomplete="autocomplete || undefined"
            :aria-invalid="error ? 'true' : undefined"
            :aria-describedby="ariaDescribedby"
            v-bind="controlAttrs"
            @input="onInput"
        />
    </FormField>
</template>

<script lang="ts">
    /**
     * A labelled text field (`multiline` for a textarea).
     *
     * The label is required and linked to the input (`for`/`id`), so a click
     * on it focuses the field and screen readers announce it. `hint` and
     * `error` are read with the field (aria-describedby); `error` also sets
     * aria-invalid. `v-model` holds a string (use `Number(…)` yourself for
     * numbers: an empty field is '' rather than 0).
     *
     * `class` and `style` go on the field's frame; every other attribute
     * (maxlength, @keydown.enter, @blur…) goes on the input itself.
     */
    import { defineComponent, type PropType } from 'vue'
    import FormField from './FormField.vue'
    import { describedBy, fieldIds, type FieldIds } from './field'

    export default defineComponent({
        name: 'TextField',

        components: { FormField },

        inheritAttrs: false,

        props: {
            modelValue:   { type: [String, Number], default: '' },
            label:        { type: String, required: true },
            type:         { type: String as PropType<'text' | 'email' | 'password' | 'number' | 'url' | 'search' | 'tel'>, default: 'text' },
            placeholder:  { type: String, default: '' },
            hint:         { type: String, default: '' },
            error:        { type: String, default: '' },
            required:     { type: Boolean, default: false },
            disabled:     { type: Boolean, default: false },
            readonly:     { type: Boolean, default: false },
            multiline:    { type: Boolean, default: false },
            rows:         { type: Number, default: 4 },
            autocomplete: { type: String, default: '' },
            hideLabel:    { type: Boolean, default: false },
            /** Id of the input (generated when omitted). */
            id:           { type: String, default: '' },
        },

        emits: {
            'update:modelValue': (_value: string) => true,
        },

        data(): { ids: FieldIds } {
            return { ids: fieldIds('field', this.id) }
        },

        computed: {
            ariaDescribedby(): string | undefined {
                return describedBy(this.ids, this.hint, this.error)
            },

            /** Every attribute except class/style (those style the frame). */
            controlAttrs(): Record<string, unknown> {
                const { class: _class, style: _style, ...rest } = this.$attrs
                return rest
            },
        },

        methods: {
            onInput(e: Event) {
                this.$emit('update:modelValue', (e.target as HTMLInputElement | HTMLTextAreaElement).value)
            },

            focus() {
                (this.$refs.control as HTMLElement | undefined)?.focus()
            },
        },
    })
</script>

<style lang="scss" scoped>
.ui-text-field {
    width: 100%;
    padding: 7px 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    font-family: inherit;
    font-size: var(--font-size-sm);
    line-height: var(--line-height);
    outline: none;
    transition: border-color var(--transition), box-shadow var(--transition);

    &::placeholder { color: var(--text-muted); }

    &:focus,
    &:focus-visible {
        border-color: var(--focus-ring-color);
        box-shadow: 0 0 0 1px var(--focus-ring-color), 0 0 0 4px color-mix(in srgb, var(--accent) 12%, transparent);
    }

    &.-invalid { border-color: var(--danger); }

    &:disabled { opacity: 0.6; cursor: not-allowed; }
}

textarea.ui-text-field {
    display: block;
    resize: vertical;
}
</style>
