<template>
    <BaseModal
        bare
        size="sm"
        :labelledby="titleId"
        :describedby="bodyId"
        :dismissible="!busy"
        :show-close="false"
        @close="cancel"
        @keydown="onKeydown"
    >
        <div class="confirm-dialog" :class="{ '-danger': danger }">
            <h2 :id="titleId" class="confirm-title">
                <vue-feather v-if="danger" type="alert-triangle" size="16" class="confirm-icon" aria-hidden="true" />
                {{ title }}
            </h2>
            <div :id="bodyId" class="confirm-body">
                <p v-if="message" class="confirm-message">{{ message }}</p>
                <p v-if="details" class="confirm-details">{{ details }}</p>
                <slot />
            </div>
            <p v-if="error" class="confirm-error" role="alert">{{ error }}</p>
            <div class="confirm-actions">
                <BaseButton
                    variant="ghost"
                    :disabled="busy"
                    :data-autofocus="danger ? '' : undefined"
                    @click="cancel"
                >
                    {{ cancelLabel || $t('common.cancel') }}
                </BaseButton>
                <BaseButton
                    :variant="danger ? 'danger' : 'primary'"
                    :loading="busy"
                    :data-autofocus="danger ? undefined : ''"
                    @click="confirm"
                >
                    {{ confirmLabel || $t('common.confirm') }}
                </BaseButton>
            </div>
        </div>
    </BaseModal>
</template>

<script lang="ts">
    /**
     * An accessible confirmation dialog, built on BaseModal.
     *
     * Mount it with `v-if` when a confirmation is needed; it emits `confirm`
     * or `cancel` and the parent unmounts it. Keep it mounted with `busy` set
     * while the confirmed action runs (buttons inactive, Escape ignored), and
     * pass `error` to show why it failed without closing.
     *
     * Keyboard and focus (WAI-ARIA dialog pattern; BaseModal does the rest:
     * top layer, focus trap, focus restored on close):
     * - focus starts on Cancel for `danger` dialogs, so an accidental Enter
     *   or Space can't destroy anything, otherwise on the confirm button;
     * - Escape cancels (the top dialog only);
     * - Enter confirms, except when a button has focus: then that button is
     *   activated as usual, so Enter on Cancel cancels.
     *
     * Key events don't leave the dialog, so document-level shortcuts (the
     * library view's, another modal's) don't also react to them.
     */
    import { defineComponent } from 'vue'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import { uniqueId } from '@/utils/dom'

    export default defineComponent({
        name: 'ConfirmDialog',

        components: { BaseModal, BaseButton },

        props: {
            title:        { type: String, required: true },
            message:      { type: String, default: '' },
            /** A second, smaller paragraph (consequences, where files go…). */
            details:      { type: String, default: '' },
            confirmLabel: { type: String, default: '' },
            cancelLabel:  { type: String, default: '' },
            /** Destructive action: red confirm button, initial focus on Cancel. */
            danger:       { type: Boolean, default: false },
            /** The confirmed action is running: buttons inactive, Escape ignored. */
            busy:         { type: Boolean, default: false },
            /** Shown under the text when the confirmed action failed. */
            error:        { type: String, default: '' },
        },

        emits: ['confirm', 'cancel'],

        data() {
            return {
                titleId: uniqueId('confirm-title'),
                bodyId:  uniqueId('confirm-body'),
            }
        },

        methods: {
            confirm() {
                if (!this.busy) { this.$emit('confirm') }
            },

            cancel() {
                if (!this.busy) { this.$emit('cancel') }
            },

            /** Keys BaseModal passes on (it handles Escape and Tab itself). */
            onKeydown(e: KeyboardEvent) {
                if (e.key !== 'Enter') { return }
                const target = e.target as HTMLElement | null
                if (target && target.closest('button, a, textarea')) { return }
                e.preventDefault()
                this.confirm()
            },
        },
    })
</script>

<style lang="scss" scoped>
.confirm-dialog {
    padding: 20px 22px 16px;
}

.confirm-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
    margin: 0 0 10px;
}

.confirm-icon {
    color: var(--danger);
    flex-shrink: 0;
}

.confirm-body {
    font-size: var(--font-size-sm);
    line-height: var(--line-height);
    color: var(--text-secondary);
}

.confirm-message {
    margin: 0 0 8px;
    color: var(--text-primary);
}

.confirm-details {
    margin: 0;
    font-size: var(--font-size-xs);
}

.confirm-error {
    margin: 12px 0 0;
    padding: 8px 10px;
    font-size: var(--font-size-xs);
    color: var(--danger);
    background: var(--danger-muted);
    border-radius: var(--radius-sm);
    word-break: break-word;
}

.confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 18px;
}
</style>
