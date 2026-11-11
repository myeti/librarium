<template>
    <SettingsSection id="opds">
        <SettingRow :label="$t('settings.opds.title')" :description="$t('settings.opds.description')" stacked>
            <div class="settings-stack">
                <div class="settings-actions">
                    <Badge v-if="opdsStore.running" tone="success" icon="radio">
                        {{ $t('settings.opds.running', { port: opdsStore.status?.port ?? opdsStore.port }) }}
                    </Badge>
                    <Badge v-else tone="neutral" icon="pause-circle">{{ $t('settings.opds.stopped') }}</Badge>
                    <BaseButton
                        size="sm"
                        :variant="opdsStore.enabled ? 'ghost' : 'primary'"
                        :icon="opdsStore.enabled ? 'wifi-off' : 'wifi'"
                        :loading="toggling"
                        @click="toggleOpds"
                    >
                        {{ opdsStore.enabled ? $t('settings.opds.disable') : $t('settings.opds.enable') }}
                    </BaseButton>
                </div>

                <p v-if="opdsStore.error" class="settings-error" role="alert">
                    <vue-feather type="alert-circle" size="14" aria-hidden="true" />
                    {{ $t('settings.opds.startFailed', { error: opdsStore.error }) }}
                </p>

                <!-- This computer: always available while running. -->
                <div v-if="opdsStore.running" class="settings-copy-row">
                    <span class="settings-copy-row__label">{{ $t('settings.opds.localUrl') }}</span>
                    <span class="settings-copy-row__value">{{ opdsStore.localUrl }}</span>
                    <IconButton icon="copy" size="sm" :label="`${$t('settings.opds.copy')}: ${opdsStore.localUrl}`" @click="copy(opdsStore.localUrl)" />
                </div>

                <div class="settings-field-row">
                    <TextField
                        id="opds-port"
                        v-model="portInput"
                        class="settings-field -narrow"
                        type="text"
                        inputmode="numeric"
                        :label="$t('settings.opds.port')"
                        :hint="$t('settings.opds.portHint')"
                        :error="portError"
                        @blur="savePort"
                        @keydown.enter.prevent="savePort"
                    />
                    <SaveStatus for="opds.port" />
                </div>
            </div>
        </SettingRow>

        <!-- Local network sharing: off by default, password-protected. -->
        <SettingRow :label="$t('settings.opds.lanUrl')" stacked>
            <div class="settings-stack">
                <Checkbox
                    :model-value="opdsStore.shareOnLan"
                    :label="$t('settings.opds.shareOnLan')"
                    :hint="$t('settings.opds.shareOnLanDesc')"
                    @update:model-value="setShareOnLan"
                />

                <template v-if="opdsStore.shareOnLan">
                    <p class="settings-warning">
                        <vue-feather type="alert-triangle" size="14" aria-hidden="true" />
                        {{ $t('settings.opds.lanWarning') }}
                    </p>

                    <template v-if="opdsStore.running && opdsStore.lanUrls.length">
                        <div v-for="url in opdsStore.lanUrls" :key="url" class="settings-copy-row">
                            <span class="settings-copy-row__label">{{ $t('settings.opds.lanUrl') }}</span>
                            <span class="settings-copy-row__value">{{ url }}</span>
                            <IconButton icon="copy" size="sm" :label="`${$t('settings.opds.copy')}: ${url}`" @click="copy(url)" />
                        </div>
                    </template>
                    <p v-else-if="opdsStore.running" class="settings-hint">{{ $t('settings.opds.noLanUrl') }}</p>

                    <!-- The first LAN address, for a phone or tablet to scan (never the password). -->
                    <figure v-if="opdsStore.qrDataUrl" class="opds-qr">
                        <img :src="opdsStore.qrDataUrl" :alt="$t('settings.opds.qrAlt', { url: opdsStore.qrUrl ?? '' })" width="144" height="144" />
                        <figcaption class="settings-hint">{{ $t('settings.opds.qrCaption') }}</figcaption>
                    </figure>

                    <div class="settings-copy-row">
                        <span class="settings-copy-row__label">{{ $t('settings.opds.username') }}</span>
                        <span class="settings-copy-row__value">{{ opdsStore.username }}</span>
                        <IconButton icon="copy" size="sm" :label="`${$t('settings.opds.copy')}: ${$t('settings.opds.username')}`" @click="copy(opdsStore.username)" />
                    </div>
                    <div class="settings-copy-row">
                        <span class="settings-copy-row__label">{{ $t('settings.opds.password') }}</span>
                        <span class="settings-copy-row__value">{{ showPassword ? opdsStore.password : '•'.repeat(12) }}</span>
                        <IconButton
                            :icon="showPassword ? 'eye-off' : 'eye'"
                            size="sm"
                            :label="showPassword ? $t('settings.opds.hide') : $t('settings.opds.show')"
                            :pressed="showPassword"
                            @click="showPassword = !showPassword"
                        />
                        <IconButton icon="copy" size="sm" :label="`${$t('settings.opds.copy')}: ${$t('settings.opds.password')}`" @click="copy(opdsStore.password)" />
                        <IconButton icon="refresh-cw" size="sm" :label="$t('settings.opds.regenerate')" @click="confirmRegenerate = true" />
                    </div>
                </template>
            </div>
        </SettingRow>

        <!-- Helping a reading app connect. -->
        <ul v-if="opdsStore.running" class="settings-hints -boxed">
            <li v-if="opdsStore.shareOnLan && opdsStore.lanUrls.length">{{ $t('settings.opds.ipHint') }}</li>
            <li>{{ opdsStore.shareOnLan ? $t('settings.opds.koreaderHintLan') : $t('settings.opds.koreaderHint') }}</li>
            <li>{{ $t('settings.opds.opds2Hint', { url: opdsStore.opds2Url }) }}</li>
        </ul>

        <ConfirmDialog
            v-if="confirmRegenerate"
            :title="$t('settings.opds.regenerateTitle')"
            :message="$t('settings.opds.regenerateBody')"
            :confirm-label="$t('settings.opds.regenerate')"
            @confirm="regeneratePassword"
            @cancel="confirmRegenerate = false"
        />
    </SettingsSection>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { parsePort, useOpdsStore } from '@/stores/opds'
    import { useNotificationsStore } from '@/stores/notifications'
    import { useSaveStatusStore } from '@/stores/saveStatus'
    import { errorMessage } from '@/utils/errors'
    import SettingRow from '@/components/ui/SettingRow.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import IconButton from '@/components/ui/IconButton.vue'
    import TextField from '@/components/ui/TextField.vue'
    import Checkbox from '@/components/ui/Checkbox.vue'
    import Badge from '@/components/ui/Badge.vue'
    import ConfirmDialog from '@/components/ConfirmDialog.vue'
    import SettingsSection from './SettingsSection.vue'
    import SaveStatus from './SaveStatus.vue'

    /**
     * Settings › OPDS server: on/off and its real state, the addresses to
     * type in a reading app (copyable), the port, and sharing on the local
     * network (warning, LAN addresses, a QR code of the first one, the user
     * name and password, a new password after confirming). Behaviour as in
     * the opds store; the store restarts a running server when a setting
     * changes.
     *
     * The port is checked before it is saved: a value that isn't a whole
     * number from 1024 to 65535 shows an error in the field and is not used.
     */
    export default defineComponent({
        name: 'OpdsSettings',

        components: { SettingsSection, SettingRow, BaseButton, IconButton, TextField, Checkbox, Badge, ConfirmDialog, SaveStatus },

        data() {
            return {
                portInput: '',
                portError: '',
                showPassword: false,
                confirmRegenerate: false,
                toggling: false,
            }
        },

        computed: {
            ...mapStores(useOpdsStore, useNotificationsStore, useSaveStatusStore),

            /** The OPDS address shown as a QR code (sharing on the LAN and running), or null. */
            qrUrl(): string | null {
                return this.opdsStore.qrUrl
            },
        },

        watch: {
            // Made while this section is open, whenever the address changes.
            qrUrl: {
                handler() {
                    void this.opdsStore.refreshQr()
                },
                immediate: true,
            },
        },

        created() {
            this.portInput = String(this.opdsStore.port)
        },

        async mounted() {
            // The server may have stopped (or been started) since the store last heard of it.
            await this.opdsStore.refreshStatus()
        },

        methods: {
            async toggleOpds() {
                this.toggling = true
                try {
                    await this.opdsStore.toggle()
                } finally {
                    this.toggling = false
                }
            },

            savePort() {
                const port = parsePort(this.portInput)
                if (port === null) {
                    this.portError = this.$t('settings.opds.portInvalid')
                    return
                }
                this.portError = ''
                this.portInput = String(port)
                if (port === this.opdsStore.port) { return }
                void this.saveStatusStore.track('opds.port', this.opdsStore.setPort(port))
            },

            async setShareOnLan(value: boolean) {
                await this.opdsStore.setShareOnLan(value)
            },

            async regeneratePassword() {
                this.confirmRegenerate = false
                await this.opdsStore.regeneratePassword()
                this.showPassword = true
            },

            async copy(text: string) {
                try {
                    await navigator.clipboard.writeText(text)
                    this.notificationsStore.push('success', this.$t('settings.opds.copied'), 2000)
                } catch (e) {
                    this.notificationsStore.push('error', this.$t('errors.copyFailed', { error: errorMessage(e) }))
                }
            },
        },
    })
</script>

<style lang="scss" scoped>
.opds-qr {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    margin: 0;

    img {
        flex-shrink: 0;
        padding: var(--space-2);
        border-radius: var(--radius-sm);
        // A QR code needs a light background in every theme to be scanned.
        background: var(--white);
    }
}
</style>
