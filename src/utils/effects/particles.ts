/**
 * The default background effect (MagicParticles): drifting dust, rising
 * embers and short-lived four-pointed sparkles, tinted with the theme's
 * accent hue. Pure simulation and drawing; `CanvasLoop` runs it.
 *
 * Speeds are in CSS pixels per 60 Hz frame (as they were tuned) and scaled
 * by the elapsed time; lifetimes are in milliseconds.
 */
import { between, frames60, type EffectScene, type Random } from './canvasLoop'
import type { EffectLevel } from './types'

export type ParticleKind = 'ember' | 'dust' | 'sparkle'

export interface Particle {
    kind: ParticleKind
    x: number
    y: number
    /** Velocity, in CSS pixels per 60 Hz frame. */
    vx: number
    vy: number
    /** Remaining life, from 1 (born) down to 0 (gone). */
    life: number
    /** Lifetime, in ms. */
    maxLife: number
    size: number
    hue: number
    /** Rotation of a sparkle (radians), and its speed per 60 Hz frame. */
    angle: number
    angleSpeed: number
}

/** How many particles each level keeps on screen. */
export const PARTICLE_COUNTS: Readonly<Record<EffectLevel, number>> = {
    subtle: 45,
    on: 100,
}

/** The hue used when the accent colour can't be read (Autumn's orange). */
export const FALLBACK_HUE = 25

/** Embers wander sideways, up to this speed. */
const EMBER_MAX_VX = 0.45

/** A new particle of a random kind (55 % dust, 30 % embers, 15 % sparkles). */
export function spawnParticle(baseHue: number, width: number, height: number, random: Random = Math.random): Particle {
    const r = (min: number, max: number) => between(random, min, max)
    const roll = random()
    if (roll < 0.55) {
        return {
            kind: 'dust',
            x: r(0, width), y: r(0, height),
            vx: r(-0.12, 0.12), vy: r(-0.18, 0.08),
            life: 1, maxLife: r(7000, 14000), size: r(0.7, 1.6),
            hue: baseHue + r(-8, 8), angle: 0, angleSpeed: 0,
        }
    }
    if (roll < 0.85) {
        return {
            kind: 'ember',
            x: r(0, width), y: height + 5,
            vx: r(-0.25, 0.25), vy: r(-0.35, -0.7),
            life: 1, maxLife: r(12000, 22000), size: r(0.9, 2.2),
            hue: baseHue + r(-12, 12), angle: 0, angleSpeed: 0,
        }
    }
    return {
        kind: 'sparkle',
        x: r(0, width), y: r(0, height * 0.75),
        vx: r(-0.04, 0.04), vy: r(-0.08, 0.08),
        life: 1, maxLife: r(900, 2200), size: r(1, 2.2),
        hue: baseHue + r(-10, 15), angle: r(0, Math.PI), angleSpeed: r(0.015, 0.06),
    }
}

export interface ParticlesOptions {
    /** How many particles to keep on screen (see `PARTICLE_COUNTS`). */
    count: number
    /** The base hue (degrees), usually the theme's accent: see `accentHue()`. */
    hue?: number
    random?: Random
}

export class ParticlesScene implements EffectScene {
    /** The live particles (exposed for tests). */
    readonly particles: Particle[] = []
    private count: number
    private hue: number
    private readonly random: Random
    private width = 0
    private height = 0

    constructor(options: ParticlesOptions) {
        this.count = Math.max(0, Math.round(options.count))
        this.hue = options.hue ?? FALLBACK_HUE
        this.random = options.random ?? Math.random
    }

    /** Changes how many particles are kept; extra ones go at once, missing ones come with the next step. */
    setCount(count: number): void {
        this.count = Math.max(0, Math.round(count))
        if (this.particles.length > this.count) {
            this.particles.length = this.count
        }
    }

    /** The base hue of the particles spawned from now on (the live ones keep theirs until they fade). */
    setHue(hue: number): void {
        this.hue = hue
    }

    resize(width: number, height: number): void {
        this.width = width
        this.height = height
    }

    step(dtMs: number): void {
        const k = frames60(dtMs)
        // The embers' sideways kicks are zero-mean noise: their spread grows
        // with √(number of steps), so √k keeps the same wander per second.
        const kick = Math.sqrt(k)
        const ps = this.particles
        for (let i = ps.length - 1; i >= 0; i--) {
            const p = ps[i]
            p.life -= dtMs / p.maxLife
            p.x += p.vx * k
            p.y += p.vy * k
            if (p.kind === 'ember') {
                p.vx += between(this.random, -0.015, 0.015) * kick
                p.vx = Math.max(-EMBER_MAX_VX, Math.min(EMBER_MAX_VX, p.vx))
            }
            p.angle += p.angleSpeed * k
            if (p.life <= 0) {
                ps.splice(i, 1)
            }
        }
        this.fill()
    }

    draw(ctx: CanvasRenderingContext2D): void {
        for (const p of this.particles) {
            drawParticle(ctx, p)
        }
    }

    /**
     * Tops the pool up (after each step, there are exactly `count`), each
     * newcomer at a random point of its life so they don't all fade together.
     */
    private fill(): void {
        while (this.particles.length < this.count) {
            const p = spawnParticle(this.hue, this.width, this.height, this.random)
            p.life = this.random()
            if (p.kind !== 'ember') {
                p.x = between(this.random, 0, this.width)
                p.y = between(this.random, 0, this.height)
            }
            this.particles.push(p)
        }
    }
}

function drawParticle(ctx: CanvasRenderingContext2D, p: Particle): void {
    const progress = 1 - p.life
    const fade = Math.sin(progress * Math.PI)

    if (p.kind === 'ember') {
        const alpha = fade * 0.42
        const g = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, p.size * 3)
        g.addColorStop(0,   `hsla(${p.hue}, 85%, 50%, ${alpha})`)
        g.addColorStop(0.4, `hsla(${p.hue}, 75%, 38%, ${alpha * 0.45})`)
        g.addColorStop(1,   `hsla(${p.hue}, 65%, 28%, 0)`)
        ctx.beginPath()
        ctx.arc(p.x, p.y, p.size * 3, 0, Math.PI * 2)
        ctx.fillStyle = g
        ctx.fill()
        return
    }

    if (p.kind === 'dust') {
        const alpha = fade * 0.35
        ctx.beginPath()
        ctx.arc(p.x, p.y, p.size, 0, Math.PI * 2)
        ctx.fillStyle = `hsla(${p.hue}, 60%, 45%, ${alpha})`
        ctx.fill()
        return
    }

    const alpha = fade * 0.65
    const s = p.size * (0.4 + 0.6 * fade)
    ctx.save()
    ctx.translate(p.x, p.y)
    ctx.rotate(p.angle)
    ctx.beginPath()
    for (let i = 0; i < 4; i++) {
        const a1 = (i / 4) * Math.PI * 2
        const a2 = a1 + Math.PI / 4
        ctx.lineTo(Math.cos(a1) * s * 3.2, Math.sin(a1) * s * 3.2)
        ctx.lineTo(Math.cos(a2) * s * 0.9, Math.sin(a2) * s * 0.9)
    }
    ctx.closePath()
    ctx.fillStyle = `hsla(${p.hue}, 80%, 45%, ${alpha * 0.75})`
    ctx.shadowColor = `hsla(${p.hue}, 80%, 40%, ${alpha * 0.3})`
    ctx.shadowBlur = 4
    ctx.fill()
    ctx.shadowBlur = 0
    ctx.restore()
}

// ── The accent hue ──────────────────────────────────────────────────────────

/** Hue (degrees, 0–359) of an RGB colour with channels in 0–1; 0 for greys. */
function rgbHue(r: number, g: number, b: number): number {
    const max = Math.max(r, g, b)
    const min = Math.min(r, g, b)
    const d = max - min
    if (d === 0) {
        return 0
    }
    let h: number
    if (max === r) {
        h = ((g - b) / d + 6) % 6
    } else if (max === g) {
        h = (b - r) / d + 2
    } else {
        h = (r - g) / d + 4
    }
    return Math.round(h * 60) % 360
}

function hexHue(hex: string): number | null {
    let digits = hex.slice(1)
    if (!/^([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(digits)) {
        return null
    }
    if (digits.length <= 4) {
        digits = digits.split('').map(c => c + c).join('')
    }
    const channel = (i: number) => parseInt(digits.slice(i, i + 2), 16) / 255
    return rgbHue(channel(0), channel(2), channel(4))
}

/** The arguments of a CSS function (`rgb(1 2 3 / 50%)`, `hsl(1, 2%, 3%)`…). */
function functionArgs(value: string, names: RegExp): string[] | null {
    const match = new RegExp(`^(?:${names.source})\\(([^)]*)\\)$`, 'i').exec(value)
    return match ? match[1].trim().split(/[\s,/]+/).filter(Boolean) : null
}

function rgbFunctionHue(value: string): number | null {
    const args = functionArgs(value, /rgba?/)
    if (!args || args.length < 3) {
        return null
    }
    const channels = args.slice(0, 3).map(arg =>
        arg.endsWith('%') ? parseFloat(arg) / 100 : parseFloat(arg) / 255)
    if (channels.some(c => !Number.isFinite(c))) {
        return null
    }
    return rgbHue(channels[0], channels[1], channels[2])
}

function hslFunctionHue(value: string): number | null {
    const args = functionArgs(value, /hsla?/)
    if (!args || args.length < 1) {
        return null
    }
    const match = /^(-?[\d.]+)(deg|turn|rad|grad)?$/i.exec(args[0])
    if (!match) {
        return null
    }
    const n = parseFloat(match[1])
    const unit = (match[2] ?? 'deg').toLowerCase()
    const degrees = unit === 'turn' ? n * 360 : unit === 'rad' ? n * 180 / Math.PI : unit === 'grad' ? n * 0.9 : n
    if (!Number.isFinite(degrees)) {
        return null
    }
    return Math.round(((degrees % 360) + 360) % 360) % 360
}

/**
 * The hue (degrees) of a CSS colour value, such as the computed `--accent`:
 * hex (`#rgb`, `#rrggbb`, with or without alpha), `rgb()`/`rgba()` and
 * `hsl()`/`hsla()`. Anything else (a name, `color-mix()`, an empty value)
 * gives `fallback`.
 */
export function accentHue(cssColor: string, fallback: number = FALLBACK_HUE): number {
    const value = cssColor.trim()
    const hue = value.startsWith('#') ? hexHue(value) : rgbFunctionHue(value) ?? hslFunctionHue(value)
    return hue ?? fallback
}
