import { defineStore } from 'pinia'
import { errorMessage } from '@/utils/errors'

/** How long "Saved" stays next to a setting. */
export const SAVED_VISIBLE_MS = 2500

export type SaveState = 'saving' | 'saved' | 'error'

export interface SaveStatusEntry {
    state: SaveState
    /** Why the save failed (`error` only). */
    error: string
}

interface SaveStatusState {
    /** By setting key (e.g. `devices.subfolder`); absent: nothing to say. */
    entries: Record<string, SaveStatusEntry>
}

// "Saved" timers, by key (kept out of the state).
const timers = new Map<string, ReturnType<typeof setTimeout>>()

/**
 * Save feedback for the Settings screen. Settings apply as soon as they
 * change; `track(key, save)` follows the save so the setting's row can say
 * "Saved" for a moment, or "Not saved: <reason>" until the next attempt
 * (components/settings/SaveStatus.vue shows it).
 */
export const useSaveStatusStore = defineStore('saveStatus', {
    state: (): SaveStatusState => ({
        entries: {},
    }),

    getters: {
        statusOf: (state) => (key: string): SaveStatusEntry | null => state.entries[key] ?? null,
    },

    actions: {
        /**
         * Waits for `save` (a promise, or a function returning one) and
         * records how it went for `key`. Resolves with whether it was saved;
         * never rejects, so callers can `void` it.
         */
        async track(key: string, save: Promise<unknown> | (() => Promise<unknown>)): Promise<boolean> {
            this.stopTimer(key)
            this.entries[key] = { state: 'saving', error: '' }
            try {
                await (typeof save === 'function' ? save() : save)
            } catch (e) {
                console.error(`[settings] ${key} could not be saved`, e)
                this.entries[key] = { state: 'error', error: errorMessage(e) }
                return false
            }
            this.entries[key] = { state: 'saved', error: '' }
            timers.set(key, setTimeout(() => this.clear(key), SAVED_VISIBLE_MS))
            return true
        },

        clear(key: string) {
            this.stopTimer(key)
            const { [key]: _removed, ...rest } = this.entries
            this.entries = rest
        },

        stopTimer(key: string) {
            const timer = timers.get(key)
            if (timer !== undefined) {
                clearTimeout(timer)
                timers.delete(key)
            }
        },
    },
})
