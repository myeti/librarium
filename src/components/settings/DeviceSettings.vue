<template>
    <SettingsSection id="devices">
        <SettingRow :label="$t('settings.device.title')">
            <template #description>
                {{ $t('settings.device.description') }}
                <span class="settings-hint">
                    {{ deviceStore.device ? $t('settings.device.connected', { name: deviceStore.device.name }) : $t('settings.device.none') }}
                </span>
            </template>
            <BaseButton icon="refresh-cw" :loading="deviceStore.detecting" @click="deviceStore.detectDevice()">
                {{ $t('settings.device.detect') }}
            </BaseButton>
        </SettingRow>

        <SettingRow :label="$t('settings.deviceSubfolder.title')" :description="$t('settings.deviceSubfolder.description')" stacked>
            <div class="settings-field-row">
                <TextField
                    id="device-subfolder"
                    v-model="subfolder"
                    class="settings-field"
                    :label="$t('settings.deviceSubfolder.label')"
                    :placeholder="$t('settings.deviceSubfolder.placeholder')"
                    autocomplete="off"
                    @blur="saveSubfolder"
                    @keydown.enter.prevent="saveSubfolder"
                />
                <SaveStatus for="devices.subfolder" />
            </div>
        </SettingRow>

        <!-- The sidebar links here (#mtp-setting) when an MTP Kindle is plugged in with this off. -->
        <SettingRow id="mtp-setting" :label="$t('settings.mtp.title')" :description="$t('settings.mtp.description')" stacked>
            <div class="settings-stack">
                <Checkbox
                    :model-value="settingsStore.mtpEnabled"
                    :label="$t('settings.mtp.enable')"
                    :hint="$t('settings.mtp.enableDesc')"
                    :disabled="savingMtp"
                    @update:model-value="setMtpEnabled"
                />
                <ul class="settings-hints">
                    <li>{{ $t('settings.mtp.macosHint') }}</li>
                    <li>{{ $t('settings.mtp.linuxHint') }}</li>
                </ul>
            </div>
        </SettingRow>
    </SettingsSection>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { useDeviceStore } from '@/stores/device'
    import { useSettingsStore } from '@/stores/settings'
    import { useNotificationsStore } from '@/stores/notifications'
    import { useSaveStatusStore } from '@/stores/saveStatus'
    import { errorMessage } from '@/utils/errors'
    import SettingRow from '@/components/ui/SettingRow.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import TextField from '@/components/ui/TextField.vue'
    import Checkbox from '@/components/ui/Checkbox.vue'
    import SettingsSection from './SettingsSection.vue'
    import SaveStatus from './SaveStatus.vue'

    /**
     * Settings › Devices: the connected e-reader (look for one), the folder
     * books go to on it, and experimental MTP support for 2024+ Kindles
     * (anchor `#mtp-setting`).
     *
     * The folder is saved when the field loses focus or on Enter, with
     * "Saved" next to it. MTP is applied by the backend as soon as it is
     * saved: a failed save puts the switch back and says why (toast), and a
     * poll starts at once to show what the Kindle does now.
     */
    export default defineComponent({
        name: 'DeviceSettings',

        components: { SettingsSection, SettingRow, BaseButton, TextField, Checkbox, SaveStatus },

        data() {
            return {
                subfolder: '',
                /** The MTP setting is being saved (its switch is disabled meanwhile). */
                savingMtp: false,
            }
        },

        computed: {
            ...mapStores(useDeviceStore, useSettingsStore, useNotificationsStore, useSaveStatusStore),
        },

        created() {
            this.subfolder = this.settingsStore.deviceSubfolder
        },

        methods: {
            saveSubfolder() {
                const value = this.subfolder.trim()
                this.subfolder = value
                if (value === this.settingsStore.deviceSubfolder) { return }
                void this.saveStatusStore.track('devices.subfolder', this.settingsStore.setDeviceSubfolder(value))
            },

            async setMtpEnabled(enabled: boolean) {
                this.savingMtp = true
                try {
                    await this.settingsStore.setMtpEnabled(enabled)
                    void this.deviceStore.pollDevice()
                } catch (e) {
                    this.notificationsStore.push('error', this.$t('errors.mtpSettingFailed', { error: errorMessage(e) }))
                } finally {
                    this.savingMtp = false
                }
            },
        },
    })
</script>
