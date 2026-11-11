import { describe, expect, it } from 'vitest'
import { TaskQueue } from './taskQueue'

/** A task the test finishes by hand, recording when it started. */
function manualTask(started: string[], key: string) {
    let finish!: () => void
    const done = new Promise<void>(r => { finish = r })
    return { run: async () => { started.push(key); await done }, finish: () => finish() }
}

/** Lets queued microtasks (task starts, `finally` handlers) run. */
const tick = () => new Promise(r => setTimeout(r, 0))

describe('TaskQueue', () => {
    it('runs at most `concurrency` tasks at once, oldest first', async () => {
        const queue = new TaskQueue(2)
        const started: string[] = []
        const tasks = ['a', 'b', 'c', 'd'].map(k => ({ key: k, ...manualTask(started, k) }))
        tasks.forEach(t => queue.push(t.key, t.run))
        await tick()
        expect(started).toEqual(['a', 'b'])
        expect([queue.running, queue.pending]).toEqual([2, 2])

        tasks[1].finish()
        await tick()
        expect(started).toEqual(['a', 'b', 'c'])

        tasks[0].finish(); tasks[2].finish()
        await tick()
        tasks[3].finish()
        await queue.whenIdle()
        expect(started).toEqual(['a', 'b', 'c', 'd'])
    })

    it('counts interest: a task is dropped only when everyone cancelled', async () => {
        const queue = new TaskQueue(1)
        const started: string[] = []
        const blocker = manualTask(started, 'blocker')
        queue.push('blocker', blocker.run)
        const wanted = manualTask(started, 'x')
        expect(queue.push('x', wanted.run)).toBe(true)
        expect(queue.push('x', wanted.run)).toBe(true) // card + detail panel

        expect(queue.cancel('x')).toBe(false) // the card scrolled away…
        expect(queue.has('x')).toBe(true) // …the panel still wants it
        expect(queue.cancel('x')).toBe(true)
        expect(queue.has('x')).toBe(false)

        blocker.finish()
        await queue.whenIdle()
        expect(started).toEqual(['blocker'])
    })

    it('does not queue a key that is already running', async () => {
        const queue = new TaskQueue(1)
        const started: string[] = []
        const task = manualTask(started, 'a')
        queue.push('a', task.run)
        await tick()
        expect(queue.push('a', task.run)).toBe(false)
        expect(queue.cancel('a')).toBe(false) // running tasks can't be cancelled
        task.finish()
        await queue.whenIdle()
        expect(started).toEqual(['a'])
    })

    it('clear drops waiting tasks; a failing task does not stop the queue', async () => {
        const queue = new TaskQueue(1)
        const started: string[] = []
        queue.push('boom', () => { started.push('boom'); return Promise.reject(new Error('bad book')) })
        queue.push('next', () => { started.push('next'); return Promise.resolve() })
        await queue.whenIdle()
        expect(started).toEqual(['boom', 'next'])

        const blocker = manualTask(started, 'blocker')
        queue.push('blocker', blocker.run)
        queue.push('dropped', () => { started.push('dropped'); return Promise.resolve() })
        queue.clear()
        blocker.finish()
        await queue.whenIdle()
        expect(started).toEqual(['boom', 'next', 'blocker'])
    })
})
