import { defineStore } from 'pinia'

/** The "reduce motion" accessibility setting (the background effects' default). */
export const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)'

interface SystemState {
    /** The user asked the system to reduce motion. */
    prefersReducedMotion: boolean
}

/** `window.matchMedia(query)`, or null where it doesn't exist (some test environments). */
function mediaQuery(query: string): MediaQueryList | null {
    return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
        ? window.matchMedia(query)
        : null
}

// The listeners `install()` added, to remove them (kept out of the state).
let cleanup: (() => void) | null = null

/**
 * What the operating system says about the user's preferences, live:
 * "reduce motion" (the background effects' default).
 *
 * The value is read when the store is created, so it is right from the
 * first render; `install()` (App.vue, once) then follows changes.
 */
export const useSystemStore = defineStore('system', {
    state: (): SystemState => ({
        prefersReducedMotion: mediaQuery(REDUCED_MOTION_QUERY)?.matches ?? false,
    }),

    actions: {
        /** Follows the system preferences from now on (idempotent). */
        install() {
            if (cleanup) { return }
            const motion = mediaQuery(REDUCED_MOTION_QUERY)
            const onMotion = (e: MediaQueryListEvent) => { this.prefersReducedMotion = e.matches }
            motion?.addEventListener('change', onMotion)
            // Read again: the system may have changed since the store was created.
            this.prefersReducedMotion = motion?.matches ?? this.prefersReducedMotion
            cleanup = () => {
                motion?.removeEventListener('change', onMotion)
            }
        },

        uninstall() {
            cleanup?.()
            cleanup = null
        },
    },
})
