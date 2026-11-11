/**
 * A debounced function: calls `fn` once, `waitMs` after the last call, with
 * the arguments of that last call. Used for search fields, so filtering a
 * large library runs once the user pauses typing instead of on every key.
 *
 * - `cancel()` drops a pending call (a component being unmounted, a value
 *   reset from elsewhere);
 * - `flush()` runs a pending call now (Enter in a search field);
 * - `pending` tells whether a call is waiting.
 */
export interface Debounced<Args extends unknown[]> {
    (...args: Args): void
    cancel(): void
    flush(): void
    readonly pending: boolean
}

export function debounce<Args extends unknown[]>(fn: (...args: Args) => void, waitMs: number): Debounced<Args> {
    let timer: ReturnType<typeof setTimeout> | null = null
    let lastArgs: Args | null = null

    const run = () => {
        timer = null
        const args = lastArgs
        lastArgs = null
        if (args) { fn(...args) }
    }

    const debounced = (...args: Args) => {
        lastArgs = args
        if (timer !== null) { clearTimeout(timer) }
        timer = setTimeout(run, waitMs)
    }

    return Object.defineProperties(debounced, {
        cancel: {
            value: () => {
                if (timer !== null) { clearTimeout(timer) }
                timer = null
                lastArgs = null
            },
        },
        flush: {
            value: () => {
                if (timer === null) { return }
                clearTimeout(timer)
                run()
            },
        },
        pending: { get: () => timer !== null },
    }) as Debounced<Args>
}
