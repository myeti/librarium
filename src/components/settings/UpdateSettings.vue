<template>
    <SettingsSection id="updates">
        <SettingRow :label="$t('settings.updates.title')" stacked>
            <template #description>
                <template v-if="version">{{ $t('settings.updates.currentVersion', { version }) }} </template>
                <template v-if="updatesStore.newRelease">
                    {{ $t('settings.updates.newRelease', { version: updatesStore.newRelease.version }) }} — {{ $t('settings.updates.newReleaseBody') }}
                </template>
                <template v-else>{{ $t('settings.updates.upToDate') }}</template>
            </template>
            <div class="settings-stack">
                <!-- The release notes come from the update server: rendered
                     from a parsed tree, never as HTML (see utils/safeText.ts). -->
                <details v-if="releaseNotes.length > 0" class="release-notes">
                    <summary>{{ $t('settings.updates.releaseNotes') }}</summary>
                    <p v-for="(paragraph, pi) in releaseNotes" :key="pi">
                        <template v-for="(node, ni) in paragraph" :key="ni">
                            <br v-if="node.type === 'br'" />
                            <a
                                v-else-if="node.type === 'link'"
                                :href="node.href"
                                target="_blank"
                                rel="noopener noreferrer"
                            >{{ node.text }}</a>
                            <template v-else>{{ node.text }}</template>
                        </template>
                    </p>
                </details>
                <p v-if="updatesStore.installError" class="settings-error" role="alert">
                    <vue-feather type="alert-circle" size="14" aria-hidden="true" />
                    {{ updatesStore.installError }}
                </p>
                <div class="settings-actions">
                    <BaseButton
                        v-if="updatesStore.hasNewRelease"
                        variant="primary"
                        icon="arrow-up-circle"
                        :loading="updatesStore.installing"
                        @click="updatesStore.downloadAndInstall()"
                    >
                        {{ updatesStore.installing ? $t('settings.updates.installing') : $t('settings.updates.download') }}
                    </BaseButton>
                    <template v-else>
                        <Badge tone="success" icon="check">{{ $t('settings.updates.upToDateBadge') }}</Badge>
                        <BaseButton size="sm" variant="ghost" icon="refresh-cw" :loading="checking" @click="checkNow">
                            {{ checking ? $t('settings.updates.checking') : $t('settings.updates.check') }}
                        </BaseButton>
                    </template>
                </div>
            </div>
        </SettingRow>

        <!-- The two connections made at launch (PRIVACY.md): each can be turned off. -->
        <SettingRow :label="$t('settings.updates.auto.title')" :description="$t('settings.updates.auto.description')" control-id="auto-check-updates">
            <SaveStatus for="updates.auto" />
            <Checkbox id="auto-check-updates" :model-value="settingsStore.autoCheckUpdates" @update:model-value="setAutoCheck" />
        </SettingRow>

        <SettingRow :label="$t('settings.updates.news.title')" :description="$t('settings.updates.news.description')" control-id="show-news">
            <SaveStatus for="updates.news" />
            <Checkbox id="show-news" :model-value="settingsStore.showNews" @update:model-value="setShowNews" />
        </SettingRow>
    </SettingsSection>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { getVersion } from '@tauri-apps/api/app'
    import { useUpdatesStore } from '@/stores/updates'
    import { useSettingsStore } from '@/stores/settings'
    import { useSaveStatusStore } from '@/stores/saveStatus'
    import { parseSafeText, type SafeParagraph } from '@/utils/safeText'
    import SettingRow from '@/components/ui/SettingRow.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import Badge from '@/components/ui/Badge.vue'
    import Checkbox from '@/components/ui/Checkbox.vue'
    import SettingsSection from './SettingsSection.vue'
    import SaveStatus from './SaveStatus.vue'

    /**
     * Settings › Updates: this version, whether a newer one is available
     * (with its release notes, as safe text), install it, or check again;
     * and the two launch-time connections, each on by default and
     * switchable: the update check (GitHub) and the "What's new" news
     * (the Librarium server). They take effect at the next launch
     * (stores/updates.ts `checkAtStartup`).
     */
    export default defineComponent({
        name: 'UpdateSettings',

        components: { SettingsSection, SettingRow, BaseButton, Badge, Checkbox, SaveStatus },

        data() {
            return { version: '', checking: false }
        },

        computed: {
            ...mapStores(useUpdatesStore, useSettingsStore, useSaveStatusStore),

            /** The pending update's release notes, as safe paragraphs (never HTML). */
            releaseNotes(): SafeParagraph[] {
                return parseSafeText(this.updatesStore.newRelease?.changelog)
            },
        },

        async mounted() {
            this.version = await getVersion().catch(() => '')
        },

        methods: {
            async checkNow() {
                this.checking = true
                try {
                    await this.updatesStore.checkReleases()
                } finally {
                    this.checking = false
                }
            },

            setAutoCheck(enabled: boolean) {
                void this.saveStatusStore.track('updates.auto', this.settingsStore.setAutoCheckUpdates(enabled))
            },

            setShowNews(enabled: boolean) {
                void this.saveStatusStore.track('updates.news', this.settingsStore.setShowNews(enabled))
            },
        },
    })
</script>

<style lang="scss" scoped>
.release-notes {
    font-size: var(--font-size-sm);
    color: var(--text-secondary);

    summary {
        cursor: pointer;
        font-weight: var(--font-weight-medium);
        color: var(--accent-text);
    }

    p { margin: var(--space-2) 0 0; }
    a { color: var(--accent-text); text-decoration: underline; }
}
</style>
