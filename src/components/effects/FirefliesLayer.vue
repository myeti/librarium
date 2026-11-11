<template>
    <canvas ref="canvas" class="firefly-canvas" aria-hidden="true"></canvas>
</template>

<script lang="ts">
    import { defineComponent, markRaw, type PropType } from 'vue'
    import { startCanvasLoop, type CanvasLoop } from '@/utils/effects/canvasLoop'
    import { fireflyCount, FirefliesScene } from '@/utils/effects/fireflies'
    import { isEffectLevel, LEVEL_MAX_FPS, type EffectLevel } from '@/utils/effects/types'

    /** Glowing fireflies in a hue range (Folklore night: warm embers). */
    export default defineComponent({
        name: 'FirefliesLayer',

        props: {
            hueMin: { type: Number, default: 82 },
            hueMax: { type: Number, default: 128 },
            /** How many at the `on` level (`subtle` shows half). */
            count:  { type: Number, default: 18 },
            level:  { type: String as PropType<EffectLevel>, default: 'on', validator: isEffectLevel },
        },

        data(): {
            /** The simulation and its loop: not reactive (markRaw), they run outside Vue. */
            scene: FirefliesScene | null
            loop: CanvasLoop | null
        } {
            return { scene: null, loop: null }
        },

        computed: {
            shownCount(): number {
                return fireflyCount(this.count, this.level)
            },
        },

        watch: {
            shownCount(count: number) {
                this.scene?.setCount(count)
            },
            level(level: EffectLevel) {
                this.loop?.setMaxFps(LEVEL_MAX_FPS[level])
            },
            hueMin() {
                this.scene?.setHueRange(this.hueMin, this.hueMax)
            },
            hueMax() {
                this.scene?.setHueRange(this.hueMin, this.hueMax)
            },
        },

        mounted() {
            const scene = markRaw(new FirefliesScene({
                count: this.shownCount,
                hueMin: this.hueMin,
                hueMax: this.hueMax,
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

        beforeUnmount() {
            this.loop?.destroy()
            this.loop = null
            this.scene = null
        },
    })
</script>

<style scoped>
.firefly-canvas {
    position: fixed;
    inset: 0;
    width: 100vw;
    height: 100vh;
    pointer-events: none;
    z-index: 0;
}
</style>
