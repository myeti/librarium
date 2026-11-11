<template>
    <Teleport to="body">
        <div v-if="visible" class="debug-panel">
            <div class="debug-header">
                <span class="debug-title">{{ $t('debug.title') }}</span>
                <span class="debug-count">{{ logStore.entries.length }}</span>
                <button class="debug-btn" :title="$t('debug.copyAll')" @click="copy">
                    <vue-feather type="copy" size="13" />
                </button>
                <button class="debug-btn" :title="$t('debug.clear')" @click="logStore.clear()">
                    <vue-feather type="trash-2" size="13" />
                </button>
                <button class="debug-btn" :title="$t('common.close')" @click="visible = false">
                    <vue-feather type="x" size="13" />
                </button>
            </div>
            <div ref="bodyEl" class="debug-body">
                <div
                    v-for="entry in logStore.entries"
                    :key="entry.id"
                    class="debug-entry"
                    :class="[`-${entry.level}`, `-${entry.source}`]"
                >
                    <span class="debug-time">{{ entry.time }}</span>
                    <span class="debug-src">{{ entry.source === 'rust' ? 'RS' : 'JS' }}</span>
                    <span class="debug-msg">{{ entry.message }}</span>
                </div>
                <div v-if="logStore.entries.length === 0" class="debug-empty">{{ $t('debug.empty') }}</div>
            </div>
        </div>
    </Teleport>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { useLogStore } from '@/stores/log'
    import { useNotificationsStore } from '@/stores/notifications'
    import { useShortcutsStore } from '@/stores/shortcuts'
    import { errorMessage } from '@/utils/errors'

    /**
     * The developer console: the log store's entries (frontend console and
     * backend log events), toggled with Ctrl+Shift+D. It follows the newest
     * line while open.
     */
    export default defineComponent({
        name: 'DebugPanel',

        data() {
            return {
                visible: false,
            }
        },

        computed: {
            ...mapStores(useLogStore, useNotificationsStore, useShortcutsStore),

            entryCount(): number {
                return this.logStore.entries.length
            },
        },

        watch: {
            async entryCount() {
                if (!this.visible) {return}
                await this.$nextTick()
                const body = this.$refs.bodyEl as HTMLElement | undefined
                if (body) {body.scrollTop = body.scrollHeight}
            },
        },

        mounted() {
            // Ctrl+Shift+D (constants/shortcuts.ts), through the global shortcut listener.
            this.shortcutsStore.register('debug.toggle', this.toggle)
        },

        beforeUnmount() {
            this.shortcutsStore.unregister('debug.toggle', this.toggle)
        },

        methods: {
            toggle() {
                this.visible = !this.visible
            },

            async copy() {
                const text = this.logStore.entries
                    .map(e => `[${e.time}] [${e.source.toUpperCase()}] [${e.level.toUpperCase()}] ${e.message}`)
                    .join('\n')
                try {
                    await navigator.clipboard.writeText(text)
                } catch (e) {
                    this.notificationsStore.push('error', this.$t('errors.copyFailed', { error: errorMessage(e) }))
                }
            },
        },
    })
</script>

<style lang="scss">
.debug-panel {
    position: fixed;
    bottom: 16px;
    left: 50%;
    transform: translateX(-50%);
    width: min(860px, 96vw);
    height: 300px;
    background: #0f0f0f;
    border: 1px solid #333;
    border-radius: 8px;
    box-shadow: 0 8px 40px rgba(0, 0, 0, 0.7);
    display: flex;
    flex-direction: column;
    z-index: 9999;
    font-family: 'Menlo', 'Consolas', 'monospace';
    font-size: 11px;
}

.debug-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid #2a2a2a;
    flex-shrink: 0;
}

.debug-title {
    color: #888;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    flex: 1;
}

.debug-count {
    font-size: 10px;
    color: #555;
    background: #1e1e1e;
    border-radius: 99px;
    padding: 1px 6px;
}

.debug-btn {
    background: none;
    border: none;
    color: #555;
    cursor: pointer;
    padding: 3px;
    display: flex;
    border-radius: 4px;

    &:hover { color: #ccc; background: #222; }
}

.debug-body {
    flex: 1;
    overflow-y: auto;
    padding: 4px 0;
}

.debug-entry {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 2px 10px;
    line-height: 1.6;

    &:hover { background: #181818; }

    &.-error .debug-msg { color: #f87171; }
    &.-warn  .debug-msg { color: #fbbf24; }
    &.-info  .debug-msg { color: #d4d4d4; }

    &.-rust .debug-src { color: #f97316; }
    &.-frontend .debug-src { color: #60a5fa; }
}

.debug-time {
    color: #444;
    flex-shrink: 0;
    font-size: 10px;
}

.debug-src {
    font-size: 10px;
    font-weight: 700;
    flex-shrink: 0;
    width: 18px;
}

.debug-msg {
    word-break: break-all;
    white-space: pre-wrap;
}

.debug-empty {
    color: #444;
    padding: 20px 10px;
    text-align: center;
}
</style>
