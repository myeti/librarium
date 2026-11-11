<template>
    <SettingsSection id="advanced">
        <SettingRow :label="$t('settings.feedback.title')" :description="$t('settings.feedback.description')">
            <BaseButton icon="send" @click="showFeedback = true">{{ $t('settings.feedback.button') }}</BaseButton>
        </SettingRow>

        <SettingRow :label="$t('settings.logs.title')" :description="$t('settings.logs.description')">
            <BaseButton icon="file-text" @click="openLogFolder">{{ $t('settings.logs.open') }}</BaseButton>
        </SettingRow>

        <SettingRow :label="$t('settings.debugConsole.title')" :description="$t('settings.debugConsole.description', { keys: debugKeys })">
            <BaseButton icon="terminal" @click="toggleDebugConsole">{{ $t('settings.debugConsole.open') }}</BaseButton>
        </SettingRow>

        <FeedbackModal v-if="showFeedback" @close="showFeedback = false" />
    </SettingsSection>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { useShortcutsStore } from '@/stores/shortcuts'
    import { formatCombo, SHORTCUTS } from '@/constants/shortcuts'
    import SettingRow from '@/components/ui/SettingRow.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import FeedbackModal from '@/components/FeedbackModal.vue'
    import SettingsSection from './SettingsSection.vue'

    /**
     * Settings › Advanced: send feedback, show the log file (the menu's
     * Help › Open Log Folder) and the debug console (Ctrl+Shift+D). The last
     * two run the same commands as their shortcut and menu item.
     */
    export default defineComponent({
        name: 'AdvancedSettings',

        components: { SettingsSection, SettingRow, BaseButton, FeedbackModal },

        data() {
            return { showFeedback: false }
        },

        computed: {
            ...mapStores(useShortcutsStore),

            debugKeys(): string {
                return formatCombo(SHORTCUTS['debug.toggle'].keys[0], this.shortcutsStore.isMac)
            },
        },

        methods: {
            openLogFolder() {
                this.shortcutsStore.runCommand('help.logs')
            },

            toggleDebugConsole() {
                this.shortcutsStore.runCommand('debug.toggle')
            },
        },
    })
</script>
