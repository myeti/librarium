import { describe, expect, it } from 'vitest'
import type { Random } from './canvasLoop'
import { accentHue, FALLBACK_HUE, PARTICLE_COUNTS, ParticlesScene } from './particles'

/** Returns `values` in order, then `rest` forever (0.5 makes every random kick 0). */
function scripted(values: number[], rest = 0.5): Random {
    let i = 0
    return () => (i < values.length ? values[i++] : rest)
}

/** A 2D context that records the calls it gets (gradients included). */
function recordingContext() {
    const calls: string[] = []
    const state: Record<string | symbol, unknown> = {}
    const ctx = new Proxy(state, {
        get(target, prop) {
            if (prop in target) {
                return target[prop]
            }
            return (..._args: unknown[]) => {
                calls.push(String(prop))
                if (prop === 'createRadialGradient' || prop === 'createLinearGradient') {
                    return { addColorStop: () => {} }
                }
                return undefined
            }
        },
        set(target, prop, value) {
            target[prop] = value
            return true
        },
    })
    return { ctx: ctx as unknown as CanvasRenderingContext2D, calls }
}

// The random numbers a newcomer draws: kind roll, x, y, vx, vy, lifetime,
// size, hue (+ angle and spin for sparkles), then its starting life and, for
// dust and sparkles, a new position.
const DUST = [0.1, 0.5, 0.5, 0.9, 0.2, 0.5, 0.5, 0.5, 0.9, 0.5, 0.5]
const EMBER = [0.6, 0.5, 0.9, 0.5, 0.5, 0.5, 0.5, 0.9]
const SPARKLE = [0.9, 0.5, 0.5, 0.9, 0.2, 0.5, 0.5, 0.5, 0.5, 0.5, 0.9, 0.5, 0.5]

/** One particle, stepped for a second at `hz`. */
function oneSecond(script: number[], hz: number) {
    const scene = new ParticlesScene({ count: 1, hue: 30, random: scripted(script) })
    scene.resize(800, 600)
    scene.step(0)
    const p = scene.particles[0]
    const start = { ...p }
    for (let i = 0; i < hz; i++) {
        scene.step(1000 / hz)
    }
    expect(scene.particles[0]).toBe(p) // still the same particle
    return { start, end: { ...p } }
}

describe('ParticlesScene motion', () => {
    it.each([
        ['dust', DUST],
        ['ember', EMBER],
        ['sparkle', SPARKLE],
    ])('moves a %s as far in a second at 60 Hz as at 120 Hz', (kind, script) => {
        const at60 = oneSecond(script, 60)
        const at120 = oneSecond(script, 120)
        expect(at60.start.kind).toBe(kind)
        expect(at60.end.x).toBeCloseTo(at120.end.x, 6)
        expect(at60.end.y).toBeCloseTo(at120.end.y, 6)
        expect(at60.end.angle).toBeCloseTo(at120.end.angle, 6)
        expect(at60.end.life).toBeCloseTo(at120.end.life, 6)
        // …and as far as 60 frames of the old per-frame motion.
        expect(at60.end.x - at60.start.x).toBeCloseTo(at60.start.vx * 60, 6)
        expect(at60.end.y - at60.start.y).toBeCloseTo(at60.start.vy * 60, 6)
    })

    it('ages particles by time and replaces the ones that fade out', () => {
        const scene = new ParticlesScene({ count: 20, random: scripted([], 0.3) })
        scene.resize(800, 600)
        scene.step(0)
        const first = [...scene.particles]
        expect(first.every(p => p.kind === 'dust')).toBe(true)
        // Dust lives 7–14 s; after 20 s all of the first ones are gone.
        for (let i = 0; i < 200; i++) {
            scene.step(100)
        }
        expect(scene.particles).toHaveLength(20)
        expect(scene.particles.some(p => first.includes(p))).toBe(false)
        expect(scene.particles.every(p => p.life > 0 && p.life <= 1)).toBe(true)
    })

    it('spawns with the hue it is given', () => {
        const scene = new ParticlesScene({ count: 5, hue: 200, random: scripted([], 0.5) })
        scene.resize(800, 600)
        scene.step(0)
        expect(scene.particles.every(p => p.hue === 200)).toBe(true)
        scene.setHue(100)
        scene.setCount(10)
        scene.step(0)
        expect(scene.particles.slice(5).every(p => p.hue === 100)).toBe(true)
    })
})

describe('ParticlesScene levels', () => {
    it('keeps fewer particles at the subtle level', () => {
        expect(PARTICLE_COUNTS).toEqual({ subtle: 45, on: 100 })
        const subtle = new ParticlesScene({ count: PARTICLE_COUNTS.subtle })
        const on = new ParticlesScene({ count: PARTICLE_COUNTS.on })
        for (const scene of [subtle, on]) {
            scene.resize(800, 600)
            scene.step(0)
        }
        expect(subtle.particles).toHaveLength(45)
        expect(on.particles).toHaveLength(100)
    })

    it('changes its count on the fly', () => {
        const scene = new ParticlesScene({ count: 100 })
        scene.resize(800, 600)
        scene.step(0)
        scene.setCount(45)
        expect(scene.particles).toHaveLength(45)
        scene.setCount(100)
        scene.step(16)
        expect(scene.particles).toHaveLength(100)
    })
})

describe('ParticlesScene drawing', () => {
    it('draws every particle, with balanced save/restore', () => {
        const scene = new ParticlesScene({ count: 100 })
        scene.resize(800, 600)
        scene.step(0)
        const { ctx, calls } = recordingContext()
        scene.draw(ctx)
        const count = (name: string) => calls.filter(c => c === name).length
        expect(count('fill')).toBe(100)
        expect(count('save')).toBe(count('restore'))
        expect(count('save')).toBe(scene.particles.filter(p => p.kind === 'sparkle').length)
    })
})

describe('accentHue', () => {
    it('reads hex colours', () => {
        expect(accentHue('#ff0000')).toBe(0)
        expect(accentHue('#00ff00')).toBe(120)
        expect(accentHue('#0000FF')).toBe(240)
        expect(accentHue('#0f0')).toBe(120)
        expect(accentHue('#00f8')).toBe(240)
        expect(accentHue('#00ffff80')).toBe(180)
        expect(accentHue('  #c4531a ')).toBe(20)
    })

    it('reads rgb() and hsl() colours', () => {
        expect(accentHue('rgb(0, 0, 255)')).toBe(240)
        expect(accentHue('rgba(0, 255, 0, 0.5)')).toBe(120)
        expect(accentHue('rgb(0 100% 0 / 50%)')).toBe(120)
        expect(accentHue('hsl(200 50% 50%)')).toBe(200)
        expect(accentHue('hsla(210deg, 50%, 50%, 1)')).toBe(210)
        expect(accentHue('hsl(0.5turn 50% 50%)')).toBe(180)
        expect(accentHue('hsl(-90 50% 50%)')).toBe(270)
    })

    it('falls back for anything else', () => {
        expect(accentHue('')).toBe(FALLBACK_HUE)
        expect(accentHue('red')).toBe(FALLBACK_HUE)
        expect(accentHue('#zzz')).toBe(FALLBACK_HUE)
        expect(accentHue('#12345')).toBe(FALLBACK_HUE)
        expect(accentHue('color-mix(in srgb, red 50%, blue)')).toBe(FALLBACK_HUE)
        expect(accentHue('oklch(0.6 0.2 30)', 99)).toBe(99)
        expect(accentHue('rgb(a, b, c)')).toBe(FALLBACK_HUE)
    })

    it('gives 0 for greys, as before', () => {
        expect(accentHue('#808080')).toBe(0)
    })
})
