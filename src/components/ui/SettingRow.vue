<template>
    <div class="ui-setting" :class="{ '-stacked': stacked }">
        <div class="ui-setting__text">
            <div class="ui-setting__heading">
                <label v-if="controlId" :id="labelId" :for="controlId" class="ui-setting__label">{{ label }}</label>
                <span v-else :id="labelId" class="ui-setting__label">{{ label }}</span>
                <slot name="status" />
            </div>
            <p v-if="description || $slots.description" :id="descriptionId" class="ui-setting__description">
                <slot name="description">{{ description }}</slot>
            </p>
        </div>
        <div class="ui-setting__control">
            <slot :label-id="labelId" :description-id="description || $slots.description ? descriptionId : undefined" />
        </div>
    </div>
</template>

<script lang="ts">
    /**
     * One setting of the Settings screen: its name and description on the
     * left, the control on the right (below with `stacked`).
     *
     * The name labels the control. With a single native control, give it an
     * id and pass it as `control-id` (the name becomes its <label>).
     * Otherwise use the ids the slot provides:
     *
     *     <SettingRow :label="$t('settings.effects.title')" :description="…">
     *         <template #default="{ labelId, descriptionId }">
     *             <div role="radiogroup" :aria-labelledby="labelId" :aria-describedby="descriptionId">…</div>
     *         </template>
     *     </SettingRow>
     *
     * The `status` slot sits beside the name (a SaveStatus for a `stacked`
     * row, where it would otherwise push a wide control aside when it
     * appears).
     */
    import { defineComponent } from 'vue'
    import { uniqueId } from '@/utils/dom'

    export default defineComponent({
        name: 'SettingRow',

        props: {
            label:       { type: String, required: true },
            description: { type: String, default: '' },
            /** Id of the control the name labels (rendered as its <label for>). */
            controlId:   { type: String, default: '' },
            /** Control under the text rather than beside it (wide controls). */
            stacked:     { type: Boolean, default: false },
        },

        data() {
            const base = uniqueId('setting')
            return { labelId: `${base}-label`, descriptionId: `${base}-description` }
        },
    })
</script>

<style lang="scss" scoped>
.ui-setting {
    display: flex;
    align-items: center;
    gap: 20px;
    padding: 20px 24px;
    background: color-mix(in srgb, var(--text-secondary) 5%, transparent);
    border: 1px solid var(--border);
    border-radius: var(--radius);

    &.-stacked {
        flex-direction: column;
        align-items: stretch;
    }
}

.ui-setting__text {
    flex: 1;
    min-width: 0;
}

.ui-setting__heading {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-3);
    margin-bottom: var(--space-1);
}

.ui-setting__label {
    display: block;
    font-size: 15px;
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
}

.ui-setting__description {
    font-size: var(--font-size-sm);
    color: var(--text-secondary);
}

.ui-setting__control {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
}
</style>
