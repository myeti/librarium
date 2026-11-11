<template>
    <label class="select-checkbox" data-select-toggle @click.stop>
        <input
            type="checkbox"
            class="select-checkbox__input"
            tabindex="-1"
            :checked="checked"
            :aria-label="label"
        />
        <span class="select-checkbox__box" aria-hidden="true">
            <vue-feather v-if="checked" type="check" size="12" />
        </span>
    </label>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'

    /**
     * The selection checkbox of a book card or list row: a real
     * `<input type="checkbox">` under a drawn box.
     *
     * It has no handler of its own: the collection (BookCollection) listens
     * to `change` events bubbling from `[data-select-toggle]` and toggles the
     * book, so one listener serves every card. The input is out of the Tab
     * order (`tabindex="-1"`): from the keyboard, Space on the focused book
     * does the same, and the book's `aria-selected` carries the state.
     * The click doesn't reach the card (it would open the book).
     */
    export default defineComponent({
        name: 'SelectCheckbox',

        props: {
            checked: { type: Boolean, default: false },
            /** "Select “Dune”". */
            label: { type: String, required: true },
        },
    })
</script>

<style lang="scss" scoped>
.select-checkbox {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    cursor: pointer;
}

.select-checkbox__input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    cursor: inherit;
}

.select-checkbox__box {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border: 1.5px solid var(--text-muted);
    border-radius: var(--radius-xs);
    background: var(--bg-input);
    color: var(--on-accent);
    box-shadow: var(--shadow-sm);
    transition: background var(--transition), border-color var(--transition);
    pointer-events: none;
}

.select-checkbox__input:checked + .select-checkbox__box {
    background: var(--accent-strong);
    border-color: var(--accent-strong);
}

.select-checkbox:hover .select-checkbox__box {
    border-color: var(--accent-text);
}
</style>
