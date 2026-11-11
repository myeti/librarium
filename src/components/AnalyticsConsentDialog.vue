<template>
    <BaseModal
        bare
        :width="520"
        :labelledby="titleId"
        :describedby="introId"
        :dismissible="!saving"
        :show-close="false"
        @close="answer('denied')"
    >
        <div class="consent-dialog">
            <h2 :id="titleId" class="consent-title">
                <vue-feather type="bar-chart-2" size="16" aria-hidden="true" />
                {{ $t('consent.title') }}
            </h2>
            <p :id="introId" class="consent-text">{{ $t('consent.intro') }}</p>

            <!-- The same component as Settings › Privacy & analytics: same words. -->
            <AnalyticsDisclosure class="consent-disclosure" />

            <p class="consent-text -small">
                {{ $t('consent.change') }}
                <button type="button" class="consent-policy" :disabled="saving" @click="showPolicy = true">
                    {{ $t('consent.policy') }}
                </button>
            </p>

            <!-- Both answers look the same: no nudging towards "yes". -->
            <div class="consent-actions">
                <BaseButton :disabled="saving" data-autofocus @click="answer('denied')">
                    {{ $t('consent.deny') }}
                </BaseButton>
                <BaseButton :disabled="saving" @click="answer('granted')">
                    {{ $t('consent.grant') }}
                </BaseButton>
            </div>
        </div>

        <!-- On top of this dialog: Escape closes the policy only. -->
        <PrivacyPolicyDialog v-if="showPolicy" @close="showPolicy = false" />
    </BaseModal>
</template>

<script lang="ts">
    /**
     * One-time analytics consent prompt (decision D2: opt-in).
     *
     * Shown by App.vue while the analytics store has no recorded choice: on a
     * first run, and once for installs from before this dialog existed. The
     * text says exactly what would be sent; keep it in sync with
     * services/analytics.ts and `reportError` in services/librarium-api.ts.
     *
     * Escape counts as "No thanks" (the privacy-preserving answer), and focus
     * starts on that button for the same reason. The choice can be changed
     * later in Settings › Privacy & analytics. "Read the privacy policy"
     * opens PRIVACY.md above it. BaseModal provides the rest (top layer,
     * focus kept inside, focus restored afterwards).
     */
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import AnalyticsDisclosure from '@/components/AnalyticsDisclosure.vue'
    import PrivacyPolicyDialog from '@/components/PrivacyPolicyDialog.vue'
    import { useAnalyticsStore } from '@/stores/analytics'
    import type { AnalyticsConsent } from '@/services/analytics'
    import { uniqueId } from '@/utils/dom'

    export default defineComponent({
        name: 'AnalyticsConsentDialog',

        components: { BaseModal, BaseButton, AnalyticsDisclosure, PrivacyPolicyDialog },

        data() {
            return {
                saving: false,
                showPolicy: false,
                titleId: uniqueId('consent-title'),
                introId: uniqueId('consent-intro'),
            }
        },

        computed: {
            ...mapStores(useAnalyticsStore),
        },

        methods: {
            async answer(value: AnalyticsConsent) {
                if (this.saving) { return }
                this.saving = true
                try {
                    await this.analyticsStore.setConsent(value)
                } finally {
                    this.saving = false
                }
            },
        },
    })
</script>

<style lang="scss" scoped>
.consent-dialog {
    padding: 22px 24px 18px;
}

.consent-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 17px;
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
    margin: 0 0 10px;
}

.consent-text {
    font-size: var(--font-size-sm);
    line-height: 1.55;
    color: var(--text-secondary);
    margin: 0 0 10px;

    &.-small { font-size: var(--font-size-xs); }
}

.consent-disclosure {
    margin: 0 0 10px;
}

/* A link-looking button: it opens a dialog, it doesn't navigate. */
.consent-policy {
    padding: 0;
    border: none;
    background: none;
    font: inherit;
    color: var(--accent-text);
    text-decoration: underline;
    cursor: pointer;

    &:disabled { cursor: default; opacity: 0.6; }
}

.consent-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
}
</style>
