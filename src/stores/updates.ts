import { defineStore } from 'pinia'
import { markRaw } from 'vue'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { fetchMessages } from '@/services/librarium-api'
import type { ApiMessage } from '@/services/librarium-api'
import { useNotificationsStore } from '@/stores/notifications'
import { useSettingsStore } from '@/stores/settings'
import { errorMessage } from '@/utils/errors'
import { t } from '@/i18n'

interface PendingRelease {
    version: string
    /** The release notes (`update.body`, from `latest.json`); may be empty. */
    changelog: string
    date: string
}

const LAST_MSG_KEY = 'lastMessageId'

function currentOs(): string {
    const ua = navigator.userAgent
    if (ua.includes('Mac')) {return 'macos'}
    if (ua.includes('Win')) {return 'windows'}
    return 'linux'
}

function storedLastMessageId(): number | null {
    const v = localStorage.getItem(LAST_MSG_KEY)
    return v !== null ? Number(v) : null
}

/**
 * App updates (Tauri updater) and "What's new" messages from the Librarium API.
 *
 * At startup, `checkAtStartup()` runs the two checks the user left on in
 * Settings › Updates ("Check for updates automatically", "Show news from
 * Librarium"; both on by default): with both off, Librarium contacts neither
 * GitHub nor its own server at launch (PRIVACY.md).
 *
 * Update flow: the update check asks the updater whether a newer
 * version is published; if so `newRelease` describes it (Options shows the
 * version and its release notes). `downloadAndInstall()` downloads and
 * installs it, then relaunches the app (`tauri-plugin-process`): without the
 * relaunch the previous version would keep running and the button spin
 * forever on macOS. On Windows the installer quits the app itself, so the relaunch is
 * never reached there. Any failure is shown (toast, and `installError` for
 * the Options card) and the button becomes usable again.
 *
 * The `Update` object is kept with `markRaw`: it keeps its resource id in a
 * private class field, which can't be read through the reactive proxy Pinia
 * would otherwise wrap it in (calling its methods would throw).
 */
export const useUpdatesStore = defineStore('updates', {
    state: () => ({
        newRelease:      null as PendingRelease | null,
        pendingMessages: [] as ApiMessage[],
        lastMessageId:   storedLastMessageId(),
        pendingUpdate:   null as Update | null,
        installing:      false,
        /** Why the last install attempt failed, or null. */
        installError:    null as string | null,
    }),

    getters: {
        hasNewRelease: (state): boolean => state.newRelease !== null,
    },

    actions: {
        /** The launch-time checks, as allowed by the two settings. Never rejects. */
        async checkAtStartup() {
            const settings = useSettingsStore()
            await Promise.all([
                settings.autoCheckUpdates ? this.checkReleases() : null,
                settings.showNews ? this.checkMessages() : null,
            ])
        },

        async checkReleases() {
            try {
                const update = await check()
                if (update?.available) {
                    this.pendingUpdate = markRaw(update)
                    this.newRelease = {
                        version:   update.version,
                        changelog: update.body?.trim() ?? '',
                        date:      update.date ?? new Date().toISOString(),
                    }
                }
            } catch (e) {
                // Offline, or the update server is down: try again next launch.
                console.warn('[updates] update check failed', e)
            }
        },

        async checkMessages() {
            const all = await fetchMessages(this.lastMessageId)
            const os  = currentOs()
            this.pendingMessages = all.filter(m => !m.constraintOs || m.constraintOs === os)
        },

        /**
         * Downloads and installs the pending update, then restarts into it.
         * Resolves either way; a failure is shown and `installing` is reset.
         */
        async downloadAndInstall() {
            const update = this.pendingUpdate
            if (!update || this.installing) {return}
            this.installing = true
            this.installError = null
            try {
                await update.downloadAndInstall()
            } catch (e) {
                this.failInstall(t('settings.updates.installFailed', { error: errorMessage(e) }))
                return
            }
            try {
                await relaunch()
            } catch (e) {
                // Installed, but still running the old version until restarted.
                this.failInstall(t('settings.updates.restartFailed', { error: errorMessage(e) }))
            }
        },

        failInstall(message: string) {
            console.error('[updates]', message)
            this.installing = false
            this.installError = message
            useNotificationsStore().push('error', message)
        },

        dismissMessages() {
            if (this.pendingMessages.length === 0) {return}
            const maxId = Math.max(...this.pendingMessages.map(m => m.id))
            this.lastMessageId = maxId
            localStorage.setItem(LAST_MSG_KEY, String(maxId))
            this.pendingMessages = []
        },
    },
})
