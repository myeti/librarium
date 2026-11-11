import { describe, expect, it } from 'vitest'
import type { Random } from './canvasLoop'
import {
    meteorInterval, starCount, STARFIELD_PRESETS, StarfieldScene, twinkle, type StarfieldOptions,
} from './starfield'

/** A seeded generator, so a scene is the same on every run. */
function seeded(seed = 1): Random {
    let s = seed
    return () => {
        s = (s * 16807) % 2147483647
        return (s - 1) / 2147483646
    }
}

function scene(options: Partial<StarfieldOptions> = {}) {
    const s = new StarfieldScene({ ...STARFIELD_PRESETS.nocturne, count: 50, random: seeded(), ...options })
    s.resize(800, 600)
    return s
}

describe('StarfieldScene', () => {
    it('places its stars inside the window and keeps them there through a resize', () => {
        const s = scene()
        expect(s.stars).toHaveLength(50)
        const before = s.stars.map(star => [star.x, star.y])
        s.resize(1600, 900)
        s.step(16)
        expect(s.stars.map(star => [star.x, star.y])).toEqual(before)
        expect(s.stars.every(star => star.x >= 0 && star.x < 1 && star.y >= 0 && star.y < 1)).toBe(true)
    })

    it('twinkles without ever going dark', () => {
        const star = scene().stars[0]
        const samples = Array.from({ length: 200 }, (_, i) => twinkle(star, i * 50))
        expect(Math.min(...samples)).toBeGreaterThan(star.brightness * 0.34)
        expect(Math.max(...samples)).toBeLessThanOrEqual(star.brightness + 1e-9)
    })

    it('sends shooting stars about as often at 60 Hz as at 120 Hz', () => {
        const count = (hz: number) => {
            const s = scene({ meteorEveryMs: 1000, random: seeded(7) })
            let seen = 0
            let flying = 0
            for (let i = 0; i < hz * 120; i++) {
                s.step(1000 / hz)
                if (s.meteors.length > flying) { seen += s.meteors.length - flying }
                flying = s.meteors.length
            }
            return seen
        }
        // ~120 in two minutes at either rate (Poisson noise allowed).
        expect(count(60)).toBeGreaterThan(90)
        expect(count(60)).toBeLessThan(150)
        expect(count(120)).toBeGreaterThan(90)
        expect(count(120)).toBeLessThan(150)
    })

    it('lets each shooting star burn out', () => {
        const s = scene({ meteorEveryMs: 1 })
        s.step(16)
        expect(s.meteors.length).toBeGreaterThan(0)
        s.setMeteorInterval(null)
        for (let i = 0; i < 100; i++) { s.step(16) }
        expect(s.meteors).toHaveLength(0)
    })

    it('drifts the whole sky (Cerulean) and wraps round the edges', () => {
        const s = scene({ ...STARFIELD_PRESETS.cerulean, meteorEveryMs: null })
        const star = s.stars[0]
        star.x = 0.999
        s.step(1000)
        expect(star.x).toBeLessThan(0.5) // went past the right edge, back on the left
        const still = scene({ meteorEveryMs: null })
        const fixed = { x: still.stars[0].x, y: still.stars[0].y }
        still.step(1000)
        expect(still.stars[0]).toMatchObject(fixed)
    })

    it('changes its number of stars', () => {
        const s = scene()
        s.setCount(10)
        expect(s.stars).toHaveLength(10)
        s.setCount(30)
        expect(s.stars).toHaveLength(30)
    })
})

describe('levels', () => {
    it('halves the stars and the shooting stars at the subtle level', () => {
        expect(starCount('on')).toBe(180)
        expect(starCount('subtle')).toBe(90)
        expect(meteorInterval(10_000, 'subtle')).toBe(20_000)
        expect(meteorInterval(10_000, 'on')).toBe(10_000)
        expect(meteorInterval(null, 'on')).toBeNull()
    })
})
