<template>
    <!-- Always rendered: a live region must exist before its text changes to be announced. -->
    <span class="save-status" :class="status ? `-${status.state}` : null" role="status">
        <template v-if="status?.state === 'saved'">
            <vue-feather type="check" size="12" aria-hidden="true" />
            {{ $t('settings.saved') }}
        </template>
        <template v-else-if="status?.state === 'error'">
            <vue-feather type="alert-circle" size="12" aria-hidden="true" />
            {{ $t('settings.notSaved', { error: status.error }) }}
        </template>
    </span>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { useSaveStatusStore, type SaveStatusEntry } from '@/stores/saveStatus'

    /**
     * "Saved" for a moment after a setting was saved, or "Not saved: why"
     * until it is saved again (stores/saveStatus.ts, by `for` key). Polite
     * live region: screen readers hear it without losing their place.
     */
    export default defineComponent({
        name: 'SaveStatus',

        props: {
            /** The setting's key, as given to `saveStatusStore.track`. */
            for: { type: String, required: true },
        },

        computed: {
            ...mapStores(useSaveStatusStore),

            status(): SaveStatusEntry | null {
                return this.saveStatusStore.statusOf(this.for)
            },
        },
    })
</script>

<style lang="scss" scoped>
.save-status {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    white-space: nowrap;

    &.-saved { color: var(--success); }
    &.-error { color: var(--danger); white-space: normal; }
}
</style>
