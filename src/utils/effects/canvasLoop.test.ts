import { describe, expect, it, vi } from 'vitest'
import {
    CanvasLoop,
    FRAME_SLACK_MS,
    MAX_DT_MS,
    frames60,
    startCanvasLoop,
    type CanvasLoopOptions,
    type EffectScene,
    type LoopCanvas,
} from './canvasLoop'

// ── Fakes ───────────────────────────────────────────────────────────────────

/** requestAnimationFrame on a fake clock: `tick(ms)` advances it and runs the frames requested so far. */
class FakeFrames {
    time = 0
    private nextId = 1
    private readonly callbacks = new Map<number, (timeMs: number) => void>()

    readonly request = (callback: (timeMs: number) => void): number => {
        const id = this.nextId++
        this.callbacks.set(id, callback)
        return id
    }

    readonly cancel = (id: number): void => {
        this.callbacks.delete(id)
    }

    get pending(): number {
        return this.callbacks.size
    }

    tick(ms: number): void {
        this.time += ms
        const due = [...this.callbacks.values()]
        this.callbacks.clear()
        for (const callback of due) {
            callback(this.time)
        }
    }

    /** Display frames at `hz` for `seconds`. */
    run(hz: number, seconds: number): void {
        const count = Math.round(hz * seconds)
        for (let i = 0; i < count; i++) {
            this.tick(1000 / hz)
        }
    }
}

class FakeTarget {
    readonly listeners = new Map<string, Set<() => void>>()

    addEventListener(type: string, listener: () => void): void {
        const set = this.listeners.get(type) ?? new Set()
        set.add(listener)
        this.listeners.set(type, set)
    }

    removeEventListener(type: string, listener: () => void): void {
        this.listeners.get(type)?.delete(listener)
    }

    emit(type: string): void {
        for (const listener of [...(this.listeners.get(type) ?? [])]) {
            listener()
        }
    }

    get listenerCount(): number {
        let n = 0
        for (const set of this.listeners.values()) {
            n += set.size
        }
        return n
    }
}

class FakeDocument extends FakeTarget {
    hidden = false

    setHidden(hidden: boolean): void {
        this.hidden = hidden
        this.emit('visibilitychange')
    }
}

class FakeWindow extends FakeTarget {
    innerWidth = 800
    innerHeight = 600
    devicePixelRatio = 1
}

/** Records what the loop asks of it; `x` moves 0.5 CSS px per 60 Hz frame. */
class RecordingScene implements EffectScene {
    readonly steps: number[] = []
    readonly times: number[] = []
    readonly sizes: [number, number][] = []
    draws = 0
    x = 0

    resize(width: number, height: number): void {
        this.sizes.push([width, height])
    }

    step(dtMs: number, timeMs: number): void {
        this.steps.push(dtMs)
        this.times.push(timeMs)
        this.x += 0.5 * frames60(dtMs)
    }

    draw(): void {
        this.draws++
    }
}

function last<T>(items: readonly T[]): T | undefined {
    return items[items.length - 1]
}

function fakeContext() {
    return { setTransform: vi.fn(), clearRect: vi.fn() }
}

function setup(options: CanvasLoopOptions = {}) {
    const frames = new FakeFrames()
    const doc = new FakeDocument()
    const win = new FakeWindow()
    const canvas: LoopCanvas = { width: 300, height: 150 }
    const ctx = fakeContext()
    const scene = new RecordingScene()
    const loop = new CanvasLoop(canvas, ctx as unknown as CanvasRenderingContext2D, scene, {
        requestFrame: frames.request,
        cancelFrame: frames.cancel,
        document: doc,
        window: win,
        ...options,
    })
    return { frames, doc, win, canvas, ctx, scene, loop }
}

// ── Tests ───────────────────────────────────────────────────────────────────

describe('CanvasLoop timing', () => {
    it('steps by the time elapsed since the last frame, starting with a 0 ms step', () => {
        const { frames, scene, loop } = setup()
        loop.start()
        frames.run(60, 0.1)
        expect(scene.steps[0]).toBe(0)
        for (const dt of scene.steps.slice(1)) {
            expect(dt).toBeCloseTo(1000 / 60, 6)
        }
        expect(scene.draws).toBe(scene.steps.length)
        // The scene's clock is the sum of the steps.
        expect(last(scene.times)).toBeCloseTo(scene.steps.reduce((a, b) => a + b, 0), 6)
    })

    it('moves things as far in a second at 60 Hz, at 120 Hz, and capped at 30 fps', () => {
        const at60 = setup()
        const at120 = setup()
        const capped = setup({ maxFps: 30 })
        for (const [run, hz] of [[at60, 60], [at120, 120], [capped, 120]] as const) {
            run.loop.start()
            run.frames.tick(1000 / hz) // first frame: no motion
            run.frames.run(hz, 1)
        }
        expect(at60.scene.x).toBeCloseTo(30, 6)
        expect(at120.scene.x).toBeCloseTo(30, 6)
        expect(capped.scene.x).toBeCloseTo(30, 6)
        expect(at60.scene.draws).toBe(61)
        expect(at120.scene.draws).toBe(121)
        // The cap draws one display frame in four at 120 Hz.
        expect(capped.scene.draws).toBe(31)
        for (const dt of capped.scene.steps.slice(1)) {
            expect(dt).toBeCloseTo(1000 / 30, 6)
        }
    })

    it('caps at 30 fps on a 60 Hz display too, despite a little timing jitter', () => {
        const { frames, scene, loop } = setup({ maxFps: 30 })
        loop.start()
        frames.tick(16)
        for (let i = 0; i < 30; i++) {
            frames.tick(17)
            frames.tick(1000 / 30 - 17 - FRAME_SLACK_MS / 2) // a frame that is a bit early still counts
        }
        expect(scene.draws).toBe(31)
    })

    it('can change the cap while running', () => {
        const { frames, scene, loop } = setup()
        loop.start()
        frames.run(120, 0.5)
        expect(scene.draws).toBe(60)
        loop.setMaxFps(30)
        frames.run(120, 0.5)
        expect(scene.draws).toBe(75)
        loop.setMaxFps(null)
        frames.run(120, 0.5)
        expect(scene.draws).toBe(135)
    })

    it('clamps the step after a long gap, so nothing teleports', () => {
        const { frames, scene, loop } = setup()
        loop.start()
        frames.run(60, 0.1)
        frames.tick(5000)
        expect(last(scene.steps)).toBe(MAX_DT_MS)
        frames.tick(1000 / 60)
        expect(last(scene.steps)).toBeCloseTo(1000 / 60, 6)
    })
})

describe('CanvasLoop visibility', () => {
    it('requests no frame while the window is hidden, and resumes without a jump', () => {
        const { frames, doc, scene, loop } = setup()
        loop.start()
        frames.run(60, 0.5)
        const clockBefore = last(scene.times)!
        const stepsBefore = scene.steps.length

        doc.setHidden(true)
        expect(frames.pending).toBe(0)
        frames.tick(60_000)
        expect(scene.steps.length).toBe(stepsBefore)

        doc.setHidden(false)
        expect(frames.pending).toBe(1)
        frames.tick(1000 / 60)
        expect(last(scene.steps)).toBe(0)
        expect(last(scene.times)).toBe(clockBefore)
        frames.tick(1000 / 60)
        expect(last(scene.steps)).toBeCloseTo(1000 / 60, 6)
    })

    it('waits for the window to be shown when started while hidden', () => {
        const { frames, doc, scene, loop } = setup()
        doc.hidden = true
        loop.start()
        expect(frames.pending).toBe(0)
        doc.setHidden(false)
        frames.tick(16)
        expect(scene.draws).toBe(1)
    })

    it('does not schedule a second frame when shown twice', () => {
        const { frames, doc, loop } = setup()
        loop.start()
        doc.setHidden(false)
        doc.setHidden(false)
        expect(frames.pending).toBe(1)
    })
})

describe('CanvasLoop lifecycle', () => {
    it('stop() pauses and start() resumes from a 0 ms step', () => {
        const { frames, scene, loop } = setup()
        loop.start()
        frames.run(60, 0.1)
        loop.stop()
        expect(loop.isRunning).toBe(false)
        expect(frames.pending).toBe(0)
        frames.tick(3000)
        loop.start()
        frames.tick(16)
        expect(last(scene.steps)).toBe(0)
    })

    it('destroy() removes every listener, cancels the frame and ignores later starts', () => {
        const { frames, doc, win, loop } = setup()
        loop.start()
        expect(doc.listenerCount).toBe(1)
        expect(win.listenerCount).toBe(1)
        expect(frames.pending).toBe(1)
        loop.destroy()
        expect(doc.listenerCount).toBe(0)
        expect(win.listenerCount).toBe(0)
        expect(frames.pending).toBe(0)
        loop.start()
        expect(frames.pending).toBe(0)
        expect(doc.listenerCount).toBe(0)
    })

    it('stops when the scene throws, instead of failing on every frame', () => {
        const { frames, scene, loop } = setup()
        scene.step = () => { throw new Error('boom') }
        loop.start()
        expect(() => frames.tick(16)).toThrow('boom')
        expect(frames.pending).toBe(0)
    })
})

describe('CanvasLoop sizing', () => {
    it('sizes the backing store to the CSS size × devicePixelRatio and draws in CSS pixels', () => {
        const { frames, win, canvas, ctx, scene, loop } = setup()
        win.devicePixelRatio = 2
        loop.start()
        expect(canvas.width).toBe(1600)
        expect(canvas.height).toBe(1200)
        expect(scene.sizes).toEqual([[800, 600]])
        expect(loop.size).toEqual({ width: 800, height: 600, pixelRatio: 2 })
        frames.tick(16)
        expect(ctx.setTransform).toHaveBeenLastCalledWith(2, 0, 0, 2, 0, 0)
        expect(ctx.clearRect).toHaveBeenLastCalledWith(0, 0, 800, 600)
    })

    it('re-measures on resize and redraws at once', () => {
        const { frames, win, canvas, scene, loop } = setup()
        win.devicePixelRatio = 2
        loop.start()
        frames.tick(16)
        const draws = scene.draws
        win.innerWidth = 1000
        win.innerHeight = 700
        win.emit('resize')
        expect(canvas.width).toBe(2000)
        expect(canvas.height).toBe(1400)
        expect(last(scene.sizes)).toEqual([1000, 700])
        expect(scene.draws).toBe(draws + 1)
    })

    it('follows a change of devicePixelRatio (window moved to another screen)', () => {
        const { frames, win, canvas, ctx, loop } = setup()
        win.devicePixelRatio = 2
        loop.start()
        frames.tick(16)
        win.devicePixelRatio = 1
        frames.tick(16)
        expect(canvas.width).toBe(800)
        expect(ctx.setTransform).toHaveBeenLastCalledWith(1, 0, 0, 1, 0, 0)
    })

    it('rounds fractional ratios, caps very high ones and ignores nonsense', () => {
        const { win, canvas, loop } = setup()
        win.devicePixelRatio = 1.25
        win.innerWidth = 801
        loop.start()
        expect(canvas.width).toBe(1001)
        win.devicePixelRatio = 3
        win.emit('resize')
        expect(canvas.width).toBe(1602)
        expect(loop.size.pixelRatio).toBe(2)
        win.devicePixelRatio = 0
        win.emit('resize')
        expect(canvas.width).toBe(801)
    })
})

describe('startCanvasLoop', () => {
    it('does nothing without a 2D context', () => {
        const canvas = document.createElement('canvas')
        vi.spyOn(canvas, 'getContext').mockReturnValue(null)
        const scene = new RecordingScene()
        const requestFrame = vi.fn(() => 1)
        expect(startCanvasLoop(canvas, scene, { requestFrame })).toBeNull()
        expect(requestFrame).not.toHaveBeenCalled()
        expect(scene.sizes).toEqual([])
    })

    it('does nothing when getContext throws', () => {
        const canvas = document.createElement('canvas')
        vi.spyOn(canvas, 'getContext').mockImplementation(() => { throw new Error('no canvas') })
        expect(startCanvasLoop(canvas, new RecordingScene())).toBeNull()
    })

    it('starts a loop on the 2D context', () => {
        const canvas = document.createElement('canvas')
        const ctx = fakeContext()
        vi.spyOn(canvas, 'getContext').mockReturnValue(ctx as unknown as CanvasRenderingContext2D)
        const frames = new FakeFrames()
        const scene = new RecordingScene()
        const loop = startCanvasLoop(canvas, scene, {
            requestFrame: frames.request,
            cancelFrame: frames.cancel,
            document: new FakeDocument(),
            window: new FakeWindow(),
        })
        expect(loop?.isRunning).toBe(true)
        frames.tick(16)
        expect(scene.draws).toBe(1)
        loop?.destroy()
    })
})
