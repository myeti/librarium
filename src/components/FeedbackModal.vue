<template>
    <BaseModal
        :width="520"
        :title="step === 'write' ? $t('feedback.title') : $t('feedback.doneTitle')"
        :dirty="dirty"
        :dismissible="!sending"
        close-on-backdrop
        @close="$emit('close')"
    >
        <!-- Write step -->
        <template v-if="step === 'write'">
            <p class="feedback-hint">{{ $t('feedback.hint') }}</p>
            <TextField
                v-model="message"
                multiline
                :rows="6"
                :label="$t('feedback.messageLabel')"
                :placeholder="$t('feedback.placeholder')"
                :maxlength="MAX_LENGTH"
                :disabled="sending"
                :aria-describedby="hintId"
                class="feedback-field"
                data-autofocus
                @keydown.meta.enter.prevent="submit"
                @keydown.ctrl.enter.prevent="submit"
            />
            <div class="feedback-footer-row">
                <span :id="hintId" class="send-hint">{{ $t('feedback.sendHint', { keys: sendKeys }) }}</span>
                <span class="char-count" :class="{ '-warn': message.length > WARN_LENGTH }">
                    {{ $t('feedback.charCount', { count: message.length, max: MAX_LENGTH }) }}
                </span>
            </div>
            <!-- What goes with the message: the values feedbackAttachments() sends. -->
            <div class="feedback-attached">
                <p class="feedback-attached__title">{{ $t('feedback.attachedTitle') }}</p>
                <ul class="feedback-attached__list">
                    <li>{{ $t('feedback.attachedVersion', { version: attachments.version }) }}</li>
                    <li>{{ $t('feedback.attachedSystem', { system: systemLabel }) }}</li>
                    <li v-if="deviceLabel">{{ $t('feedback.attachedDevice', { device: deviceLabel }) }}</li>
                    <li>{{ $t('feedback.attachedId') }}</li>
                </ul>
            </div>
            <!-- Stays open on failure: nothing typed is lost. -->
            <p v-if="error" class="feedback-error" role="alert">
                <vue-feather type="alert-circle" size="14" aria-hidden="true" />
                <span>{{ error }}</span>
            </p>
        </template>

        <!-- Done step -->
        <div v-else class="feedback-done">
            <vue-feather type="check-circle" size="40" class="done-icon" aria-hidden="true" />
            <h3 class="done-title">{{ $t('feedback.doneHeading') }}</h3>
            <p class="done-body">{{ $t('feedback.doneBody') }}</p>
        </div>

        <template #footer>
            <template v-if="step === 'write'">
                <BaseButton variant="ghost" :disabled="sending" @click="$emit('close')">{{ $t('common.cancel') }}</BaseButton>
                <BaseButton
                    variant="primary"
                    icon="send"
                    :disabled="!message.trim()"
                    :loading="sending"
                    @click="submit"
                >
                    {{ sending ? $t('feedback.sending') : $t('feedback.send') }}
                </BaseButton>
            </template>
            <BaseButton v-else variant="primary" data-autofocus @click="$emit('close')">{{ $t('common.close') }}</BaseButton>
        </template>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapState } from 'pinia'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import TextField from '@/components/ui/TextField.vue'
    import { feedbackAttachments, sendFeedback, type FeedbackAttachments, type FeedbackDevice } from '@/services/librarium-api'
    import { useDeviceStore } from '@/stores/device'
    import { errorMessage } from '@/utils/errors'
    import { formatCombo, isMacPlatform } from '@/constants/shortcuts'
    import { uniqueId } from '@/utils/dom'

    const MAX_LENGTH = 2000
    /** The counter turns to a warning colour (with the count, not colour alone) near the limit. */
    const WARN_LENGTH = 1800

    const OS_NAMES: Record<string, string> = { macos: 'macOS', windows: 'Windows', linux: 'Linux' }
    const BRAND_NAMES: Record<string, string> = { kindle: 'Kindle', kobo: 'Kobo' }

    /**
     * "Send feedback": a message to the developer (through the Librarium
     * API). The dialog lists what goes with it, with the actual values:
     * the app version, the OS and its version, the connected e-reader's
     * brand and model (never a serial number: safeDeviceModel), and the
     * random installation ID (PRIVACY.md, "Feedback").
     *
     * Nothing typed is lost by accident: Escape, × and a click outside ask
     * "Discard your changes?" once something is written (BaseModal's guard),
     * and a failed send keeps the dialog open with the reason.
     * ⌘/Ctrl+Enter sends.
     */
    export default defineComponent({
        name: 'FeedbackModal',

        components: { BaseModal, BaseButton, TextField },

        emits: ['close'],

        data(): { step: 'write' | 'done', message: string, sending: boolean, error: string, hintId: string } {
            return {
                step: 'write',
                message: '',
                sending: false,
                error: '',
                hintId: uniqueId('feedback-hint'),
            }
        },

        computed: {
            ...mapState(useDeviceStore, ['device']),

            /** The e-reader mentioned with the message, if one is connected. */
            feedbackDevice(): FeedbackDevice | null {
                const d = this.device
                return d ? { brand: d.deviceType, model: d.model ?? null } : null
            },

            /** Exactly what sendFeedback() adds to the message (besides the installation ID). */
            attachments(): FeedbackAttachments {
                return feedbackAttachments(this.feedbackDevice)
            },

            systemLabel(): string {
                const { os, osVersion } = this.attachments
                const name = OS_NAMES[os] ?? os
                return osVersion ? `${name} ${osVersion}` : name
            },

            /** "Kindle Paperwhite 5", "Kobo Clara HD": the brand, then the model unless it starts with it. */
            deviceLabel(): string {
                const { deviceBrand, deviceModel } = this.attachments
                if (!deviceBrand) { return '' }
                const brand = BRAND_NAMES[deviceBrand] ?? deviceBrand
                if (!deviceModel || deviceModel === 'unknown') { return brand }
                return deviceModel.toLowerCase().startsWith(brand.toLowerCase()) ? deviceModel : `${brand} ${deviceModel}`
            },

            dirty(): boolean {
                return this.step === 'write' && this.message.trim() !== ''
            },

            /** "⌘Enter" on macOS, "Ctrl+Enter" elsewhere. */
            sendKeys: (): string => formatCombo('Mod+Enter', isMacPlatform()),

            MAX_LENGTH: () => MAX_LENGTH,
            WARN_LENGTH: () => WARN_LENGTH,
        },

        methods: {
            async submit() {
                if (!this.message.trim() || this.sending) { return }
                this.sending = true
                this.error = ''
                try {
                    await sendFeedback(this.message.trim(), this.feedbackDevice)
                    this.step = 'done'
                } catch (e) {
                    console.warn('[feedback] not sent', errorMessage(e))
                    this.error = this.$t('feedback.failed')
                } finally {
                    this.sending = false
                }
            },
        },
    })
</script>

<style lang="scss" scoped>
.feedback-hint {
    margin: 0 0 var(--space-4);
    font-size: var(--font-size-sm);
    line-height: 1.6;
    color: var(--text-secondary);
}

.feedback-field :deep(textarea) {
    min-height: 140px;
    line-height: 1.6;
}

.feedback-footer-row {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
    margin-top: 6px;
}

.send-hint {
    font-size: var(--font-size-xs);
    color: var(--text-muted);
}

.char-count {
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);

    &.-warn { color: var(--warning); }
}

.feedback-attached {
    margin-top: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--text-secondary) 6%, transparent);
    font-size: var(--font-size-xs);
    line-height: var(--line-height);
    color: var(--text-secondary);
}

.feedback-attached__title {
    margin: 0 0 var(--space-1);
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
}

.feedback-attached__list {
    margin: 0;
    padding-left: var(--space-4);
}

.feedback-error {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: var(--space-3) 0 0;
    padding: var(--space-2) 10px;
    border-radius: var(--radius-sm);
    background: var(--danger-muted);
    font-size: var(--font-size-xs);
    color: var(--danger);

    svg { flex-shrink: 0; margin-top: 1px; }
}

.feedback-done {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-5) var(--space-4);
    text-align: center;
}

.done-icon { color: var(--success); }

.done-title {
    margin: 0;
    font-size: var(--font-size-xl);
    font-weight: var(--font-weight-bold);
    color: var(--text-primary);
}

.done-body {
    max-width: 340px;
    margin: 0;
    font-size: var(--font-size-sm);
    line-height: 1.6;
    color: var(--text-secondary);
}
</style>
