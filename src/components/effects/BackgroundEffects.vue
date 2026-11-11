<template>
    <StarfieldLayer v-if="sky" :variant="sky" :level="level" />
    <FirefliesLayer
        v-else-if="theme === 'folklore'"
        :hue-min="12" :hue-max="44" :count="16" :level="level"
    />
    <MagicParticles v-else :level="level" />
</template>

<script lang="ts">
    import { defineComponent, type PropType } from 'vue'
    import FirefliesLayer from './FirefliesLayer.vue'
    import MagicParticles from './MagicParticles.vue'
    import StarfieldLayer from './StarfieldLayer.vue'
    import type { StarfieldVariant } from '@/utils/effects/starfield'
    import { isEffectLevel, type EffectLevel, type ThemeName } from '@/utils/effects/types'

    /**
     * The decorative background animation of the theme (one at a time):
     *
     * | Theme    | Effect |
     * |----------|--------|
     * | Saphir   | gold dust and sparkles (the theme's accent hue) |
     * | Nocturne | a starfield with shooting stars |
     * | Folklore | drifting embers (lantern light) |
     * | Cerulean | silver moon dust, drifting |
     *
     * The user's "Off" setting is the parent not rendering this at all.
     */
    export default defineComponent({
        name: 'BackgroundEffects',

        components: { FirefliesLayer, MagicParticles, StarfieldLayer },

        props: {
            theme: { type: String as PropType<ThemeName>, required: true },
            level: { type: String as PropType<EffectLevel>, required: true, validator: isEffectLevel },
        },

        computed: {
            /** The theme's night sky, if it has one. */
            sky(): StarfieldVariant | null {
                return this.theme === 'nocturne' || this.theme === 'cerulean' ? this.theme : null
            },
        },
    })
</script>
