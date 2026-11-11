import * as api from '@/services/api'
import type { OpdsCatalogSource } from '@/stores/opdsSources'

/**
 * The app settings, saved by the backend in the app data folder
 * (`get_config` / `set_config`, see docs/IPC.md). Every key is optional: a
 * missing key means "never set", and the stores fall back to their defaults.
 */
export interface AppConfig {
    /**
     * An older setting: a random id, never sent. Never written; cleared
     * (set to null) at startup by the analytics store. The installation ID is services/installationId.ts.
     */
    appUid?:               string | null
    /** Legacy opt-out switch, read once to migrate to `analyticsConsent`. */
    analyticsOptOut?:      boolean
    /** Explicit analytics choice (opt-in). Absent = not asked yet. */
    analyticsConsent?:     'granted' | 'denied'
    /** A ThemeName; 0.5's seasonal names are migrated on load (`themeNameFrom`). */
    theme?:                string
    lang?:                 string
    /** Check GitHub for a new version at each launch (on when absent). */
    autoCheckUpdates?:     boolean
    /** Fetch the "What's new" messages from the Librarium server at each launch (on when absent). */
    showNews?:             boolean
    soundVolume?:          number
    /** Background effects: `off`, `subtle` or `on`. Absent: Subtle, or Off under "reduce motion". */
    effects?:              string
    /** Play a sound when the app starts (off unless set). */
    launchSound?:          boolean
    libraryRootOverride?:  string | null
    deviceSubfolder?:      string
    googleBooksApiKey?:    string
    opdsCatalogs?:         OpdsCatalogSource[]
    /** Whether the OPDS server listens on the local network (off = this computer only). */
    opdsShareOnLan?:       boolean
    /** HTTP Basic password required by the OPDS server in LAN mode (user `librarium`). */
    opdsPassword?:         string
    /**
     * Experimental support for MTP Kindles (2024+); off when absent. The
     * backend reads it too, and applies a change at once (docs/IPC.md):
     * save it with `saveConfigOrThrow`.
     */
    mtpEnabled?:           boolean
}

let cache: AppConfig | null = null

/** Reads the settings once per run (later calls get the cached copy); `{}` if they can't be read. */
export async function loadConfig(): Promise<AppConfig> {
    if (cache) {return cache}
    try {
        // The file holds whatever was saved; each store checks the values it reads.
        cache = await api.getConfig()
    } catch (e) {
        console.error('[appConfig] get_config failed', e)
        cache = {}
    }
    return cache
}

/**
 * Saves some settings (the others are kept). Never rejects: a failure is
 * logged, and the value stays in effect for this run.
 */
export async function saveConfig(updates: Partial<AppConfig>): Promise<void> {
    if (cache) {Object.assign(cache, updates)}
    try {
        await api.setConfig(updates)
    } catch (e) {
        console.error('[appConfig] set_config failed', e)
    }
}

/**
 * Saves settings the backend itself acts on (`mtpEnabled`), and rejects
 * with the backend's error when they couldn't be saved: then they weren't
 * applied either, and the UI must not show them as if they were.
 */
export async function saveConfigOrThrow(updates: Partial<AppConfig>): Promise<void> {
    await api.setConfig(updates)
    if (cache) {Object.assign(cache, updates)}
}
