<template>
    <span class="ui-spinner" :role="label ? 'status' : undefined">
        <vue-feather type="loader" :size="String(size)" class="spinning" aria-hidden="true" />
        <span v-if="label" class="sr-only">{{ label }}</span>
    </span>
</template>

<script lang="ts">
    /**
     * The loading indicator: the app's spinning loader icon.
     *
     * Decorative by default (hidden from screen readers: the surrounding
     * text or a busy button says what is happening). Give it a `label` when
     * it is the only sign that something is loading; it then becomes a
     * polite live region (`role="status"`) that announces the label.
     */
    import { defineComponent } from 'vue'

    export default defineComponent({
        name: 'Spinner',

        props: {
            /** Icon size in px. */
            size: { type: Number, default: 16 },
            /** Announced to screen readers; omit when the spinner is decorative. */
            label: { type: String, default: '' },
        },
    })
</script>

<style lang="scss" scoped>
.ui-spinner {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    line-height: 0;
}
</style>
