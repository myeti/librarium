import { defineStore } from 'pinia'
import { saveConfigOrThrow, type AppConfig } from '@/services/appConfig'
import { useSystemStore } from '@/stores/system'

/** How much the background effects move: nothing, a calmer version, or the full one. */
export type EffectsLevel = 'off' | 'subtle' | 'on'
export const EFFECTS_LEVELS: readonly EffectsLevel[] = ['off', 'subtle', 'on']

export function isEffectsLevel(value: unknown): value is EffectsLevel {
    return typeof value === 'string' && (EFFECTS_LEVELS as readonly string[]).includes(value)
}

interface SettingsState {
    /** Volume of the sound effects, 0–1 (0 mutes them). */
    soundVolume: number
    /** The startup sound; off unless the user turns it on. */
    launchSound: boolean
    /** The user's choice of background effects; null: never chosen (see `effectsLevel`). */
    effects: EffectsLevel | null
    libraryRootOverride: string | null
    deviceSubfolder: string
    googleBooksApiKey: string
    /** Experimental support for MTP Kindles (2024+), off by default. */
    mtpEnabled: boolean
    /** Ask GitHub for a new version at each launch (Settings › Updates); on by default. */
    autoCheckUpdates: boolean
    /** Fetch the "What's new" messages at each launch (Settings › Updates); on by default. */
    showNews: boolean
}

/**
 * App-wide settings (not per user), saved in the app config (`get_config` /
 * `set_config`; read once by App.vue through `initFromConfig`).
 *
 * The setters apply the new value at once and save it. They return the
 * save's promise, which rejects with the backend's error when the setting
 * couldn't be written: the value then stays in effect for this session,
 * and Settings says it wasn't saved (stores/saveStatus.ts). MTP is the
 * exception: the backend acts on it, so a failed save puts it back.
 */
export const useSettingsStore = defineStore('settings', {
    state: (): SettingsState => ({
        soundVolume: 0.8,
        launchSound: false,
        effects: null,
        libraryRootOverride: null,
        deviceSubfolder: '',
        googleBooksApiKey: '',
        mtpEnabled: false,
        autoCheckUpdates: true,
        showNews: true,
    }),

    getters: {
        /**
         * The background effects to show: the user's choice, else Off when
         * the system asks to reduce motion, else Subtle. Follows the system
         * setting live until the user chooses.
         */
        effectsLevel: (state): EffectsLevel => {
            if (state.effects) { return state.effects }
            return useSystemStore().prefersReducedMotion ? 'off' : 'subtle'
        },
    },

    actions: {
        initFromConfig(cfg: AppConfig) {
            if (typeof cfg.soundVolume === 'number' && Number.isFinite(cfg.soundVolume)) {
                this.soundVolume = clampVolume(cfg.soundVolume)
            }
            if (cfg.libraryRootOverride !== undefined)  {this.libraryRootOverride  = cfg.libraryRootOverride ?? null}
            if (cfg.deviceSubfolder !== undefined)      {this.deviceSubfolder      = cfg.deviceSubfolder}
            if (cfg.googleBooksApiKey !== undefined)    {this.googleBooksApiKey    = cfg.googleBooksApiKey}
            this.effects = isEffectsLevel(cfg.effects) ? cfg.effects : null
            // Off unless explicitly turned on.
            this.launchSound = cfg.launchSound === true
            // The backend reads it the same way: anything but `true` is off.
            this.mtpEnabled = cfg.mtpEnabled === true
            // On unless explicitly turned off.
            this.autoCheckUpdates = cfg.autoCheckUpdates !== false
            this.showNews = cfg.showNews !== false
        },

        setSoundVolume(v: number): Promise<void> {
            this.soundVolume = clampVolume(v)
            return saveConfigOrThrow({ soundVolume: this.soundVolume })
        },

        setLaunchSound(enabled: boolean): Promise<void> {
            this.launchSound = enabled
            return saveConfigOrThrow({ launchSound: enabled })
        },

        setEffects(level: EffectsLevel): Promise<void> {
            this.effects = level
            return saveConfigOrThrow({ effects: level })
        },

        setGoogleBooksApiKey(key: string): Promise<void> {
            this.googleBooksApiKey = key.trim()
            return saveConfigOrThrow({ googleBooksApiKey: this.googleBooksApiKey })
        },

        setLibraryRootOverride(path: string | null): Promise<void> {
            this.libraryRootOverride = path
            return saveConfigOrThrow({ libraryRootOverride: path })
        },

        setDeviceSubfolder(sub: string): Promise<void> {
            this.deviceSubfolder = sub
            return saveConfigOrThrow({ deviceSubfolder: sub })
        },

        /** Takes effect at the next launch ("Check now" still works when off). */
        setAutoCheckUpdates(enabled: boolean): Promise<void> {
            this.autoCheckUpdates = enabled
            return saveConfigOrThrow({ autoCheckUpdates: enabled })
        },

        /** Takes effect at the next launch. */
        setShowNews(enabled: boolean): Promise<void> {
            this.showNews = enabled
            return saveConfigOrThrow({ showNews: enabled })
        },

        /**
         * Turns experimental MTP support on or off. The backend applies it as
         * soon as it is saved, so unlike the other settings a failed save
         * isn't kept for the session: the switch goes back, and the promise
         * rejects with the backend's error (for the caller to show).
         */
        async setMtpEnabled(enabled: boolean) {
            const previous = this.mtpEnabled
            this.mtpEnabled = enabled
            try {
                await saveConfigOrThrow({ mtpEnabled: enabled })
            } catch (e) {
                this.mtpEnabled = previous
                throw e
            }
        },
    },
})

function clampVolume(v: number): number {
    return Number.isFinite(v) ? Math.max(0, Math.min(1, v)) : 0.8
}
