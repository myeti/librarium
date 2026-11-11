<template>
    <div role="separator" class="ui-menu-separator"></div>
</template>

<script lang="ts">
    /** A line between groups of items in a DropdownMenu. */
    import { defineComponent } from 'vue'

    export default defineComponent({
        name: 'MenuSeparator',
    })
</script>

<style lang="scss" scoped>
.ui-menu-separator {
    height: 1px;
    margin: var(--space-1) var(--space-2);
    background: var(--border);
}
</style>
