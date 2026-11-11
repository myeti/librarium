import { vi } from 'vitest'

/**
 * Test helper: a controllable `window.matchMedia`. `set(query, matches)`
 * changes a media query's answer and fires `change` on every list made for
 * it, like the browser does when the system switches to dark mode or turns
 * "reduce motion" on.
 *
 *     const media = stubMatchMedia({ '(prefers-color-scheme: dark)': true })
 *     media.set('(prefers-color-scheme: dark)', false)
 *
 * Undo with `vi.unstubAllGlobals()`.
 */
export function stubMatchMedia(initial: Record<string, boolean> = {}) {
    const state = new Map(Object.entries(initial))
    const listeners = new Map<string, Set<(e: MediaQueryListEvent) => void>>()

    function matchMedia(query: string): MediaQueryList {
        const own = listeners.get(query) ?? new Set()
        listeners.set(query, own)
        return {
            get matches() { return state.get(query) ?? false },
            media: query,
            onchange: null,
            addEventListener: (_type: string, fn: (e: MediaQueryListEvent) => void) => { own.add(fn) },
            removeEventListener: (_type: string, fn: (e: MediaQueryListEvent) => void) => { own.delete(fn) },
            addListener: (fn: (e: MediaQueryListEvent) => void) => { own.add(fn) },
            removeListener: (fn: (e: MediaQueryListEvent) => void) => { own.delete(fn) },
            dispatchEvent: () => true,
        } as unknown as MediaQueryList
    }

    vi.stubGlobal('matchMedia', matchMedia)
    return {
        set(query: string, matches: boolean) {
            state.set(query, matches)
            for (const fn of listeners.get(query) ?? []) { fn({ matches, media: query } as MediaQueryListEvent) }
        },
        /** How many `change` listeners are attached for `query` (to check clean-up). */
        listenerCount(query: string): number {
            return listeners.get(query)?.size ?? 0
        },
    }
}
