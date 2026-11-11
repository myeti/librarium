/**
 * A small FIFO queue of async tasks, keyed, with at most `concurrency`
 * running at once. Used to load covers lazily: one task per book id.
 *
 * Interest is counted per key: pushing a key that is already waiting adds
 * interest instead of a second task, and `cancel` removes one unit of
 * interest. A waiting task is dropped only when nobody wants it any more, so
 * a card scrolling out of view doesn't cancel a cover the detail panel is
 * also waiting for. Tasks that already started always run to completion.
 *
 * A task's rejection is swallowed (tasks handle their own errors); it never
 * stops the queue.
 */
export class TaskQueue {
    private readonly waiting = new Map<string, { run: () => Promise<void>, interest: number }>()
    private readonly runningKeys = new Set<string>()
    private idleWaiters: (() => void)[] = []

    constructor(private readonly concurrency: number) {}

    /** Number of tasks waiting to start. */
    get pending(): number { return this.waiting.size }

    /** Number of tasks running now. */
    get running(): number { return this.runningKeys.size }

    /** Whether a task for `key` is waiting or running. */
    has(key: string): boolean { return this.waiting.has(key) || this.runningKeys.has(key) }

    /**
     * Queues `run` under `key`. If a task for `key` is already waiting, adds
     * interest to it instead; if one is running, does nothing (its result is
     * on its way). Returns true when interest was registered, i.e. when a
     * matching `cancel(key)` is due if the caller loses interest.
     */
    push(key: string, run: () => Promise<void>): boolean {
        if (this.runningKeys.has(key)) { return false }
        const entry = this.waiting.get(key)
        if (entry) {
            entry.interest++
        } else {
            this.waiting.set(key, { run, interest: 1 })
            this.pump()
        }
        return true
    }

    /**
     * Withdraws one unit of interest in `key`'s waiting task; the task is
     * dropped when none is left. Returns true if it was dropped.
     */
    cancel(key: string): boolean {
        const entry = this.waiting.get(key)
        if (!entry) { return false }
        entry.interest--
        if (entry.interest > 0) { return false }
        this.waiting.delete(key)
        this.notifyIfIdle()
        return true
    }

    /** Drops every waiting task (running ones finish). */
    clear(): void {
        this.waiting.clear()
        this.notifyIfIdle()
    }

    /** Resolves once nothing is waiting or running (tests, shutdown). */
    whenIdle(): Promise<void> {
        if (this.waiting.size === 0 && this.runningKeys.size === 0) { return Promise.resolve() }
        return new Promise(resolve => { this.idleWaiters.push(resolve) })
    }

    private pump(): void {
        while (this.runningKeys.size < this.concurrency && this.waiting.size > 0) {
            // Maps iterate in insertion order: the oldest waiting task.
            const [key, entry] = this.waiting.entries().next().value as [string, { run: () => Promise<void> }]
            this.waiting.delete(key)
            this.runningKeys.add(key)
            void Promise.resolve()
                .then(entry.run)
                .catch(() => { /* tasks report their own errors */ })
                .finally(() => {
                    this.runningKeys.delete(key)
                    this.pump()
                    this.notifyIfIdle()
                })
        }
    }

    private notifyIfIdle(): void {
        if (this.waiting.size > 0 || this.runningKeys.size > 0) { return }
        const waiters = this.idleWaiters
        this.idleWaiters = []
        waiters.forEach(resolve => resolve())
    }
}
