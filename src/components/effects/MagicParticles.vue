<template>
    <canvas ref="canvas" class="particles-canvas" aria-hidden="true"></canvas>
</template>

<script lang="ts">
    import { defineComponent, markRaw, type PropType } from 'vue'
    import { startCanvasLoop, type CanvasLoop } from '@/utils/effects/canvasLoop'
    import { accentHue, PARTICLE_COUNTS, ParticlesScene } from '@/utils/effects/particles'
    import { isEffectLevel, LEVEL_MAX_FPS, type EffectLevel } from '@/utils/effects/types'

    /** The hue of the theme's `--accent` (Autumn's orange when it isn't a colour we can read). */
    function readAccentHue(): number {
        return accentHue(getComputedStyle(document.documentElement).getPropertyValue('--accent'))
    }

    /**
     * The default background effect: dust, embers and sparkles in the theme's
     * accent hue, following theme changes (`data-theme` on <html>).
     */
    export default defineComponent({
        name: 'MagicParticles',

        props: {
            level: { type: String as PropType<EffectLevel>, default: 'on', validator: isEffectLevel },
        },

        data(): {
            /** The simulation and its loop: not reactive (markRaw), they run outside Vue. */
            scene: ParticlesScene | null
            loop: CanvasLoop | null
            observer: MutationObserver | null
        } {
            return { scene: null, loop: null, observer: null }
        },

        watch: {
            level(level: EffectLevel) {
                this.scene?.setCount(PARTICLE_COUNTS[level])
                this.loop?.setMaxFps(LEVEL_MAX_FPS[level])
            },
        },

        mounted() {
            const scene = markRaw(new ParticlesScene({ count: PARTICLE_COUNTS[this.level], hue: readAccentHue() }))
            const loop = startCanvasLoop(this.$refs.canvas as HTMLCanvasElement, scene, {
                maxFps: LEVEL_MAX_FPS[this.level],
            })
            if (!loop) {
                return // no 2D context: nothing to draw on
            }
            this.scene = scene
            this.loop = markRaw(loop)
            const observer = markRaw(new MutationObserver(() => {
                scene.setHue(readAccentHue())
            }))
            observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] })
            this.observer = observer
        },

        beforeUnmount() {
            this.loop?.destroy()
            this.observer?.disconnect()
            this.loop = null
            this.scene = null
            this.observer = null
        },
    })
</script>

<style scoped>
.particles-canvas {
    position: fixed;
    inset: 0;
    width: 100vw;
    height: 100vh;
    pointer-events: none;
    z-index: 0;
}
</style>
