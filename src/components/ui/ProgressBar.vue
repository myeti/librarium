<template>
    <div
        class="ui-progress"
        :class="[`-${size}`, `-${tone}`, { '-indeterminate': indeterminate }]"
        role="progressbar"
        :aria-label="label"
        aria-valuemin="0"
        :aria-valuemax="max"
        :aria-valuenow="indeterminate ? undefined : clamped"
        :aria-valuetext="valueText || undefined"
    >
        <div class="ui-progress__track">
            <div class="ui-progress__fill" :style="indeterminate ? undefined : { width: `${percent}%` }"></div>
        </div>
        <span v-if="showValue && !indeterminate" class="ui-progress__value" aria-hidden="true">{{ Math.round(percent) }}%</span>
    </div>
</template>

<script lang="ts">
    /**
     * A progress bar: a track and a fill, announced as a progressbar with
     * its value (aria-valuenow, out of `max`).
     *
     * - `value: null` is indeterminate (moving stripe, no value announced);
     * - `label` is required: what is progressing ("Sending 3 books");
     * - `valueText` replaces the announced number ("12 of 40 books");
     * - `showValue` prints the percentage next to the bar.
     * The value is clamped to [0, max].
     */
    import { defineComponent, type PropType } from 'vue'
    import type { ProgressTone } from './types'

    export default defineComponent({
        name: 'ProgressBar',

        props: {
            value:     { type: Number as PropType<number | null>, default: null },
            max:       { type: Number, default: 100 },
            label:     { type: String, required: true },
            valueText: { type: String, default: '' },
            showValue: { type: Boolean, default: false },
            size:      { type: String as PropType<'sm' | 'md'>, default: 'sm' },
            tone:      { type: String as PropType<ProgressTone>, default: 'accent' },
        },

        computed: {
            indeterminate(): boolean {
                return this.value === null || Number.isNaN(this.value)
            },

            clamped(): number {
                if (this.value === null) { return 0 }
                return Math.min(Math.max(this.value, 0), this.max)
            },

            percent(): number {
                return this.max > 0 ? (this.clamped / this.max) * 100 : 0
            },
        },
    })
</script>

<style lang="scss" scoped>
.ui-progress {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
}

.ui-progress__track {
    position: relative;
    flex: 1;
    min-width: 0;
    background: var(--bg-hover);
    border-radius: var(--radius-full);
    overflow: hidden;
}

.-sm .ui-progress__track { height: 3px; }
.-md .ui-progress__track { height: 6px; }

.ui-progress__fill {
    height: 100%;
    border-radius: var(--radius-full);
    transition: width 300ms ease;
}

.-accent .ui-progress__fill  { background: linear-gradient(90deg, var(--accent), var(--secondary)); }
.-success .ui-progress__fill { background: var(--success); }
.-danger .ui-progress__fill  { background: var(--danger); }

.-indeterminate .ui-progress__fill {
    position: absolute;
    width: 35%;
    animation: ui-progress-slide 1.2s ease-in-out infinite;
}

@keyframes ui-progress-slide {
    from { left: -35%; }
    to   { left: 100%; }
}

@media (prefers-reduced-motion: reduce) {
    .-indeterminate .ui-progress__fill { animation-duration: 3s; }
}

.ui-progress__value {
    flex-shrink: 0;
    min-width: 3ch;
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-secondary);
    text-align: right;
}
</style>
