<template>
    <div class="ui-field" :class="{ '-invalid': !!error, '-disabled': disabled }">
        <label :for="controlId" class="ui-field__label" :class="{ 'sr-only': hideLabel }">
            {{ label }}<span v-if="required" class="ui-field__required" aria-hidden="true"> *</span>
        </label>
        <slot />
        <p v-if="hint" :id="hintId" class="ui-field__hint">{{ hint }}</p>
        <p v-if="error" :id="errorId" class="ui-field__error" role="alert">
            <vue-feather type="alert-circle" size="12" aria-hidden="true" />
            {{ error }}
        </p>
    </div>
</template>

<script lang="ts">
    /**
     * The frame shared by TextField and SelectField: a label linked to the
     * control (`for` = `controlId`), then the control (default slot), a hint
     * and an error. The control references the hint and the error with
     * `aria-describedby` (`describedBy()` in field.ts builds the value), so screen
     * readers read them with the field. Not meant to be used on its own.
     *
     * The error has an icon and text, never colour alone.
     */
    import { defineComponent } from 'vue'

    export default defineComponent({
        name: 'FormField',

        props: {
            label:     { type: String, required: true },
            controlId: { type: String, required: true },
            hintId:    { type: String, required: true },
            errorId:   { type: String, required: true },
            hint:      { type: String, default: '' },
            error:     { type: String, default: '' },
            required:  { type: Boolean, default: false },
            disabled:  { type: Boolean, default: false },
            /** Keep the label for screen readers only (a field whose purpose is obvious from the layout). */
            hideLabel: { type: Boolean, default: false },
        },
    })
</script>

<style lang="scss" scoped>
.ui-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
}

.ui-field__label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: .06em;
    text-transform: uppercase;
    color: var(--text-secondary);
}

.ui-field__required { color: var(--danger); }

.ui-field__hint {
    font-size: var(--font-size-xs);
    color: var(--text-muted);
}

.ui-field__error {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--font-size-xs);
    color: var(--danger);
}

.-disabled .ui-field__label { opacity: 0.6; }
</style>
