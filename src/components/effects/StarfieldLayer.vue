<template>
    <canvas ref="canvas" class="starfield-canvas" aria-hidden="true"></canvas>
</template>

<script lang="ts">
    import { defineComponent, markRaw, type PropType } from 'vue'
    import { startCanvasLoop, type CanvasLoop } from '@/utils/effects/canvasLoop'
    import {
        meteorInterval, starCount, STARFIELD_PRESETS, StarfieldScene, type StarfieldVariant,
    } from '@/utils/effects/starfield'
    import { isEffectLevel, LEVEL_MAX_FPS, type EffectLevel } from '@/utils/effects/types'

    /**
     * A twinkling night sky with shooting stars (Nocturne and Cerulean night
     * editions): see utils/effects/starfield.ts. The variant picks the
     * colours, how often a star falls, and Cerulean's slow drift.
     */
    export default defineComponent({
        name: 'StarfieldLayer',

        props: {
            variant: { type: String as PropType<StarfieldVariant>, default: 'nocturne' },
            level:   { type: String as PropType<EffectLevel>, default: 'on', validator: isEffectLevel },
        },

        data(): {
            /** The simulation and its loop: not reactive (markRaw), they run outside Vue. */
            scene: StarfieldScene | null
            loop: CanvasLoop | null
        } {
            return { scene: null, loop: null }
        },

        watch: {
            level(level: EffectLevel) {
                this.scene?.setCount(starCount(level))
                this.scene?.setMeteorInterval(meteorInterval(STARFIELD_PRESETS[this.variant].meteorEveryMs, level))
                this.loop?.setMaxFps(LEVEL_MAX_FPS[level])
            },
            // Another sky: start again with its colours.
            variant() {
                this.stop()
                this.start()
            },
        },

        mounted() {
            this.start()
        },

        beforeUnmount() {
            this.stop()
        },

        methods: {
            start() {
                const preset = STARFIELD_PRESETS[this.variant]
                const scene = markRaw(new StarfieldScene({
                    ...preset,
                    count: starCount(this.level),
                    meteorEveryMs: meteorInterval(preset.meteorEveryMs, this.level),
                }))
                const loop = startCanvasLoop(this.$refs.canvas as HTMLCanvasElement, scene, {
                    maxFps: LEVEL_MAX_FPS[this.level],
                })
                if (!loop) {
                    return // no 2D context: nothing to draw on
                }
                this.scene = scene
                this.loop = markRaw(loop)
            },

            stop() {
                this.loop?.destroy()
                this.loop = null
                this.scene = null
            },
        },
    })
</script>

<style scoped>
.starfield-canvas {
    position: fixed;
    inset: 0;
    width: 100vw;
    height: 100vh;
    pointer-events: none;
    z-index: 0;
}
</style>
