/**
 * A night sky (Nocturne and Cerulean night editions): stars that twinkle in
 * place, now and then a shooting star, and, for Cerulean, a slow drift of
 * moon dust. Pure simulation and drawing; `CanvasLoop` runs it.
 *
 * Stars are placed in window fractions (0–1), so a resize stretches the sky
 * instead of re-drawing it. Speeds are in CSS pixels per 60 Hz frame (like
 * the other effects) and scaled by the elapsed time; durations are in ms.
 */
import { between, frames60, type EffectScene, type Random } from './canvasLoop'
import type { EffectLevel } from './types'

/** A star colour: HSL, the lightness of its core. */
export interface StarTint {
    hue: number
    saturation: number
    lightness: number
    /** How often this tint is drawn, relative to the others. */
    weight: number
}

export interface Star {
    /** Position, as a fraction of the window (0–1). */
    x: number
    y: number
    /** Core radius, CSS px. */
    size: number
    /** Brightness at the top of a twinkle (0–1). */
    brightness: number
    /** Twinkle: angular speed (radians per ms) and phase. */
    rate: number
    phase: number
    tint: StarTint
}

export interface Meteor {
    x: number
    y: number
    /** Velocity, CSS px per 60 Hz frame. */
    vx: number
    vy: number
    /** Tail length, CSS px. */
    length: number
    /** Age, from 0 (born) to 1 (gone). */
    life: number
    /** Lifetime, in ms. */
    maxLife: number
}

export interface StarfieldOptions {
    /** How many stars (see `starCount()`). */
    count: number
    tints: readonly StarTint[]
    /** Average time between two shooting stars, in ms; `null`: none. */
    meteorEveryMs: number | null
    /** Drift of the whole sky, CSS px per 60 Hz frame (wraps round). */
    driftX?: number
    driftY?: number
    random?: Random
}

/** The two skies: the preset of each theme that has one. */
export type StarfieldVariant = 'nocturne' | 'cerulean'

const WHITE: StarTint = { hue: 20, saturation: 30, lightness: 96, weight: 6 }

export const STARFIELD_PRESETS: Readonly<Record<StarfieldVariant, Omit<StarfieldOptions, 'count' | 'random'>>> = {
    // Warm white stars with a few coral and blue ones; a shooting star every ~10 s.
    nocturne: {
        tints: [
            WHITE,
            { hue: 8, saturation: 88, lightness: 74, weight: 1.4 },
            { hue: 226, saturation: 85, lightness: 82, weight: 1 },
        ],
        meteorEveryMs: 10_000,
    },
    // Silver and lilac dust, drifting slowly up and right; rare shooting stars.
    cerulean: {
        tints: [
            { hue: 218, saturation: 45, lightness: 94, weight: 6 },
            { hue: 258, saturation: 70, lightness: 86, weight: 1.2 },
        ],
        meteorEveryMs: 22_000,
        driftX: 0.05,
        driftY: -0.03,
    },
}

/** How many stars each level shows. */
export const STAR_COUNTS: Readonly<Record<EffectLevel, number>> = {
    subtle: 90,
    on: 180,
}

/** The subtle level waits this much longer between shooting stars. */
export const SUBTLE_METEOR_FACTOR = 2

/** The longest a shooting star crosses the sky, in ms. */
const METEOR_LIFE_MS: readonly [number, number] = [650, 1100]

function pickTint(tints: readonly StarTint[], random: Random): StarTint {
    const total = tints.reduce((sum, t) => sum + t.weight, 0)
    let roll = random() * total
    for (const tint of tints) {
        roll -= tint.weight
        if (roll < 0) { return tint }
    }
    return tints[tints.length - 1]
}

export function spawnStar(tints: readonly StarTint[], random: Random = Math.random): Star {
    const r = (min: number, max: number) => between(random, min, max)
    // Most stars are tiny; a few are bright enough to get a halo.
    const big = random() < 0.08
    return {
        x: random(),
        y: random(),
        size: big ? r(1.3, 2) : r(0.35, 1.1),
        brightness: big ? r(0.75, 1) : r(0.3, 0.8),
        rate: r(0.0006, 0.0024),
        phase: r(0, Math.PI * 2),
        tint: pickTint(tints, random),
    }
}

export function spawnMeteor(width: number, height: number, random: Random = Math.random): Meteor {
    const r = (min: number, max: number) => between(random, min, max)
    // From the upper part of the sky, falling left to right at 18–34°.
    const angle = r(0.32, 0.6)
    const speed = r(10, 15)
    return {
        x: r(width * 0.05, width * 0.7),
        y: r(height * 0.02, height * 0.45),
        vx: Math.cos(angle) * speed,
        vy: Math.sin(angle) * speed,
        length: r(90, 170),
        life: 0,
        maxLife: r(METEOR_LIFE_MS[0], METEOR_LIFE_MS[1]),
    }
}

/** How many stars a level shows. */
export function starCount(level: EffectLevel): number {
    return STAR_COUNTS[level]
}

/** The average wait between shooting stars at a level. */
export function meteorInterval(everyMs: number | null, level: EffectLevel): number | null {
    if (everyMs === null) { return null }
    return level === 'subtle' ? everyMs * SUBTLE_METEOR_FACTOR : everyMs
}

export class StarfieldScene implements EffectScene {
    /** The stars and the shooting stars in flight (exposed for tests). */
    readonly stars: Star[] = []
    readonly meteors: Meteor[] = []
    private count: number
    private readonly tints: readonly StarTint[]
    private meteorEveryMs: number | null
    private readonly driftX: number
    private readonly driftY: number
    private readonly random: Random
    private width = 0
    private height = 0
    private time = 0

    constructor(options: StarfieldOptions) {
        this.count = Math.max(0, Math.round(options.count))
        this.tints = options.tints
        this.meteorEveryMs = options.meteorEveryMs
        this.driftX = options.driftX ?? 0
        this.driftY = options.driftY ?? 0
        this.random = options.random ?? Math.random
        this.fill()
    }

    /** Changes how many stars there are; extra ones go at once, new ones appear at random places. */
    setCount(count: number): void {
        this.count = Math.max(0, Math.round(count))
        if (this.stars.length > this.count) {
            this.stars.length = this.count
        }
        this.fill()
    }

    /** The average time between shooting stars from now on (`null`: none). */
    setMeteorInterval(everyMs: number | null): void {
        this.meteorEveryMs = everyMs
    }

    resize(width: number, height: number): void {
        this.width = width
        this.height = height
    }

    step(dtMs: number, timeMs: number = this.time + dtMs): void {
        this.time = timeMs
        const k = frames60(dtMs)

        if ((this.driftX !== 0 || this.driftY !== 0) && this.width > 0 && this.height > 0) {
            const dx = (this.driftX * k) / this.width
            const dy = (this.driftY * k) / this.height
            for (const s of this.stars) {
                s.x = wrap(s.x + dx)
                s.y = wrap(s.y + dy)
            }
        }

        for (let i = this.meteors.length - 1; i >= 0; i--) {
            const m = this.meteors[i]
            m.life += dtMs / m.maxLife
            m.x += m.vx * k
            m.y += m.vy * k
            if (m.life >= 1) { this.meteors.splice(i, 1) }
        }

        // A shooting star is a Poisson event: the chance of one in `dtMs`
        // is dt / interval, whatever the frame rate.
        if (this.meteorEveryMs !== null && dtMs > 0 && this.random() < dtMs / this.meteorEveryMs) {
            this.meteors.push(spawnMeteor(this.width, this.height, this.random))
        }
    }

    draw(ctx: CanvasRenderingContext2D, width: number, height: number): void {
        for (const s of this.stars) {
            drawStar(ctx, s, width, height, this.time)
        }
        for (const m of this.meteors) {
            drawMeteor(ctx, m)
        }
    }

    private fill(): void {
        while (this.stars.length < this.count) {
            this.stars.push(spawnStar(this.tints, this.random))
        }
    }
}

function wrap(v: number): number {
    return v - Math.floor(v)
}

/** A star's brightness at `timeMs`: it never goes fully dark (0.35–1 of its peak). */
export function twinkle(star: Star, timeMs: number): number {
    return star.brightness * (0.675 + 0.325 * Math.sin(timeMs * star.rate + star.phase))
}

function drawStar(ctx: CanvasRenderingContext2D, s: Star, width: number, height: number, timeMs: number): void {
    const alpha = twinkle(s, timeMs)
    const x = s.x * width
    const y = s.y * height
    const { hue, saturation, lightness } = s.tint
    if (s.size > 1.2) {
        const halo = s.size * 6
        const g = ctx.createRadialGradient(x, y, 0, x, y, halo)
        g.addColorStop(0, `hsla(${hue},${saturation}%,${lightness}%,${alpha * 0.45})`)
        g.addColorStop(1, `hsla(${hue},${saturation}%,${lightness}%,0)`)
        ctx.beginPath()
        ctx.arc(x, y, halo, 0, Math.PI * 2)
        ctx.fillStyle = g
        ctx.fill()
    }
    ctx.beginPath()
    ctx.arc(x, y, s.size, 0, Math.PI * 2)
    ctx.fillStyle = `hsla(${hue},${saturation}%,${lightness}%,${alpha})`
    ctx.fill()
}

function drawMeteor(ctx: CanvasRenderingContext2D, m: Meteor): void {
    // Fades in fast, out slowly.
    const alpha = Math.min(1, m.life * 6) * (1 - m.life)
    const speed = Math.hypot(m.vx, m.vy) || 1
    const tailX = m.x - (m.vx / speed) * m.length
    const tailY = m.y - (m.vy / speed) * m.length
    const g = ctx.createLinearGradient(m.x, m.y, tailX, tailY)
    g.addColorStop(0, `rgba(255,248,240,${alpha})`)
    g.addColorStop(0.25, `rgba(255,220,200,${alpha * 0.45})`)
    g.addColorStop(1, 'rgba(255,220,200,0)')
    ctx.beginPath()
    ctx.moveTo(m.x, m.y)
    ctx.lineTo(tailX, tailY)
    ctx.strokeStyle = g
    ctx.lineWidth = 1.4
    ctx.lineCap = 'round'
    ctx.stroke()
}
