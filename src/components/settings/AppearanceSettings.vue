<template>
    <SettingsSection id="appearance">
        <SettingRow :label="$t('settings.theme.title')" :description="$t('settings.theme.description')" stacked>
            <!-- Beside the title: under it, "Saved" would push the cards aside. -->
            <template #status>
                <SaveStatus for="appearance.theme" />
            </template>
            <template #default="{ labelId, descriptionId }">
                <ThemePicker
                    :model-value="themeStore.theme"
                    :labelledby="labelId"
                    :describedby="descriptionId"
                    @update:model-value="setTheme"
                />
            </template>
        </SettingRow>

        <SettingRow :label="$t('settings.effects.title')">
            <template #description>
                {{ $t('settings.effects.description') }}
                <!-- Until the user chooses: why the default is what it is. -->
                <span v-if="settingsStore.effects === null" class="settings-hint">{{ effectsDefaultHint }}</span>
            </template>
            <template #default="{ labelId, descriptionId }">
                <SaveStatus for="appearance.effects" />
                <ChoiceGroup
                    :model-value="settingsStore.effectsLevel"
                    :options="effectsOptions"
                    :labelledby="labelId"
                    :describedby="descriptionId"
                    @update:model-value="setEffects"
                />
            </template>
        </SettingRow>

        <SettingRow :label="$t('settings.language.title')" :description="$t('settings.language.description')">
            <SaveStatus for="appearance.language" />
            <SelectField
                class="appearance-select"
                hide-label
                :label="$t('settings.language.title')"
                :model-value="lang"
                :options="languageOptions"
                @update:model-value="setLanguage"
            />
        </SettingRow>

        <SettingRow :label="$t('settings.sounds.title')" :description="$t('settings.sounds.description')" control-id="sound-volume">
            <SaveStatus for="appearance.volume" />
            <vue-feather :type="volume === 0 ? 'volume-x' : 'volume-2'" size="16" class="volume-icon" aria-hidden="true" />
            <input
                id="sound-volume"
                type="range"
                min="0"
                max="100"
                step="5"
                class="volume-slider"
                :value="volume"
                :aria-valuetext="`${volume}%`"
                @input="previewVolume"
                @change="saveVolume"
            />
            <span class="volume-value" aria-hidden="true">{{ volume }}%</span>
        </SettingRow>

        <SettingRow :label="$t('settings.sounds.launch')" control-id="launch-sound">
            <SaveStatus for="appearance.launchSound" />
            <Checkbox id="launch-sound" :model-value="settingsStore.launchSound" @update:model-value="setLaunchSound" />
        </SettingRow>
    </SettingsSection>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { useThemeStore, isThemeName } from '@/stores/theme'
    import { useSettingsStore, isEffectsLevel } from '@/stores/settings'
    import { useSystemStore } from '@/stores/system'
    import { useSaveStatusStore } from '@/stores/saveStatus'
    import { saveConfigOrThrow } from '@/services/appConfig'
    import { currentLang, isLang, LANGS, type Lang } from '@/i18n'
    import SettingRow from '@/components/ui/SettingRow.vue'
    import SelectField from '@/components/ui/SelectField.vue'
    import Checkbox from '@/components/ui/Checkbox.vue'
    import type { SelectOption } from '@/components/ui/types'
    import SettingsSection from './SettingsSection.vue'
    import SaveStatus from './SaveStatus.vue'
    import ChoiceGroup from './ChoiceGroup.vue'
    import ThemePicker from './ThemePicker.vue'
    import type { ChoiceOption } from './sections'

    /** Each language in its own name. */
    const LANGUAGE_NAMES: Record<Lang, string> = { en: 'English', fr: 'Français' }

    /**
     * Settings › Appearance: the theme (four, each shown as a small title
     * page in its own colours: ThemePicker), the background effects
     * (Off / Subtle / On; the default follows "reduce motion"), the
     * language, the sound volume and the launch sound (off by default).
     *
     * Everything applies at once; each row says "Saved" (or why not).
     */
    export default defineComponent({
        name: 'AppearanceSettings',

        components: { SettingsSection, SettingRow, SelectField, Checkbox, ChoiceGroup, SaveStatus, ThemePicker },

        data() {
            return {
                /** The slider's position while it is dragged (saved when released). */
                draggedVolume: null as number | null,
            }
        },

        computed: {
            ...mapStores(useThemeStore, useSettingsStore, useSystemStore, useSaveStatusStore),

            lang(): Lang { return currentLang.value },

            /** 0–100. */
            volume(): number {
                return this.draggedVolume ?? Math.round(this.settingsStore.soundVolume * 100)
            },

            effectsOptions(): ChoiceOption[] {
                return [
                    { value: 'off', label: this.$t('settings.effects.off') },
                    { value: 'subtle', label: this.$t('settings.effects.subtle') },
                    { value: 'on', label: this.$t('settings.effects.on') },
                ]
            },

            /** Why the default is what it is (shown until the user chooses). */
            effectsDefaultHint(): string {
                return this.systemStore.prefersReducedMotion
                    ? this.$t('settings.effects.defaultReduced')
                    : this.$t('settings.effects.defaultSubtle')
            },

            languageOptions(): SelectOption[] {
                return LANGS.map(l => ({ value: l, label: LANGUAGE_NAMES[l] }))
            },
        },

        methods: {
            setTheme(value: string | number) {
                if (isThemeName(value)) { void this.saveStatusStore.track('appearance.theme', this.themeStore.setTheme(value)) }
            },

            setEffects(value: string) {
                if (isEffectsLevel(value)) { void this.saveStatusStore.track('appearance.effects', this.settingsStore.setEffects(value)) }
            },

            setLanguage(value: string | number) {
                if (!isLang(value)) { return }
                // Applied at once (and the menu bar follows: App.vue); saved here to say so.
                currentLang.value = value
                void this.saveStatusStore.track('appearance.language', saveConfigOrThrow({ lang: value }))
            },

            previewVolume(e: Event) {
                this.draggedVolume = (e.target as HTMLInputElement).valueAsNumber
            },

            saveVolume(e: Event) {
                const volume = (e.target as HTMLInputElement).valueAsNumber
                this.draggedVolume = null
                void this.saveStatusStore.track('appearance.volume', this.settingsStore.setSoundVolume(volume / 100))
            },

            setLaunchSound(value: boolean) {
                void this.saveStatusStore.track('appearance.launchSound', this.settingsStore.setLaunchSound(value))
            },
        },
    })
</script>

<style lang="scss" scoped>
.appearance-select {
    min-width: 200px;
}

.volume-icon {
    color: var(--text-muted);
}

.volume-slider {
    width: 140px;
    accent-color: var(--accent-strong);
}

.volume-value {
    min-width: 3.5em;
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-secondary);
    text-align: right;
}
</style>
