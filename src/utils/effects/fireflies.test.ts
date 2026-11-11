import { describe, expect, it } from 'vitest'
import type { Random } from './canvasLoop'
import { fireflyCount, FirefliesScene } from './fireflies'

/** Returns `values` in order, then `rest` forever (0.5 makes every random kick 0). */
function scripted(values: number[], rest = 0.5): Random {
    let i = 0
    return () => (i < values.length ? values[i++] : rest)
}

// A newcomer draws x, y, vx, vy, lifetime, size, hue, then its starting age.
const MID_WINDOW = [0.5, 0.5, 0.9, 0.1, 0.5, 0.5, 0.5, 0.2]

function scene(random: Random, count = 1) {
    const s = new FirefliesScene({ count, hueMin: 82, hueMax: 128, random })
    s.resize(800, 600)
    s.step(0)
    return s
}

function run(s: FirefliesScene, hz: number, seconds: number) {
    for (let i = 0; i < Math.round(hz * seconds); i++) {
        s.step(1000 / hz)
    }
}

describe('FirefliesScene', () => {
    it('moves a firefly as far in a second at 60 Hz as at 120 Hz', () => {
        const at60 = scene(scripted(MID_WINDOW))
        const at120 = scene(scripted(MID_WINDOW))
        const f60 = at60.fireflies[0]
        const f120 = at120.fireflies[0]
        expect(f60).toMatchObject({ x: 400, y: 300 })
        run(at60, 60, 1)
        run(at120, 120, 1)
        expect(at60.fireflies[0]).toBe(f60)
        expect(f60.x).toBeCloseTo(400 + 0.2 * 60, 6)
        expect(f120.x).toBeCloseTo(f60.x, 6)
        expect(f120.y).toBeCloseTo(f60.y, 6)
        expect(f120.life).toBeCloseTo(f60.life, 6)
    })

    it('nudges a firefly back from an edge at the same rate at any frame rate', () => {
        const at60 = scene(scripted(MID_WINDOW))
        const at120 = scene(scripted(MID_WINDOW))
        for (const s of [at60, at120]) {
            Object.assign(s.fireflies[0], { x: 10, vx: -0.2, vy: 0 })
        }
        run(at60, 60, 0.2)
        run(at120, 120, 0.2)
        expect(at60.fireflies[0].vx).toBeCloseTo(-0.2 + 0.014 * 12, 6)
        expect(at120.fireflies[0].vx).toBeCloseTo(at60.fireflies[0].vx, 6)
        // Positions integrate that change step by step: they agree to a fraction of a pixel.
        expect(Math.abs(at120.fireflies[0].x - at60.fireflies[0].x)).toBeLessThan(0.1)
    })

    it('keeps its count, replacing the fireflies that fade out', () => {
        const s = scene(Math.random, 18)
        expect(s.fireflies).toHaveLength(18)
        run(s, 60, 15)
        expect(s.fireflies).toHaveLength(18)
        s.setCount(9)
        expect(s.fireflies).toHaveLength(9)
    })

    it('spawns within its hue range, and follows a new range', () => {
        const s = scene(Math.random, 30)
        expect(s.fireflies.every(f => f.hue >= 82 && f.hue < 128)).toBe(true)
        s.setHueRange(22, 48)
        s.setCount(0)
        s.setCount(30)
        s.step(0)
        expect(s.fireflies.every(f => f.hue >= 22 && f.hue < 48)).toBe(true)
    })
})

describe('fireflyCount', () => {
    it('halves the count at the subtle level', () => {
        expect(fireflyCount(18, 'on')).toBe(18)
        expect(fireflyCount(18, 'subtle')).toBe(9)
        expect(fireflyCount(15, 'subtle')).toBe(8)
        expect(fireflyCount(1, 'subtle')).toBe(1)
        expect(fireflyCount(0, 'subtle')).toBe(0)
        expect(fireflyCount(-3, 'on')).toBe(0)
    })
})
