<template>
    <SettingsSection id="privacy">
        <SettingRow :label="$t('settings.analytics.title')" :description="$t('settings.analytics.description')" stacked>
            <div class="settings-stack">
                <div class="settings-field-row">
                    <Checkbox
                        :model-value="analyticsStore.enabled"
                        :label="$t('settings.analytics.toggle')"
                        @update:model-value="setEnabled"
                    />
                    <SaveStatus for="privacy.analytics" />
                </div>
                <!-- The same component as the consent dialog: same words. -->
                <AnalyticsDisclosure />
            </div>
        </SettingRow>

        <!-- What Librarium contacts without the statistics (PRIVACY.md, "At a glance"). -->
        <SettingRow :label="$t('settings.connections.title')" :description="$t('settings.connections.description')" stacked>
            <ul class="settings-hints">
                <li>{{ settingsStore.autoCheckUpdates ? $t('settings.connections.updatesOn') : $t('settings.connections.updatesOff') }}</li>
                <li>{{ settingsStore.showNews ? $t('settings.connections.newsOn') : $t('settings.connections.newsOff') }}</li>
                <li>{{ $t('settings.connections.onDemand') }}</li>
                <li>{{ $t('settings.connections.feedback') }}</li>
            </ul>
            <div class="settings-actions">
                <RouterLink :to="{ path: '/settings', hash: '#updates' }" replace class="privacy-link">
                    {{ $t('settings.connections.manage') }}
                </RouterLink>
            </div>
        </SettingRow>

        <SettingRow :label="$t('settings.privacyPolicy.title')" :description="$t('settings.privacyPolicy.description')">
            <BaseButton icon="file-text" @click="showPolicy = true">{{ $t('settings.privacyPolicy.open') }}</BaseButton>
        </SettingRow>

        <PrivacyPolicyDialog v-if="showPolicy" @close="showPolicy = false" />
    </SettingsSection>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { RouterLink } from 'vue-router'
    import { useAnalyticsStore } from '@/stores/analytics'
    import { useSettingsStore } from '@/stores/settings'
    import { useSaveStatusStore } from '@/stores/saveStatus'
    import SettingRow from '@/components/ui/SettingRow.vue'
    import Checkbox from '@/components/ui/Checkbox.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import AnalyticsDisclosure from '@/components/AnalyticsDisclosure.vue'
    import PrivacyPolicyDialog from '@/components/PrivacyPolicyDialog.vue'
    import SettingsSection from './SettingsSection.vue'
    import SaveStatus from './SaveStatus.vue'

    /**
     * Settings › Privacy & analytics (also the menu's Help › Privacy): the
     * opt-in anonymous statistics (decision D2) with what they contain and
     * what is never sent (the consent dialog's own words: AnalyticsDisclosure),
     * the other connections Librarium makes (the two launch-time checks are
     * switched in Settings › Updates), and the privacy policy (PRIVACY.md).
     */
    export default defineComponent({
        name: 'PrivacySettings',

        components: { SettingsSection, SettingRow, Checkbox, BaseButton, SaveStatus, AnalyticsDisclosure, PrivacyPolicyDialog, RouterLink },

        data() {
            return { showPolicy: false }
        },

        computed: {
            ...mapStores(useAnalyticsStore, useSettingsStore, useSaveStatusStore),
        },

        methods: {
            setEnabled(enabled: boolean) {
                void this.saveStatusStore.track('privacy.analytics', this.analyticsStore.setConsent(enabled ? 'granted' : 'denied'))
            },
        },
    })
</script>

<style lang="scss" scoped>
.privacy-link {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--accent-text);
    text-decoration: underline;
}
</style>
