<template>
    <div class="app-shell">
        <!-- Decorative, behind everything; Off / Subtle / On in Settings › Appearance. -->
        <BackgroundEffects
            v-if="effectsLevel !== 'off'"
            :theme="themeStore.theme"
            :level="effectsLevel"
        />

        <Transition name="fade">
            <UserPicker v-if="!usersStore.currentUser" />
        </Transition>

        <template v-if="usersStore.currentUser">
            <!-- First: its commands are registered before any view's (see AppCommands). -->
            <AppCommands />
            <AppSidebar />
            <div class="app-content">
                <RouterView v-slot="{ Component }">
                    <Transition name="fade-y" mode="out-in">
                        <component :is="Component" />
                    </Transition>
                </RouterView>
            </div>
            <!-- Files dropped on any view are imported. -->
            <FileDropTarget />
            <MessagesModal
                v-if="updatesStore.pendingMessages.length > 0"
                :messages="updatesStore.pendingMessages"
                @close="updatesStore.dismissMessages()"
            />
        </template>

        <!-- Opt-in analytics: asked once, before anything is sent. -->
        <AnalyticsConsentDialog v-if="analyticsStore.needsConsent" />

        <DebugPanel />
        <AppNotifications />
    </div>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { RouterView } from 'vue-router'
    import AppSidebar from '@/components/AppSidebar.vue'
    import AppCommands from '@/components/AppCommands.vue'
    import UserPicker from '@/components/UserPicker.vue'
    import DebugPanel from '@/components/DebugPanel.vue'
    import BackgroundEffects from '@/components/effects/BackgroundEffects.vue'
    import { useDeviceStore } from '@/stores/device'
    import { useUsersStore } from '@/stores/users'
    import { useLogStore } from '@/stores/log'
    import { useThemeStore } from '@/stores/theme'
    import { useSystemStore } from '@/stores/system'
    import { useOpdsStore } from '@/stores/opds'
    import { useSettingsStore, type EffectsLevel } from '@/stores/settings'
    import { playIntro } from '@/utils/sounds'
    import { analytics, Analytics } from '@/services/analytics'
    import * as api from '@/services/api'
    import { useUpdatesStore } from '@/stores/updates'
    import MessagesModal from '@/components/MessagesModal.vue'
    import AppNotifications from '@/components/AppNotifications.vue'
    import AnalyticsConsentDialog from '@/components/AnalyticsConsentDialog.vue'
    import FileDropTarget from '@/components/library/FileDropTarget.vue'
    import { useAnalyticsStore } from '@/stores/analytics'
    import { getVersion } from '@tauri-apps/api/app'
    import { cacheAppVersion, reportError } from '@/services/librarium-api'
    import { loadConfig } from '@/services/appConfig'
    import { useOpdsSourcesStore } from '@/stores/opdsSources'
    import { errorMessage } from '@/utils/errors'
    import { currentLang, isLang, setLang, type Lang } from '@/i18n'
    import { themeNameFrom } from '@/stores/theme'
    import router from '@/router'

    /**
     * The app shell: background effects, the user picker or the sidebar and
     * the current view, the app-wide commands and dialogs. `mounted` runs the
     * startup sequence (settings, users and library, device polling, update
     * checks).
     *
     * It keeps three things in step with the settings: the `data-theme`
     * attribute (the theme), the background effects level, and the language
     * of the native menu bar.
     */
    export default defineComponent({
        name: 'App',

        components: {
            RouterView,
            AppSidebar,
            AppCommands,
            UserPicker,
            DebugPanel,
            BackgroundEffects,
            MessagesModal,
            AppNotifications,
            AnalyticsConsentDialog,
            FileDropTarget,
        },

        computed: {
            ...mapStores(
                useDeviceStore, useUsersStore, useThemeStore, useUpdatesStore, useAnalyticsStore,
                useLogStore, useSettingsStore, useOpdsSourcesStore, useOpdsStore, useSystemStore,
            ),

            effectsLevel(): EffectsLevel {
                return this.settingsStore.effectsLevel
            },

            lang(): Lang {
                return currentLang.value
            },
        },

        watch: {
            // The theme changed (Settings › Appearance).
            'themeStore.dataTheme'() {
                this.themeStore.applyTheme()
            },

            // The native menu bar (macOS) speaks the app's language.
            lang: {
                handler(lang: Lang) {
                    api.setMenuLanguage(lang).catch((e: unknown) => {
                        console.error('[menu] could not change the menu language', errorMessage(e))
                    })
                },
                immediate: true,
            },
        },

        async mounted() {
            this.logStore.install()
            // The effects default follows the system's "reduce motion", live.
            this.systemStore.install()

            // Restore persisted preferences from the app config.
            const cfg = await loadConfig()
            // A seasonal theme saved by 0.5 becomes its successor (saved as
            // such the next time the theme changes).
            const savedTheme = themeNameFrom(cfg.theme)
            if (savedTheme)                 {this.themeStore.theme = savedTheme}
            if (isLang(cfg.lang))           {setLang(cfg.lang)}
            // Analytics stays off until the user has answered the consent
            // dialog (shown by the template when no choice is recorded).
            this.analyticsStore.initFromConfig(cfg)

            // Initialise settings stores from config (replaces localStorage).
            // OPDS settings must be known before usersStore.init(), which
            // starts the server for the current user.
            this.settingsStore.initFromConfig(cfg)
            this.opdsSourcesStore.initFromConfig(cfg.opdsCatalogs ?? [])
            this.opdsStore.initFromConfig(cfg)

            this.themeStore.applyTheme()

            // Opens the current user's library and starts the OPDS server
            // for it; with several users that happens once one is picked.
            try {
                await this.usersStore.init()
            } catch (e) {
                reportError('USERS_INIT_FAILED', 'app.startup', e)
            }

            try {
                await this.deviceStore.setupProgressListener()
            } catch (e) {
                // Transfers would then never show progress: worth knowing.
                console.error('[app] could not listen for transfer progress', e)
            }
            await this.deviceStore.pollDevice()
            this.deviceStore.startPolling()
            // Off unless the user turned it on (Settings › Appearance).
            if (this.settingsStore.launchSound) { playIntro() }

            // Background: the update check and the "What's new" messages, each
            // only if left on in Settings › Updates (silently fail offline).
            const version = await getVersion().catch(() => 'unknown')
            cacheAppVersion(version)
            void this.updatesStore.checkAtStartup()

            // Analytics (only with consent): user properties, sent once per
            // session, and the theme. Nothing else about the setup is sent.
            const theme = this.themeStore
            analytics.setUserProperties({
                os:          Analytics.getOs(),
                app_version: version,
                language:    navigator.language.split('-')[0],
            })
            analytics.track('app_start', { theme: theme.theme })

            // Track app close via beforeunload (reliable in Tauri WebView)
            window.addEventListener('beforeunload', () => analytics.trackSessionEnd())

            // Track navigation — pageview populates Overview, event populates Events tab
            router.afterEach(to => {
                const view = String(to.name ?? to.path)
                analytics.trackPageview(`/${view}`, view)
                analytics.track('view_open', { view })
            })
        },

        beforeUnmount() {
            this.systemStore.uninstall()
        },
    })
</script>

<style lang="scss">
.app-shell {
    display: flex;
    height: 100vh;
    overflow: hidden;
    position: relative;
}

.fade-y-enter-active { transition: opacity var(--duration) ease, transform var(--duration) ease; }
.fade-y-leave-active { transition: opacity var(--duration-fast) ease, transform var(--duration-fast) ease; }
.fade-y-enter-from   { opacity: 0; transform: translateY(10px); }
.fade-y-leave-to     { opacity: 0; transform: translateY(-6px); }

.app-content {
    flex: 1;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    position: relative;
    z-index: 1;
}
</style>
