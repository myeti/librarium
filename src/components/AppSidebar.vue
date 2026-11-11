<template>
    <aside class="sidebar">
        <!-- The wordmark, lettered like the theme's cover, between two of its ornaments. -->
        <div class="sidebar_brand">
            <span class="ornament -rule sidebar_rule" aria-hidden="true"></span>
            <p class="sidebar_logo display-title foil-text">Librarium</p>
            <span class="ornament -rule sidebar_rule -bottom" aria-hidden="true"></span>
        </div>

        <!-- Navigation -->
        <nav class="sidebar_nav" :aria-label="$t('nav.label')">
            <RouterLink to="/library" class="nav-item">
                <vue-feather type="book" size="18" aria-hidden="true" />
                <span>{{ $t('nav.books') }}</span>
                <span v-if="totalBooks > 0" class="nav-item_count">{{ totalBooks }}</span>
            </RouterLink>
            <ActivityPopover />
            <RouterLink to="/discover" class="nav-item">
                <vue-feather type="compass" size="18" aria-hidden="true" />
                <span>{{ $t('nav.discover') }}</span>
            </RouterLink>
            <RouterLink to="/settings" class="nav-item">
                <vue-feather type="settings" size="18" aria-hidden="true" />
                <span>{{ $t('nav.settings') }}</span>
                <span v-if="updatesStore.hasNewRelease" class="nav-item_update" :title="$t('sidebar.newVersion')">
                    <span class="sr-only">{{ $t('sidebar.newVersion') }}</span>
                </span>
            </RouterLink>
        </nav>

        <!-- Connected device info -->
        <div class="sidebar_device" :class="{ '-connected': deviceConnected }">
            <div class="device-top">
                <vue-feather type="tablet" size="24" class="device-icon" aria-hidden="true" />
                <!-- Connected: the name shows the device's books in the library view. -->
                <RouterLink
                    v-if="deviceConnected"
                    :to="deviceBooksLink"
                    class="device-name -link"
                    :title="$t('shelf.showDeviceBooks', { device: deviceBrand })"
                >
                    <span class="device-brand">{{ deviceBrand }}</span>
                    <span v-if="deviceModel" class="device-model">{{ deviceModel }}</span>
                </RouterLink>
                <div v-else class="device-name" :title="$t('sidebar.deviceHint')">
                    <!-- No usable device, but an MTP Kindle is plugged in: named here, explained below. -->
                    <span v-if="mtpNotices.length > 0" class="device-brand">{{ mtpName(mtpNotices[0]) }}</span>
                    <span v-else class="device-brand">{{ $t('sidebar.noDevice') }}</span>
                </div>
                <IconButton
                    class="device-refresh"
                    :class="{ '-busy': deviceStore.detecting }"
                    icon="refresh-cw"
                    size="sm"
                    :label="$t('sidebar.detect')"
                    :title="detectTitle"
                    :disabled="deviceStore.detecting"
                    @click="detect"
                />
                <IconButton
                    v-if="deviceConnected"
                    class="device-eject"
                    :class="{ '-busy': deviceStore.ejecting !== null }"
                    :icon="deviceStore.ejecting !== null ? 'loader' : 'log-out'"
                    size="sm"
                    :label="ejectTitle"
                    :disabled="!deviceStore.canEject"
                    @click="eject"
                />
            </div>
            <!-- Several e-readers connected: which one the views show and books go to. -->
            <select
                v-if="deviceOptions.length > 1"
                class="select device-select"
                :value="deviceStore.selectedId ?? ''"
                :title="$t('devices.choose')"
                :aria-label="$t('devices.choose')"
                @change="onSelectDevice"
            >
                <option v-for="option in deviceOptions" :key="option.id" :value="option.id">{{ option.label }}</option>
            </select>
            <div v-if="deviceStore.device?.freeSpaceBytes != null" class="device-space">
                <div class="device-space-bar" aria-hidden="true">
                    <div class="device-space-used" :style="{ width: usedSpacePct + '%' }"></div>
                </div>
                <span class="device-space-label">{{ fmtSize(deviceStore.device.freeSpaceBytes) }} {{ $t('sidebar.free') }}</span>
            </div>
            <!-- MTP Kindles plugged in but not usable (yet): why, and what to do. -->
            <div v-for="notice in mtpNotices" :key="notice.mountPath" class="device-notice" :data-state="notice.state" role="status">
                <span class="device-notice-text">
                    <vue-feather v-if="notice.state === 'connecting'" type="loader" size="12" class="spinning" aria-hidden="true" />
                    <vue-feather v-else-if="isMtpProblem(notice)" type="alert-triangle" size="12" aria-hidden="true" />
                    {{ mtpNoticeText(notice) }}
                </span>
                <span v-if="mtpNoticeDetail(notice)" class="device-notice-detail">{{ mtpNoticeDetail(notice) }}</span>
                <RouterLink v-if="notice.state === 'mtpDisabled'" :to="mtpSettingLink" class="device-notice-link">
                    {{ $t('devices.mtp.openSettings') }}
                </RouterLink>
            </div>
        </div>

        <!-- User chip — click to switch user -->
        <button
            type="button"
            class="user-chip"
            :title="$t('sidebar.switchUser')"
            :aria-label="$t('sidebar.userLabel', { name: userName })"
            @click="usersStore.clearUser()"
        >
            <span class="user-chip_avatar" :style="{ background: avatarColor(userName) }" aria-hidden="true">
                {{ initials(userName) }}
            </span>
            <span class="user-chip_name">{{ userName }}</span>
            <vue-feather type="repeat" size="14" class="user-chip_icon" aria-hidden="true" />
        </button>

        <!-- Language toggle + about button -->
        <div class="sidebar_bottom">
            <div class="lang-toggle" role="group" :aria-label="$t('sidebar.languageLabel')">
                <button
                    v-for="option in LANG_OPTIONS"
                    :key="option.lang"
                    type="button"
                    :lang="option.lang"
                    :class="{ active: currentLang === option.lang }"
                    :aria-pressed="currentLang === option.lang ? 'true' : 'false'"
                    :aria-label="option.name"
                    :title="option.name"
                    @click="setLang(option.lang)"
                >
                    {{ option.short }}
                </button>
            </div>
            <button type="button" class="sidebar_about-btn" @click="shortcutsStore.runCommand('app.about')">{{ $t('sidebar.about') }}</button>
        </div>
    </aside>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { RouterLink } from 'vue-router'
    import { useLibraryStore } from '@/stores/library'
    import { useDeviceStore } from '@/stores/device'
    import { useUsersStore } from '@/stores/users'
    import { useUpdatesStore } from '@/stores/updates'
    import { useShortcutsStore } from '@/stores/shortcuts'
    import { currentLang, setLang, type Lang } from '@/i18n'
    import { formatCombo, SHORTCUTS } from '@/constants/shortcuts'
    import { fmtSize } from '@/utils/format'
    import { initials, avatarColor } from '@/utils/avatar'
    import { isMtpDevice, mtpPort } from '@/utils/mtp'
    import type { MtpDevice } from '@/types'
    import IconButton from '@/components/ui/IconButton.vue'
    import ActivityPopover from '@/components/ActivityPopover.vue'

    /** The MTP setting in Settings (SettingsView opens Devices and scrolls to it). */
    const MTP_SETTING_LINK = { path: '/settings', hash: '#mtp-setting' }

    /** The language switch: each language named in itself. */
    const LANG_OPTIONS: readonly { lang: Lang, short: string, name: string }[] = [
        { lang: 'fr', short: 'FR', name: 'Français' },
        { lang: 'en', short: 'EN', name: 'English' },
    ]

    /**
     * The left column: navigation (with counts), the Activity popover
     * (transfers), the connected e-reader (a picker when there are several,
     * its free space, look for one, eject), the MTP Kindles plugged in that
     * can't be used (yet) and why, the current user (click to switch), the
     * language and About (an app-level dialog, see AppCommands).
     *
     * There is no Device view: the e-reader's books are in the library
     * view, with its own. The connected device's name links there, showing
     * the device's books (`/library?scope=device`).
     */
    export default defineComponent({
        name: 'AppSidebar',

        components: { RouterLink, IconButton, ActivityPopover },

        data() {
            return {
                mtpSettingLink: MTP_SETTING_LINK,
                LANG_OPTIONS,
            }
        },

        computed: {
            ...mapStores(useLibraryStore, useDeviceStore, useUsersStore, useUpdatesStore, useShortcutsStore),

            currentLang(): Lang { return currentLang.value },
            totalBooks(): number { return this.libraryStore.books.length },
            deviceConnected(): boolean { return this.deviceStore.isConnected },
            /** The library view showing the device's books (LibraryView reads `scope`). */
            deviceBooksLink(): { path: string, query: { scope: 'device' } } {
                return { path: '/library', query: { scope: 'device' } }
            },
            mtpNotices(): MtpDevice[] { return this.deviceStore.mtpNotices },
            userName(): string { return this.usersStore.currentUser ?? '' },

            /** "Look for an e-reader (⌘K)". */
            detectTitle(): string {
                const combo = SHORTCUTS['library.detectDevice'].keys[0]
                return `${this.$t('sidebar.detect')} (${formatCombo(combo, this.shortcutsStore.isMac)})`
            },

            deviceBrand(): string {
                const d = this.deviceStore.device
                if (!d) {return ''}
                return d.deviceType === 'kobo' ? 'Kobo' : 'Kindle'
            },

            deviceModel(): string | null {
                const m = this.deviceStore.device?.model
                if (!m) {return null}
                // Strip the brand prefix ("Kindle " / "Kobo ") to keep only the detail.
                return m.replace(/^(kindle|kobo)\s+/i, '') || null
            },

            /**
             * The connected devices for the picker. Two devices with the same
             * name (two Paperwhites) are told apart by their volume name, or
             * for MTP Kindles by their USB port (never their `mtp://` locator).
             */
            deviceOptions(): { id: string, label: string }[] {
                const devices = this.deviceStore.devices
                return devices.map(d => {
                    const twin = devices.some(other => other.id !== d.id && other.name === d.name)
                    const where = isMtpDevice(d)
                        ? `MTP ${mtpPort(d.mountPath)}`
                        : d.mountPath.split(/[\\/]/).filter(Boolean).pop() ?? d.mountPath
                    return { id: d.id, label: twin ? `${d.name} (${where})` : d.name }
                })
            },

            /** Tooltip of the eject button: what it does, or why it can't yet. */
            ejectTitle(): string {
                const name = this.deviceStore.device?.name ?? ''
                if (this.deviceStore.ejecting !== null) { return this.$t('devices.ejecting', { name }) }
                if (!this.deviceStore.canEject) { return this.$t('devices.ejectBusy', { name }) }
                return this.$t('devices.ejectTitle', { name })
            },

            /** Percentage of device storage already used (0–100). */
            usedSpacePct(): number {
                const d = this.deviceStore.device
                if (!d?.totalSpaceBytes || !d?.freeSpaceBytes) {return 0}
                return Math.round((1 - d.freeSpaceBytes / d.totalSpaceBytes) * 100)
            },
        },

        methods: {
            fmtSize,
            initials,
            avatarColor,
            setLang,

            /** An MTP Kindle's name (the model once connected, else the USB product name). */
            mtpName(notice: MtpDevice): string {
                return notice.name.trim() || 'Kindle'
            },

            /**
             * Why an MTP Kindle can't be used, in the UI's language: chosen by
             * state (the backend's message, in English, is the detail).
             */
            mtpNoticeText(notice: MtpDevice): string {
                const name = this.mtpName(notice)
                switch (notice.state) {
                    case 'mtpDisabled': return this.$t('devices.mtp.disabled')
                    case 'connecting':  return this.$t('devices.mtp.connecting', { name })
                    case 'claimFailed': return this.$t('devices.mtp.claimFailed', { name })
                    case 'busy':        return this.$t('devices.mtp.busy', { name })
                    case 'ejected':     return this.$t('devices.mtp.ejected', { name })
                    // `error`, and any state a newer backend may add.
                    default:            return this.$t('devices.mtp.error', { name })
                }
            },

            /**
             * The backend's message (what to do: quit the app holding the
             * Kindle, install the udev rule…), for the states where something
             * went wrong. The others say all there is to say themselves.
             */
            mtpNoticeDetail(notice: MtpDevice): string | null {
                if (notice.state === 'mtpDisabled' || notice.state === 'connecting' || notice.state === 'ejected') {
                    return null
                }
                return notice.message?.trim() || null
            },

            onSelectDevice(event: Event) {
                void this.deviceStore.selectDevice((event.target as HTMLSelectElement).value)
            },

            /** Safe eject; the store shows the outcome (success or the reason it failed). */
            eject() {
                void this.deviceStore.ejectDevice()
            },

            /** What ⌘K and the menu's "Detect E-reader" do (failures are shown by the store). */
            detect() {
                void this.deviceStore.detectDevice()
            },

            /** Something went wrong with this Kindle (not just waiting or switched off). */
            isMtpProblem(notice: MtpDevice): boolean {
                return notice.state === 'claimFailed' || notice.state === 'busy' || notice.state === 'error'
            },
        },
    })
</script>

<style lang="scss" scoped>
.sidebar {
    width: var(--sidebar-width);
    min-width: var(--sidebar-width);
    display: flex;
    flex-direction: column;
    padding: 22px 0 14px;
    user-select: none;
    position: relative;
    z-index: 1;
    // A pane of glass over the page, edged with the theme's gilt.
    background:
        linear-gradient(180deg, color-mix(in srgb, var(--glass) 100%, transparent), color-mix(in srgb, var(--glass) 70%, transparent));
    backdrop-filter: blur(18px) saturate(130%);
    -webkit-backdrop-filter: blur(18px) saturate(130%);
    border-right: 1px solid var(--border);

    // A gilt hairline, brighter at the middle, along the right edge.
    &::after {
        content: "";
        position: absolute;
        top: 8%;
        bottom: 8%;
        right: -1px;
        width: 1px;
        background: linear-gradient(180deg, transparent, var(--hairline) 30%, var(--hairline) 70%, transparent);
        pointer-events: none;
    }
}

.sidebar_brand {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding-inline: 22px;
    margin-bottom: 26px;
}

.sidebar_rule {
    width: 150px;
    opacity: .9;

    &.-bottom { transform: scaleY(-1); }
}

.sidebar_logo {
    font-size: 25px;
    line-height: 1;
    // Optical centring: the tracking adds space after the last letter.
    padding-left: var(--display-tracking);
    white-space: nowrap;
}

.sidebar_nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 12px;
}

// `:deep`: the Activity item (ActivityPopover) is a nav item too.
.sidebar_nav :deep(.nav-item) {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 12px;
    border-radius: var(--radius-control);
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.01em;
    transition: background var(--transition), color var(--transition), border-color var(--transition);
    border: 1px solid transparent;
    background: transparent;

    > .vue-feather {
        flex-shrink: 0;
        opacity: .8;
        transition: opacity var(--transition), transform var(--duration-slow) var(--ease-lift);
    }
}

.sidebar_nav :deep(.nav-item:hover) {
    background: color-mix(in srgb, var(--text-secondary) 7%, transparent);
    color: var(--text-primary);

    > .vue-feather { opacity: 1; transform: translateX(1px); }
}

// The current view: a tint of the accent, a gilt edge, and a bar of the
// theme's metal inside the left edge, centred on the item.
.sidebar_nav :deep(.nav-item.router-link-active) {
    background: var(--accent-muted);
    color: var(--accent-text); // readable on the tint in every theme (themes.test.ts)
    border-color: color-mix(in srgb, var(--hairline) 80%, transparent);
    box-shadow: inset 0 1px 0 rgba(255,255,255,.04);

    > .vue-feather { opacity: 1; }

    &::before {
        content: "";
        position: absolute;
        left: 4px;
        top: 50%;
        width: 3px;
        height: 16px;
        border-radius: var(--radius-full);
        background: linear-gradient(180deg, var(--accent-strong), var(--accent-dark));
        transform: translateY(-50%);
        box-shadow: 0 0 8px color-mix(in srgb, var(--accent) 55%, transparent);
    }
}

.sidebar_nav :deep(.nav-item_count) {
    margin-left: auto;
    background: color-mix(in srgb, var(--text-secondary) 12%, transparent);
    border-radius: var(--radius-full);
    font-size: var(--font-size-xs);
    padding: 0 7px;
    font-weight: var(--font-weight-semibold);
    color: var(--text-secondary);

    &.-accent {
        background: var(--secondary-muted);
        color: var(--secondary-text);
    }
}

.nav-item_update {
    margin-left: auto;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--success);
    flex-shrink: 0;
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--success) 25%, transparent);
}

// The e-reader: a bookplate, framed in a double gilt rule.
.sidebar_device {
    display: flex;
    flex-direction: column;
    gap: 7px;
    margin: auto 12px 8px;
    padding: 10px 11px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg-elevated) 55%, transparent);
    box-shadow: inset 0 0 0 3px color-mix(in srgb, var(--bg) 40%, transparent), inset 0 0 0 4px var(--border);

    &.-connected {
        border-color: var(--hairline);
        background:
            radial-gradient(120% 80% at 0% 0%, color-mix(in srgb, var(--accent) 10%, transparent), transparent 70%),
            color-mix(in srgb, var(--bg-elevated) 60%, transparent);
    }
}

.device-top {
    display: flex;
    align-items: center;
    gap: 4px;
}

.device-icon {
    color: var(--text-muted);
    flex-shrink: 0;
    margin-right: 3px;
}

.device-name {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    line-height: 1.1;

    // The device's books: a link, underlined on hover like one.
    &.-link {
        border-radius: var(--radius-xs);

        &:hover .device-brand { color: var(--accent-text); text-decoration: underline; text-underline-offset: 2px; }
    }
}

.device-brand {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.device-model {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-regular);
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.device-select {
    width: 100%;
    margin: 6px 0 2px;
    font-size: var(--font-size-xs);
}

.device-space {
    display: flex;
    align-items: center;
    gap: 8px;
}

.device-space-bar {
    flex: 1;
    height: 3px;
    background: var(--border);
    border-radius: var(--radius-full);
    overflow: hidden;
}

.device-space-used {
    height: 100%;
    background: var(--foil);
    border-radius: var(--radius-full);
    transition: width 0.4s ease;
}

.device-space-label {
    font-size: var(--font-size-xs);
    color: var(--text-muted);
    white-space: nowrap;
    flex-shrink: 0;
}

// An MTP Kindle that can't be used (yet): what it is doing, and what to do.
.device-notice {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: var(--font-size-xs);
    line-height: 1.35;
    color: var(--text-secondary);

    &[data-state='claimFailed'],
    &[data-state='busy'],
    &[data-state='error'] {
        .device-notice-text { color: var(--warning); }
    }
}

.device-notice-text {
    display: flex;
    align-items: center;
    gap: 5px;
}

.device-notice-detail {
    font-size: var(--font-size-xs);
    color: var(--text-muted);
    overflow-wrap: anywhere;
}

.device-notice-link {
    font-weight: var(--font-weight-semibold);
    color: var(--accent-text);

    &:hover { text-decoration: underline; }
}

.user-chip {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 12px 8px;
    padding: 7px 10px;
    border-radius: var(--radius-full);
    border: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg-elevated) 45%, transparent);
    cursor: pointer;
    width: calc(100% - 24px);
    transition: background var(--transition);
    min-width: 0;
    font: inherit;

    &:hover {
        background: color-mix(in srgb, var(--bg-elevated) 80%, transparent);
        border-color: var(--border-hover);
    }

    // The user's cover cloth (utils/avatar.ts), lettered in the cover ink,
    // set in a gilt ring like a medallion.
    &_avatar {
        width: 28px;
        height: 28px;
        border-radius: 50%;
        box-shadow: 0 0 0 1.5px var(--bg), 0 0 0 2.5px var(--accent);
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: var(--font-size-xs);
        font-weight: var(--font-weight-bold);
        font-family: var(--font-heading);
        color: var(--cover-ink);
        flex-shrink: 0;
    }

    &_name {
        font-size: var(--font-size-sm);
        font-weight: var(--font-weight-medium);
        color: var(--text-primary);
        flex: 1;
        text-align: left;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    &_icon {
        color: var(--text-muted);
        flex-shrink: 0;
    }
}

.sidebar_bottom {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 10px 4px;
    gap: 8px;
}

.lang-toggle {
    display: flex;
    align-items: center;
    background: color-mix(in srgb, var(--text-secondary) 8%, transparent);
    border-radius: var(--radius-full);
    padding: 2px;
    gap: 1px;

    button {
        background: none;
        border: none;
        cursor: pointer;
        font-size: var(--font-size-xs);
        font-weight: var(--font-weight-semibold);
        letter-spacing: 0.04em;
        color: var(--text-muted);
        padding: 3px 8px;
        border-radius: var(--radius-full);
        transition: background var(--transition), color var(--transition);

        &.active {
            background: var(--accent-strong);
            color: var(--on-accent);
        }

        &:not(.active):hover {
            color: var(--text-primary);
        }
    }
}

.sidebar_about-btn {
    font-size: var(--font-size-xs);
    color: var(--text-muted);
    letter-spacing: 0.05em;
    background: none;
    border: none;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: var(--radius-sm);
    transition: color var(--transition);

    &:hover {
        color: var(--text-secondary);
    }
}
</style>
