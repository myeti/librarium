/**
 * The animation loop behind the canvas background effects, without Vue.
 *
 * A `CanvasLoop` drives one `EffectScene` (the simulation and its drawing,
 * see particles.ts, fireflies.ts, starfield.ts) on a full-window canvas:
 *
 * - **time-based:** each step gets the milliseconds elapsed since the last
 *   drawn frame, so the effect moves at the same speed at 60, 120 or 30
 *   frames a second. The step is clamped (`maxDtMs`, 100 ms) so a stall
 *   doesn't teleport the particles;
 * - **frame-rate cap:** with `maxFps` set, display frames that come sooner
 *   than 1/maxFps after the last drawn one are skipped (the "subtle" level
 *   draws at 30 fps);
 * - **pauses while the window is hidden** (`visibilitychange`): no frame is
 *   requested at all, and the first frame after the window comes back steps
 *   by 0 ms instead of by the whole pause;
 * - **sharp on HiDPI screens:** the backing store is the window's CSS size ×
 *   devicePixelRatio (re-measured on resize, and when the ratio changes, e.g.
 *   the window moved to another screen), and the context is scaled so scenes
 *   draw in CSS pixels.
 *
 * Its dependencies (frame scheduling, document, window) default to the
 * globals and can be replaced by fakes in tests.
 */

/** One full-window canvas effect: a simulation and how to draw it. */
export interface EffectScene {
    /** The canvas now covers `width` × `height` CSS pixels. Called on start and on every resize. */
    resize(width: number, height: number): void
    /**
     * Advances the simulation by `dtMs` milliseconds: 0 on the first frame
     * after a start or a resume, at most `maxDtMs`. `timeMs` is the effect's
     * own clock (the sum of the steps): it doesn't jump over a pause.
     */
    step(dtMs: number, timeMs: number): void
    /** Draws the current state. The canvas is cleared and scaled so one unit is one CSS pixel. */
    draw(ctx: CanvasRenderingContext2D, width: number, height: number): void
}

/** What the loop uses of `document`. */
export interface LoopDocument {
    readonly hidden: boolean
    addEventListener(type: 'visibilitychange', listener: () => void): void
    removeEventListener(type: 'visibilitychange', listener: () => void): void
}

/** What the loop uses of `window`. */
export interface LoopWindow {
    readonly innerWidth: number
    readonly innerHeight: number
    readonly devicePixelRatio: number
    addEventListener(type: 'resize', listener: () => void): void
    removeEventListener(type: 'resize', listener: () => void): void
}

/** What the loop uses of the canvas element: its backing-store size. */
export interface LoopCanvas {
    width: number
    height: number
}

export interface CanvasLoopOptions {
    /** Draw at most this many frames a second; `null` (default): every display frame. */
    maxFps?: number | null
    /** The longest step, in ms (default `MAX_DT_MS`). */
    maxDtMs?: number
    /** The highest pixel ratio honoured (default `MAX_PIXEL_RATIO`). */
    maxPixelRatio?: number
    /** Defaults to `requestAnimationFrame`. */
    requestFrame?: (callback: (timeMs: number) => void) => number
    /** Defaults to `cancelAnimationFrame`. */
    cancelFrame?: (id: number) => void
    /** Defaults to `document`. */
    document?: LoopDocument
    /** Defaults to `window`. */
    window?: LoopWindow
}

/** The longest step: after a stall the effect resumes instead of jumping ahead. */
export const MAX_DT_MS = 100

/**
 * The highest device pixel ratio the backing store follows. The canvas covers
 * the whole window and its cost grows with the square of the ratio; the
 * effects are soft glows that gain nothing past Retina's 2.
 */
export const MAX_PIXEL_RATIO = 2

/**
 * Frames are timed on vsync, with a little jitter: a frame that is this much
 * early still counts as on time for the cap (otherwise a 30 fps cap at 60 Hz
 * would sometimes wait for a third frame).
 */
export const FRAME_SLACK_MS = 2

export class CanvasLoop {
    private readonly canvas: LoopCanvas
    private readonly ctx: CanvasRenderingContext2D
    private readonly scene: EffectScene
    private readonly requestFrame: (callback: (timeMs: number) => void) => number
    private readonly cancelFrame: (id: number) => void
    private readonly doc: LoopDocument
    private readonly win: LoopWindow
    private readonly maxDtMs: number
    private readonly maxPixelRatio: number

    /** Display frames closer than this to the last drawn one are skipped (0: none). */
    private minFrameMs = 0
    private frameId: number | null = null
    /** Timestamp of the last drawn frame; null until the first frame after a (re)start. */
    private lastTime: number | null = null
    /** The scene's clock: the sum of the steps. */
    private clock = 0
    private running = false
    private destroyed = false
    /** The canvas size in CSS pixels and the pixel ratio of its backing store. */
    private width = 0
    private height = 0
    private pixelRatio = 1

    constructor(
        canvas: LoopCanvas,
        ctx: CanvasRenderingContext2D,
        scene: EffectScene,
        options: CanvasLoopOptions = {},
    ) {
        this.canvas = canvas
        this.ctx = ctx
        this.scene = scene
        this.requestFrame = options.requestFrame ?? (callback => requestAnimationFrame(callback))
        this.cancelFrame = options.cancelFrame ?? (id => cancelAnimationFrame(id))
        this.doc = options.document ?? document
        this.win = options.window ?? window
        this.maxDtMs = options.maxDtMs ?? MAX_DT_MS
        this.maxPixelRatio = options.maxPixelRatio ?? MAX_PIXEL_RATIO
        this.setMaxFps(options.maxFps ?? null)
    }

    /** Whether the loop is started (it may still be waiting for the window to be shown). */
    get isRunning(): boolean {
        return this.running
    }

    /** The canvas size in CSS pixels and the pixel ratio of its backing store (as of the last resize). */
    get size(): { width: number, height: number, pixelRatio: number } {
        return { width: this.width, height: this.height, pixelRatio: this.pixelRatio }
    }

    /** Sizes the canvas, listens to resize and visibility changes, and starts drawing. */
    start(): void {
        if (this.running || this.destroyed) {
            return
        }
        this.running = true
        this.win.addEventListener('resize', this.onResize)
        this.doc.addEventListener('visibilitychange', this.onVisibilityChange)
        this.resize()
        this.lastTime = null
        this.schedule()
    }

    /** Stops drawing and listening; the canvas keeps its last frame. `start()` resumes. */
    stop(): void {
        if (!this.running) {
            return
        }
        this.running = false
        this.win.removeEventListener('resize', this.onResize)
        this.doc.removeEventListener('visibilitychange', this.onVisibilityChange)
        this.cancel()
    }

    /** Stops for good: removes every listener, cancels the pending frame, ignores later `start()`s. */
    destroy(): void {
        this.stop()
        this.destroyed = true
    }

    /** Caps the frame rate (`null` or ≤ 0: none). Motion speed doesn't change. */
    setMaxFps(fps: number | null): void {
        this.minFrameMs = fps !== null && fps > 0 ? Math.max(0, 1000 / fps - FRAME_SLACK_MS) : 0
    }

    private schedule(): void {
        if (this.frameId === null && this.running && !this.doc.hidden) {
            this.frameId = this.requestFrame(this.onFrame)
        }
    }

    private cancel(): void {
        if (this.frameId !== null) {
            this.cancelFrame(this.frameId)
            this.frameId = null
        }
    }

    private readonly onFrame = (timeMs: number): void => {
        this.frameId = null
        if (!this.running || this.doc.hidden) {
            return
        }
        if (this.currentPixelRatio() !== this.pixelRatio) {
            this.resize()
        }
        if (this.lastTime === null) {
            this.lastTime = timeMs
            this.render(0)
        } else {
            const elapsed = timeMs - this.lastTime
            if (elapsed >= this.minFrameMs) {
                this.lastTime = timeMs
                this.render(Math.min(Math.max(elapsed, 0), this.maxDtMs))
            }
        }
        // Scheduled after the work: a scene that throws stops its loop instead
        // of throwing on every frame.
        this.schedule()
    }

    private readonly onResize = (): void => {
        this.resize()
        // Resizing the backing store cleared it: redraw now rather than on the next drawn frame.
        if (this.running && !this.doc.hidden && this.lastTime !== null) {
            this.paint()
        }
    }

    private readonly onVisibilityChange = (): void => {
        if (this.doc.hidden) {
            this.cancel()
        } else if (this.running && this.frameId === null) {
            // The first frame back steps by 0 ms: no jump over the time spent hidden.
            this.lastTime = null
            this.schedule()
        }
    }

    private currentPixelRatio(): number {
        const ratio = this.win.devicePixelRatio
        return Number.isFinite(ratio) && ratio > 0 ? Math.min(ratio, this.maxPixelRatio) : 1
    }

    private resize(): void {
        const width = Math.max(0, this.win.innerWidth)
        const height = Math.max(0, this.win.innerHeight)
        const ratio = this.currentPixelRatio()
        this.canvas.width = Math.max(1, Math.round(width * ratio))
        this.canvas.height = Math.max(1, Math.round(height * ratio))
        this.width = width
        this.height = height
        this.pixelRatio = ratio
        this.scene.resize(width, height)
    }

    private render(dtMs: number): void {
        this.clock += dtMs
        this.scene.step(dtMs, this.clock)
        this.paint()
    }

    private paint(): void {
        // Setting the backing-store size resets the transform: set it every time.
        this.ctx.setTransform(this.pixelRatio, 0, 0, this.pixelRatio, 0, 0)
        this.ctx.clearRect(0, 0, this.width, this.height)
        this.scene.draw(this.ctx, this.width, this.height)
    }
}

/**
 * Creates and starts a loop for `scene` on `canvas`. Returns null, and does
 * nothing, where the canvas has no 2D context (e.g. happy-dom in tests).
 */
export function startCanvasLoop(
    canvas: HTMLCanvasElement,
    scene: EffectScene,
    options: CanvasLoopOptions = {},
): CanvasLoop | null {
    let ctx: CanvasRenderingContext2D | null = null
    try {
        ctx = canvas.getContext('2d')
    } catch {
        ctx = null
    }
    if (!ctx) {
        return null
    }
    const loop = new CanvasLoop(canvas, ctx, scene, options)
    loop.start()
    return loop
}

// ── Helpers for scenes ──────────────────────────────────────────────────────

/** A source of numbers in [0, 1), like `Math.random` (scenes take one so tests can script it). */
export type Random = () => number

/** A number in [min, max) (or (max, min] when max < min). */
export function between(random: Random, min: number, max: number): number {
    return random() * (max - min) + min
}

/**
 * How many 60 Hz frames `dtMs` stands for. The effects' speeds were tuned per
 * frame at 60 Hz: multiplying velocities by this keeps that look at any frame
 * rate.
 */
export function frames60(dtMs: number): number {
    return dtMs / (1000 / 60)
}
