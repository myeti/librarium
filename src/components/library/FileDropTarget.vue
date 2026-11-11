<template>
    <DropZone :active="importsStore.dragging" />
    <!-- After an import: offer to convert MOBI/AZW/AZW3 books to EPUB. -->
    <EpubConvertModal
        v-if="importsStore.conversionOffer.length > 0"
        :books="importsStore.conversionOffer"
        @converted="importsStore.dismissConversionOffer()"
        @close="importsStore.dismissConversionOffer()"
    />
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { getCurrentWindow } from '@tauri-apps/api/window'
    import { useImportsStore } from '@/stores/imports'
    import DropZone from '@/components/DropZone.vue'
    import EpubConvertModal from '@/components/EpubConvertModal.vue'

    /**
     * Makes the whole window a drop target for ebook files, whatever view is
     * showing: App.vue mounts it once a user (so a library) is chosen. It
     * passes the window's drag-and-drop events to the imports store, shows
     * the drop overlay while files are dragged over, and the EPUB
     * conversion offer after any import.
     *
     * Tauri's native drag-and-drop events are used rather than the DOM's:
     * they carry real file paths on every platform (a DOM drop has no path
     * in WebView2 on Windows).
     */
    export default defineComponent({
        name: 'FileDropTarget',

        components: { DropZone, EpubConvertModal },

        data(): { unlisten: (() => void) | null, unmounted: boolean } {
            return { unlisten: null, unmounted: false }
        },

        computed: {
            ...mapStores(useImportsStore),
        },

        async mounted() {
            try {
                const unlisten = await getCurrentWindow().onDragDropEvent(event => {
                    void this.importsStore.onFileDrop(event.payload)
                })
                // Unmounted while the listener was being set up: drop it at once.
                if (this.unmounted) { unlisten() } else { this.unlisten = unlisten }
            } catch (e) {
                console.error('[drop] could not listen for dropped files', e)
            }
        },

        beforeUnmount() {
            this.unmounted = true
            this.unlisten?.()
            this.unlisten = null
            this.importsStore.dragging = false
        },
    })
</script>
