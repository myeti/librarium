<template>
    <div class="analytics-disclosure">
        <p class="analytics-disclosure__title">{{ $t('consent.whatTitle') }}</p>
        <ul class="analytics-disclosure__list">
            <li>{{ $t('consent.usage') }}</li>
            <li>{{ $t('consent.errors') }}</li>
        </ul>
        <p class="analytics-disclosure__never">
            <vue-feather type="lock" size="14" aria-hidden="true" />
            <span>{{ $t('consent.never') }}</span>
        </p>
    </div>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'

    /**
     * What the opt-in statistics send, and what they never send. The consent
     * dialog and Settings › Privacy & analytics both show this component, so
     * they always say the same thing; PRIVACY.md says it in detail. Keep the
     * `consent.*` texts true to services/analytics.ts and `reportError`
     * (services/librarium-api.ts).
     */
    export default defineComponent({
        name: 'AnalyticsDisclosure',
    })
</script>

<style lang="scss" scoped>
.analytics-disclosure {
    font-size: var(--font-size-sm);
    line-height: var(--line-height);
    color: var(--text-secondary);
}

.analytics-disclosure__title {
    margin: 0 0 6px;
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
}

.analytics-disclosure__list {
    list-style: disc;
    margin: 0 0 10px;
    padding-left: 20px;

    li + li { margin-top: var(--space-1); }
}

.analytics-disclosure__never {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin: 0;
    color: var(--text-primary);

    svg { flex-shrink: 0; margin-top: 3px; }
}
</style>
