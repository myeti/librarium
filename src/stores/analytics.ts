import { defineStore } from 'pinia'
import { analytics, resolveStoredConsent, type AnalyticsConsent } from '@/services/analytics'
import { saveConfig, type AppConfig } from '@/services/appConfig'
import { resetInstallationId } from '@/services/installationId'

/**
 * The user's analytics choice (opt-in, decision D2).
 *
 * `consent` is `null` until the user answers the consent dialog; App.vue
 * shows the dialog while `needsConsent` is true. The choice is saved in the
 * app config as `analyticsConsent` and mirrored into the analytics service,
 * which drops every event unless consent is `'granted'`.
 *
 * It also owns the installation ID's lifecycle (services/installationId.ts):
 * the ID is deleted whenever the answer is no, and at startup while no
 * choice is recorded (an ID from before the consent dialog is never reused).
 */
export const useAnalyticsStore = defineStore('analytics', {
    state: () => ({
        consent: null as AnalyticsConsent | null,
        /** False until the config was read, so the dialog doesn't flash before we know. */
        loaded: false,
    }),

    getters: {
        enabled: (state): boolean => state.consent === 'granted',
        /** Kept for the Options view: true whenever nothing may be sent. */
        optOut: (state): boolean => state.consent !== 'granted',
        needsConsent: (state): boolean => state.loaded && state.consent === null,
    },

    actions: {
        initFromConfig(cfg: AppConfig) {
            this.consent = resolveStoredConsent(cfg)
            analytics.setConsent(this.consent)
            this.loaded = true
            // No recorded choice: any installation ID was created without
            // consent. Drop it.
            if (cfg.analyticsConsent !== 'granted' && cfg.analyticsConsent !== 'denied') {
                resetInstallationId()
            }
            // An old explicit opt-out becomes an explicit 'denied' so the
            // dialog never asks someone who already said no.
            if (this.consent && cfg.analyticsConsent !== this.consent) {
                void saveConfig({ analyticsConsent: this.consent })
            }
            // An older setting, `appUid`: a random id (never sent). The one
            // ID is the installation ID; clear that one. The backend merges
            // keys, so `null` is how a value is removed.
            if (typeof cfg.appUid === 'string') {
                void saveConfig({ appUid: null })
            }
        },

        async setConsent(value: AnalyticsConsent) {
            this.consent = value
            analytics.setConsent(value)
            // Saying no deletes the installation ID: reports sent after a
            // later yes can't be linked to the earlier ones.
            if (value === 'denied') { resetInstallationId() }
            await saveConfig({ analyticsConsent: value })
            if (value === 'granted') { analytics.track('analytics_opt_in') }
        },

        grant() {
            return this.setConsent('granted')
        },

        deny() {
            return this.setConsent('denied')
        },

        /** The Settings switch. Opting out sends nothing (not even an event). */
        toggle() {
            return this.setConsent(this.enabled ? 'denied' : 'granted')
        },
    },
})
