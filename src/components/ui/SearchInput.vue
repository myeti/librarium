<template>
    <div class="ui-search" :class="{ '-filled': text !== '' }">
        <label :for="inputId" class="sr-only">{{ label }}</label>
        <vue-feather type="search" size="14" class="ui-search__icon" aria-hidden="true" />
        <input
            :id="inputId"
            ref="input"
            type="search"
            class="ui-search__input"
            :value="text"
            :placeholder="placeholder || label"
            autocomplete="off"
            spellcheck="false"
            v-bind="$attrs"
            @input="onInput"
            @keydown="onKeydown"
        />
        <IconButton
            v-if="text !== ''"
            icon="x"
            size="sm"
            :label="$t('common.clear')"
            class="ui-search__clear"
            @click="clear"
        />
    </div>
</template>

<script lang="ts">
    /**
     * A search field: `v-model` is updated `debounce` ms after the user stops
     * typing (immediately on Enter, and when cleared), so filtering a large
     * library doesn't run on every key.
     *
     * - `label` is required: it labels the field for screen readers (and is
     *   the placeholder unless `placeholder` is given).
     * - The × button and Escape clear it. Escape on an empty field is left
     *   alone, so it can still close a panel or dialog.
     * - Emits `search` with the text on Enter.
     *
     * Other attributes (maxlength, data-*…) go on the <input>.
     */
    import { defineComponent } from 'vue'
    import { debounce as makeDebounced, type Debounced } from '@/utils/debounce'
    import { uniqueId } from '@/utils/dom'
    import IconButton from './IconButton.vue'

    export default defineComponent({
        name: 'SearchInput',

        components: { IconButton },

        inheritAttrs: false,

        props: {
            modelValue:  { type: String, default: '' },
            label:       { type: String, required: true },
            placeholder: { type: String, default: '' },
            /** Delay before `update:modelValue`, in ms (0: on every key). */
            debounce:    { type: Number, default: 200 },
            id:          { type: String, default: '' },
        },

        emits: {
            'update:modelValue': (_value: string) => true,
            search: (_value: string) => true,
        },

        data(): { text: string, fallbackId: string, commit: Debounced<[string]> | null } {
            return {
                text: this.modelValue,
                fallbackId: uniqueId('search'),
                commit: null,
            }
        },

        computed: {
            inputId(): string {
                return this.id || this.fallbackId
            },
        },

        watch: {
            // The value changed from outside (a filter reset): show it.
            modelValue(value: string) {
                if (value !== this.text) {
                    this.commit?.cancel()
                    this.text = value
                }
            },
        },

        created() {
            this.commit = makeDebounced((value: string) => this.emitValue(value), this.debounce)
        },

        beforeUnmount() {
            // Don't lose the last keystrokes: emit what is pending.
            this.commit?.flush()
        },

        methods: {
            emitValue(value: string) {
                if (value !== this.modelValue) { this.$emit('update:modelValue', value) }
            },

            onInput(e: Event) {
                this.text = (e.target as HTMLInputElement).value
                if (this.debounce <= 0) {
                    this.emitValue(this.text)
                } else {
                    this.commit?.(this.text)
                }
            },

            onKeydown(e: KeyboardEvent) {
                if (e.key === 'Enter') {
                    // Emit now what is waiting (once: the prop only catches up on the next render).
                    if (this.commit?.pending) { this.commit.flush() } else { this.emitValue(this.text) }
                    this.$emit('search', this.text)
                } else if (e.key === 'Escape' && this.text !== '') {
                    e.preventDefault()
                    e.stopPropagation()
                    this.clear()
                }
            },

            clear() {
                this.commit?.cancel()
                this.text = ''
                this.emitValue('')
                this.focus()
            },

            /** Focuses the field (for a "/" or ⌘F shortcut). */
            focus() {
                (this.$refs.input as HTMLInputElement | undefined)?.focus()
            },
        },
    })
</script>

<style lang="scss" scoped>
.ui-search {
    position: relative;
    display: flex;
    align-items: center;
    min-width: 0;
}

.ui-search__icon {
    position: absolute;
    left: 12px;
    color: var(--text-muted);
    pointer-events: none;
}

.ui-search__input {
    width: 100%;
    height: var(--control-height);
    padding: 0 32px 0 34px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--radius-full);
    color: var(--text-primary);
    font-size: var(--font-size-sm);
    outline: none;
    appearance: none;
    box-shadow: inset 0 1px 2px rgba(0,0,0,.12);
    transition: border-color var(--transition), box-shadow var(--transition), background var(--transition);

    &:hover { border-color: var(--border-hover); }

    &::placeholder { color: var(--text-muted); }
    // The browser's own clear button: ours replaces it.
    &::-webkit-search-cancel-button { appearance: none; display: none; }

    &:focus,
    &:focus-visible {
        border-color: var(--focus-ring-color);
        background: color-mix(in srgb, var(--bg-elevated) 70%, transparent);
        box-shadow: 0 0 0 1px var(--focus-ring-color), 0 0 0 5px color-mix(in srgb, var(--accent) 14%, transparent), var(--glow-focus);
    }
}

.ui-search__clear {
    position: absolute;
    right: 4px;
}
</style>
