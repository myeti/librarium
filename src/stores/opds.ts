import { defineStore } from 'pinia'
import * as api from '@/services/api'
import { saveConfig, type AppConfig } from '@/services/appConfig'
import { useNotificationsStore } from '@/stores/notifications'
import { generatePassword } from '@/utils/password'
import { errorMessage } from '@/utils/errors'
import { t } from '@/i18n'
import type { OpdsStatus } from '@/types'

/** User name the backend requires for HTTP Basic auth in LAN mode (docs/IPC.md). */
export const OPDS_USERNAME = 'librarium'

export const DEFAULT_OPDS_PORT = 8042
/** Ports below 1024 need administrator rights. */
export const MIN_OPDS_PORT = 1024
export const MAX_OPDS_PORT = 65535

/**
 * A port typed by the user (or read back from storage), or null when it
 * isn't one: only whole numbers from 1024 to 65535. `''`, `'abc'`, `'80'`,
 * `'8042.5'` and `'1e4'` are all refused, rather than turned into NaN or
 * silently clamped (the old field could save NaN).
 */
export function parsePort(value: unknown): number | null {
    const text = typeof value === 'number' ? String(value) : typeof value === 'string' ? value.trim() : ''
    if (!/^\d{1,5}$/.test(text)) { return null }
    const port = Number(text)
    return port >= MIN_OPDS_PORT && port <= MAX_OPDS_PORT ? port : null
}

/** An SVG document as a `data:` URL, to show it with `<img>` (never `v-html`). */
export function svgDataUrl(svg: string): string {
    return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
}

/**
 * The OPDS 2.0 (JSON) address of a catalog URL: `…/opds` → `…/opds/v2`.
 * `/opds` itself serves OPDS 1.2 (Atom), which most reading apps want.
 */
export function toOpds2Url(catalogUrl: string): string {
    return `${catalogUrl.replace(/\/+$/, '')}/v2`
}

interface OpdsState {
    enabled:     boolean
    port:        number
    shareOnLan:  boolean
    password:    string
    running:     boolean
    status:      OpdsStatus | null
    error:       string | null
    /** Folder currently served (the current user's library), or null. */
    libraryRoot: string | null
    /** QR code (SVG markup) of `qrFor`, or null. */
    qrSvg:       string | null
    /** The URL `qrSvg` encodes. */
    qrFor:       string | null
}

/**
 * The built-in OPDS server, which lets e-reader apps browse and download the
 * current user's library.
 *
 * Local-only by default: the backend binds 127.0.0.1, so only apps on this
 * computer can reach it. "Share on local network" binds all interfaces and
 * then requires a password (generated here, stored in the app config as
 * `opdsPassword`), because anyone on the same Wi-Fi could otherwise list and
 * download every book.
 *
 * The server follows the current user: the users store calls
 * `setLibraryRoot` when the user changes, which restarts the server on the
 * new user's folder (or stops it while no user is selected).
 *
 * While sharing on the LAN, Settings › OPDS shows the first LAN address as a
 * QR code (`refreshQr`), so a phone or tablet can pick it up. It encodes the
 * address only, never the password.
 */
export const useOpdsStore = defineStore('opds', {
    state: (): OpdsState => ({
        enabled:     localStorage.getItem('opdsEnabled') === 'true',
        // A damaged saved value (an old build could store "NaN") falls back to the default.
        port:        parsePort(localStorage.getItem('opdsPort')) ?? DEFAULT_OPDS_PORT,
        shareOnLan:  false,
        password:    '',
        running:     false,
        status:      null,
        error:       null,
        libraryRoot: null,
        qrSvg:       null,
        qrFor:       null,
    }),

    getters: {
        username: (): string => OPDS_USERNAME,
        localUrl: (state): string => state.status?.localUrl ?? `http://localhost:${state.port}/opds`,
        lanUrls:  (state): string[] => state.status?.lanUrls ?? [],

        /** The address to show as a QR code: the first LAN URL while sharing and running. */
        qrUrl(state): string | null {
            return state.shareOnLan && state.running ? (this.lanUrls[0] ?? null) : null
        },

        /** The QR code of `qrUrl` as an `<img>` source, once it has been made. */
        qrDataUrl(state): string | null {
            return state.qrSvg && state.qrFor && state.qrFor === this.qrUrl ? svgDataUrl(state.qrSvg) : null
        },

        /** The OPDS 2.0 address apps like Thorium can use: of the LAN URL when sharing, else local. */
        opds2Url(): string {
            return toOpds2Url(this.qrUrl ?? this.localUrl)
        },
    },

    actions: {
        initFromConfig(cfg: AppConfig) {
            this.shareOnLan = cfg.opdsShareOnLan ?? false
            this.password = cfg.opdsPassword ?? ''
        },

        /**
         * Points the server at another library. Starts it (if enabled) for a
         * new user, restarts it when the user switches, stops it when no user
         * is selected, so it never serves the previous user's books.
         */
        async setLibraryRoot(root: string | null) {
            if (root === this.libraryRoot && (this.running || !this.enabled)) { return }
            this.libraryRoot = root
            if (!root) {
                if (this.running) { await this.stop() }
                return
            }
            if (this.enabled) { await this.start() }
        },

        /** Starts (or restarts: the backend stops any running server first) with the current settings. */
        async start() {
            if (!this.libraryRoot) { return }
            if (this.shareOnLan && !this.password) {
                // LAN mode is refused without a password; make one rather than fail.
                this.password = generatePassword()
                void saveConfig({ opdsPassword: this.password })
            }
            this.error = null
            try {
                const status = await api.startOpds(
                    this.port,
                    this.libraryRoot,
                    this.shareOnLan,
                    this.shareOnLan ? this.password : null,
                )
                this.status = status ?? null
                this.running = status?.running ?? true
            } catch (e) {
                this.running = false
                this.status = null
                this.error = errorMessage(e)
                console.error('[OPDS] Failed to start server', e)
                useNotificationsStore().push('error', t('settings.opds.startFailed', { error: this.error }))
            }
        },

        async stop() {
            try {
                await api.stopOpds()
            } catch (e) {
                console.error('[OPDS] Failed to stop server', e)
            }
            this.running = false
            this.status = null
            this.qrSvg = null
            this.qrFor = null
        },

        /**
         * Reads the server's real state from the backend (it may have died, or
         * been started before this view was opened). A failure keeps what is
         * known.
         */
        async refreshStatus() {
            try {
                const status = await api.opdsStatus()
                this.status = status
                this.running = status?.running ?? false
            } catch (e) {
                console.error('[OPDS] Failed to read server status', e)
            }
        },

        /**
         * Makes the QR code of `qrUrl` (or forgets it when there is nothing
         * to show). Only the address is encoded, never the password. A failure
         * just leaves no QR code: the address is shown as text anyway.
         */
        async refreshQr() {
            const url = this.qrUrl
            if (!url) {
                this.qrSvg = null
                this.qrFor = null
                return
            }
            if (url === this.qrFor && this.qrSvg) { return }
            try {
                const svg = await api.opdsQrSvg(url)
                // The address may have changed while the code was being made.
                if (this.qrUrl === url) {
                    this.qrSvg = svg
                    this.qrFor = url
                }
            } catch (e) {
                console.error('[OPDS] Failed to make the QR code', errorMessage(e))
                this.qrSvg = null
                this.qrFor = null
            }
        },

        /** Restart only if running, so a settings change applies immediately. */
        async restartIfRunning() {
            if (this.running) { await this.start() }
        },

        /** Uses another port (restarting a running server); a value that isn't a valid port is ignored. */
        async setPort(port: number) {
            if (parsePort(port) === null) { return }
            this.port = port
            localStorage.setItem('opdsPort', String(port))
            await this.restartIfRunning()
        },

        async toggle() {
            this.enabled = !this.enabled
            localStorage.setItem('opdsEnabled', String(this.enabled))
            if (this.enabled) {
                await this.start()
            } else {
                await this.stop()
            }
        },

        async setShareOnLan(value: boolean) {
            this.shareOnLan = value
            if (value && !this.password) {
                this.password = generatePassword()
                void saveConfig({ opdsPassword: this.password })
            }
            void saveConfig({ opdsShareOnLan: value })
            await this.restartIfRunning()
        },

        /** New password; readers configured with the old one must be updated. */
        async regeneratePassword() {
            this.password = generatePassword()
            void saveConfig({ opdsPassword: this.password })
            if (this.shareOnLan) { await this.restartIfRunning() }
        },

        /**
         * The server keeps a snapshot of the book list, which it refreshes by
         * itself within a second of a change; calling this after a library
         * change (import, delete, metadata edit, convert, rename) makes it
         * immediate. A failure only means readers see the change a bit later.
         */
        async invalidateCache() {
            if (!this.running) { return }
            try {
                await api.invalidateOpdsCache()
            } catch (e) {
                console.error('[OPDS] Failed to invalidate cache', e)
            }
        },
    },
})
