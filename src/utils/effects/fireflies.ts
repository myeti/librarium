/**
 * Fireflies (Folklore night: lantern embers): glowing dots wandering slowly,
 * kept away from the window's edges, fading in and out. Pure simulation and
 * drawing; `CanvasLoop` runs it.
 *
 * Speeds are in CSS pixels per 60 Hz frame (as they were tuned) and scaled
 * by the elapsed time; lifetimes are in milliseconds.
 */
import { between, frames60, type EffectScene, type Random } from './canvasLoop'
import type { EffectLevel } from './types'

export interface Firefly {
    x: number
    y: number
    /** Velocity, in CSS pixels per 60 Hz frame. */
    vx: number
    vy: number
    /** Age, from 0 (born) up to 1 (gone). */
    life: number
    /** Lifetime, in ms. */
    maxLife: number
    size: number
    hue: number
}

/** The share of fireflies the subtle level keeps. */
export const SUBTLE_SHARE = 0.5

/** How many fireflies to show for a `count` at full level. */
export function fireflyCount(count: number, level: EffectLevel): number {
    const n = Math.max(0, count)
    return Math.round(level === 'subtle' ? n * SUBTLE_SHARE : n)
}

const MAX_VX = 0.5
const MAX_VY = 0.4
/** Within this distance of an edge (CSS px), a firefly is nudged back in. */
const EDGE = 40

export function spawnFirefly(
    width: number, height: number, hueMin: number, hueMax: number, random: Random = Math.random,
): Firefly {
    const r = (min: number, max: number) => between(random, min, max)
    return {
        x:       r(width * 0.05, width * 0.95),
        y:       r(height * 0.08, height * 0.92),
        vx:      r(-0.25, 0.25),
        vy:      r(-0.20, 0.20),
        life:    0,
        maxLife: r(5000, 12000),
        size:    r(2.5, 4.5),
        hue:     r(hueMin, hueMax),
    }
}

export interface FirefliesOptions {
    /** How many fireflies to keep on screen (see `fireflyCount()`). */
    count: number
    hueMin: number
    hueMax: number
    random?: Random
}

export class FirefliesScene implements EffectScene {
    /** The live fireflies (exposed for tests). */
    readonly fireflies: Firefly[] = []
    private count: number
    private hueMin: number
    private hueMax: number
    private readonly random: Random
    private width = 0
    private height = 0

    constructor(options: FirefliesOptions) {
        this.count = Math.max(0, Math.round(options.count))
        this.hueMin = options.hueMin
        this.hueMax = options.hueMax
        this.random = options.random ?? Math.random
    }

    /** Changes how many fireflies are kept; extra ones go at once, missing ones come with the next step. */
    setCount(count: number): void {
        this.count = Math.max(0, Math.round(count))
        if (this.fireflies.length > this.count) {
            this.fireflies.length = this.count
        }
    }

    /** The hue range of the fireflies born from now on. */
    setHueRange(hueMin: number, hueMax: number): void {
        this.hueMin = hueMin
        this.hueMax = hueMax
    }

    resize(width: number, height: number): void {
        this.width = width
        this.height = height
    }

    step(dtMs: number): void {
        const k = frames60(dtMs)
        // Random kicks are zero-mean noise: their spread grows with
        // √(number of steps), so √k keeps the same wander per second.
        const kick = Math.sqrt(k)
        const { width, height } = this
        const fs = this.fireflies
        for (let i = fs.length - 1; i >= 0; i--) {
            const f = fs[i]
            f.life += dtMs / f.maxLife
            f.x += f.vx * k
            f.y += f.vy * k

            f.vx += between(this.random, -0.007, 0.007) * kick
            f.vy += between(this.random, -0.005, 0.005) * kick
            f.vx = Math.max(-MAX_VX, Math.min(MAX_VX, f.vx))
            f.vy = Math.max(-MAX_VY, Math.min(MAX_VY, f.vy))

            if (f.x < EDGE) {
                f.vx += 0.014 * k
            }
            if (f.x > width - EDGE) {
                f.vx -= 0.014 * k
            }
            if (f.y < EDGE) {
                f.vy += 0.010 * k
            }
            if (f.y > height - EDGE) {
                f.vy -= 0.010 * k
            }

            if (f.life >= 1) {
                fs.splice(i, 1)
            }
        }
        this.fill()
    }

    draw(ctx: CanvasRenderingContext2D): void {
        for (const f of this.fireflies) {
            drawFirefly(ctx, f)
        }
    }

    /** Tops the swarm up (after each step, there are exactly `count`), each newcomer at a random point of its life. */
    private fill(): void {
        while (this.fireflies.length < this.count) {
            const f = spawnFirefly(this.width, this.height, this.hueMin, this.hueMax, this.random)
            f.life = this.random()
            this.fireflies.push(f)
        }
    }
}

function drawFirefly(ctx: CanvasRenderingContext2D, f: Firefly): void {
    const alpha = Math.sin(f.life * Math.PI)
    const bloom = f.size * 9

    const g = ctx.createRadialGradient(f.x, f.y, 0, f.x, f.y, bloom)
    g.addColorStop(0,    `hsla(${f.hue},78%,68%,${alpha * 0.88})`)
    g.addColorStop(0.20, `hsla(${f.hue},70%,55%,${alpha * 0.38})`)
    g.addColorStop(0.55, `hsla(${f.hue},62%,42%,${alpha * 0.10})`)
    g.addColorStop(1,    `hsla(${f.hue},60%,38%,0)`)
    ctx.beginPath()
    ctx.arc(f.x, f.y, bloom, 0, Math.PI * 2)
    ctx.fillStyle = g
    ctx.fill()

    ctx.beginPath()
    ctx.arc(f.x, f.y, f.size * 0.42, 0, Math.PI * 2)
    ctx.fillStyle = `hsla(${f.hue},90%,90%,${alpha})`
    ctx.fill()
}
