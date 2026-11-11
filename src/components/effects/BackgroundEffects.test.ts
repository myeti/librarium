import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import BackgroundEffects from './BackgroundEffects.vue'
import FirefliesLayer from './FirefliesLayer.vue'
import MagicParticles from './MagicParticles.vue'
import StarfieldLayer from './StarfieldLayer.vue'
import { cleanup, mountUi } from '@/test/mountUi'
import type { FirefliesScene } from '@/utils/effects/fireflies'
import type { ParticlesScene } from '@/utils/effects/particles'
import type { StarfieldScene } from '@/utils/effects/starfield'

afterEach(cleanup)

const EFFECTS = { MagicParticles, FirefliesLayer, StarfieldLayer }
type EffectName = keyof typeof EFFECTS

const CASES: [string, EffectName][] = [
    ['saphir',   'MagicParticles'],
    ['nocturne', 'StarfieldLayer'],
    ['folklore', 'FirefliesLayer'],
    ['cerulean', 'StarfieldLayer'],
]

/** A 2D context that accepts every call (happy-dom has none). */
function fakeContext(): CanvasRenderingContext2D {
    const state: Record<string | symbol, unknown> = {}
    return new Proxy(state, {
        get(target, prop) {
            if (prop in target) {
                return target[prop]
            }
            if (prop === 'createRadialGradient' || prop === 'createLinearGradient') {
                return () => ({ addColorStop: () => {} })
            }
            return () => {}
        },
        set(target, prop, value) {
            target[prop] = value
            return true
        },
    }) as unknown as CanvasRenderingContext2D
}

/** Gives canvases a 2D context and runs animation frames by hand. */
function stubCanvas() {
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(fakeContext())
    let pending = new Map<number, FrameRequestCallback>()
    let nextId = 1
    let time = 0
    vi.spyOn(globalThis, 'requestAnimationFrame').mockImplementation(callback => {
        pending.set(nextId, callback)
        return nextId++
    })
    const cancel = vi.spyOn(globalThis, 'cancelAnimationFrame').mockImplementation(id => {
        pending.delete(id)
    })
    return {
        cancel,
        get pending() { return pending.size },
        frame(ms = 16) {
            time += ms
            const due = [...pending.values()]
            pending = new Map()
            due.forEach(callback => callback(time))
        },
    }
}

function setHidden(hidden: boolean) {
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden })
    document.dispatchEvent(new Event('visibilitychange'))
}

describe('BackgroundEffects', () => {
    it.each(CASES)('%s shows %s', (theme, name) => {
        const w = mountUi(BackgroundEffects, { props: { theme, level: 'on' } })
        for (const [other, component] of Object.entries(EFFECTS)) {
            expect(w.findComponent(component).exists()).toBe(other === name)
        }
    })

    it.each(CASES)('%s: the effect is hidden from assistive technology', (theme) => {
        const w = mountUi(BackgroundEffects, { props: { theme, level: 'on' } })
        expect((w.element as HTMLElement).getAttribute('aria-hidden')).toBe('true')
    })

    it.each(CASES)('%s passes the level down', (theme, name) => {
        const w = mountUi(BackgroundEffects, { props: { theme, level: 'subtle' } })
        expect(w.findComponent(EFFECTS[name]).props('level')).toBe('subtle')
    })

    it('gives Folklore\'s embers their warm colours and count', () => {
        const w = mountUi(BackgroundEffects, { props: { theme: 'folklore', level: 'on' } })
        expect(w.findComponent(FirefliesLayer).props()).toMatchObject({ hueMin: 12, hueMax: 44, count: 16 })
    })

    it('gives each night sky its own preset', () => {
        const nocturne = mountUi(BackgroundEffects, { props: { theme: 'nocturne', level: 'on' } })
        expect(nocturne.findComponent(StarfieldLayer).props('variant')).toBe('nocturne')
        const cerulean = mountUi(BackgroundEffects, { props: { theme: 'cerulean', level: 'on' } })
        expect(cerulean.findComponent(StarfieldLayer).props('variant')).toBe('cerulean')
    })

    it('switches effect when the theme changes', async () => {
        const w = mountUi(BackgroundEffects, { props: { theme: 'saphir', level: 'on' } })
        await w.setProps({ theme: 'nocturne' })
        expect(w.findComponent(StarfieldLayer).exists()).toBe(true)
        expect(w.findComponent(MagicParticles).exists()).toBe(false)
        await w.setProps({ theme: 'folklore' })
        expect(w.findComponent(FirefliesLayer).exists()).toBe(true)
    })
})

describe('canvas effects', () => {
    it.each(['MagicParticles', 'FirefliesLayer', 'StarfieldLayer'] as const)('%s does nothing without a 2D context', name => {
        const raf = vi.spyOn(globalThis, 'requestAnimationFrame')
        const w = mountUi(EFFECTS[name], { props: { level: 'on' } })
        expect((w.element as HTMLElement).tagName).toBe('CANVAS')
        expect(raf).not.toHaveBeenCalled()
        w.unmount()
    })

    it('MagicParticles: sizes the canvas, runs its loop, follows the level, stops on unmount', async () => {
        const frames = stubCanvas()
        const removed = vi.spyOn(window, 'removeEventListener')
        const w = mountUi(MagicParticles, { props: { level: 'on' } })
        const canvas = w.element as HTMLCanvasElement
        const ratio = Math.min(window.devicePixelRatio || 1, 2)
        expect(canvas.width).toBe(Math.round(window.innerWidth * ratio))
        expect(frames.pending).toBe(1)
        frames.frame()
        const scene = (w.vm.$data as { scene: ParticlesScene }).scene
        expect(scene.particles).toHaveLength(100)

        await w.setProps({ level: 'subtle' })
        expect(scene.particles).toHaveLength(45)
        frames.frame()
        expect(scene.particles).toHaveLength(45)

        w.unmount()
        expect(frames.pending).toBe(0)
        expect(frames.cancel).toHaveBeenCalled()
        expect(removed).toHaveBeenCalledWith('resize', expect.any(Function))
    })

    it('MagicParticles: picks up the accent hue when the theme changes', async () => {
        stubCanvas()
        const root = document.documentElement
        const w = mountUi(MagicParticles, { props: { level: 'on' } })
        const scene = (w.vm.$data as { scene: ParticlesScene }).scene
        const setHue = vi.spyOn(scene, 'setHue')
        root.style.setProperty('--accent', '#0000ff')
        root.setAttribute('data-theme', 'cerulean')
        await new Promise(resolve => setTimeout(resolve, 0))
        root.removeAttribute('data-theme')
        root.style.removeProperty('--accent')
        expect(setHue).toHaveBeenCalled()
        expect(setHue.mock.calls[0][0]).toBe(240)
    })

    it('FirefliesLayer: shows half its count at the subtle level', async () => {
        const frames = stubCanvas()
        const w = mountUi(FirefliesLayer, { props: { count: 18, level: 'subtle' } })
        frames.frame()
        const scene = (w.vm.$data as { scene: FirefliesScene }).scene
        expect(scene.fireflies).toHaveLength(9)
        await w.setProps({ level: 'on' })
        frames.frame()
        expect(scene.fireflies).toHaveLength(18)
    })

    it('StarfieldLayer: fewer stars and shooting stars at the subtle level, a new sky for a new variant', async () => {
        const frames = stubCanvas()
        const w = mountUi(StarfieldLayer, { props: { variant: 'nocturne', level: 'subtle' } })
        frames.frame()
        const scene = () => (w.vm.$data as { scene: StarfieldScene }).scene
        expect(scene().stars).toHaveLength(90)
        await w.setProps({ level: 'on' })
        expect(scene().stars).toHaveLength(180)
        const before = scene()
        await w.setProps({ variant: 'cerulean' })
        expect(scene()).not.toBe(before)
        expect(scene().stars).toHaveLength(180)
        w.unmount()
        expect(frames.pending).toBe(0)
    })

    it('stop requesting frames while the window is hidden', () => {
        const frames = stubCanvas()
        mountUi(MagicParticles, { props: { level: 'on' } })
        frames.frame()
        try {
            setHidden(true)
            expect(frames.pending).toBe(0)
            setHidden(false)
            expect(frames.pending).toBe(1)
        } finally {
            Reflect.deleteProperty(document, 'hidden')
        }
    })
})
