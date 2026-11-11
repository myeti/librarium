<template>
    <Transition name="fade">
        <div v-if="active" class="dropzone-overlay" aria-hidden="true">
            <div class="dropzone-content">
                <vue-feather type="upload-cloud" size="48" />
                <p class="dropzone-title">{{ $t('dropzone.title') }}</p>
                <p class="dropzone-sub">{{ $t('dropzone.formats') }}</p>
            </div>
        </div>
    </Transition>
    <!-- Said once to screen readers (the overlay itself is decorative). -->
    <p class="sr-only" aria-live="polite">{{ active ? $t('dropzone.title') : '' }}</p>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'

    /**
     * The overlay shown over the whole window while files are dragged over
     * it (on any view: FileDropTarget shows it). It never takes the pointer,
     * so the drop reaches the window underneath.
     */
    export default defineComponent({
        name: 'DropZone',

        props: {
            active: {
                type: Boolean,
                required: true,
            },
        },
    })
</script>

<style lang="scss" scoped>
.dropzone-overlay {
    position: fixed;
    inset: 0;
    z-index: var(--z-drop-zone);
    display: flex;
    align-items: center;
    justify-content: center;
    margin: var(--space-3);
    background: color-mix(in srgb, var(--bg) 82%, transparent);
    border: 3px dashed var(--accent);
    border-radius: var(--radius-lg);
    box-shadow: inset 0 0 64px color-mix(in srgb, var(--accent) 18%, transparent);
    pointer-events: none;

    .dropzone-content {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: var(--space-3);
        color: var(--accent-text);
    }

    .dropzone-title {
        font-size: var(--font-size-xl);
        font-weight: var(--font-weight-bold);
    }

    .dropzone-sub {
        font-size: var(--font-size-sm);
        color: var(--text-secondary);
    }
}
</style>
