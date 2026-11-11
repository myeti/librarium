<template>
    <div class="settings-view">
        <nav class="settings-nav" :aria-label="$t('settings.navLabel')">
            <h1 class="settings-nav__title">{{ $t('settings.title') }}</h1>
            <ul class="settings-nav__list">
                <li v-for="item in sections" :key="item.id">
                    <RouterLink
                        :to="{ path: '/settings', hash: `#${item.id}` }"
                        replace
                        class="settings-nav__item"
                        :class="{ '-current': item.id === current }"
                        :aria-current="item.id === current ? 'page' : undefined"
                    >
                        <vue-feather :type="item.icon" size="16" aria-hidden="true" />
                        <span>{{ $t(`settings.sections.${item.id}`) }}</span>
                        <span v-if="item.id === 'updates' && updatesStore.hasNewRelease" class="settings-nav__dot" :title="$t('sidebar.newVersion')">
                            <span class="sr-only">{{ $t('sidebar.newVersion') }}</span>
                        </span>
                    </RouterLink>
                </li>
            </ul>
        </nav>

        <div ref="content" class="settings-content">
            <component :is="currentComponent" :key="current" />
        </div>
    </div>
</template>

<script lang="ts">
    import { defineComponent, type Component } from 'vue'
    import { mapStores } from 'pinia'
    import { RouterLink } from 'vue-router'
    import { useUpdatesStore } from '@/stores/updates'
    import { SETTINGS_SECTIONS, sectionFromHash, type SettingsSectionId } from '@/components/settings/sections'
    import LibrarySettings from '@/components/settings/LibrarySettings.vue'
    import DeviceSettings from '@/components/settings/DeviceSettings.vue'
    import MetadataSettings from '@/components/settings/MetadataSettings.vue'
    import OpdsSettings from '@/components/settings/OpdsSettings.vue'
    import CatalogSettings from '@/components/settings/CatalogSettings.vue'
    import AppearanceSettings from '@/components/settings/AppearanceSettings.vue'
    import PrivacySettings from '@/components/settings/PrivacySettings.vue'
    import UpdateSettings from '@/components/settings/UpdateSettings.vue'
    import BackupSettings from '@/components/settings/BackupSettings.vue'
    import AdvancedSettings from '@/components/settings/AdvancedSettings.vue'

    const COMPONENTS: Record<SettingsSectionId, Component> = {
        library: LibrarySettings,
        devices: DeviceSettings,
        metadata: MetadataSettings,
        opds: OpdsSettings,
        catalogs: CatalogSettings,
        appearance: AppearanceSettings,
        privacy: PrivacySettings,
        updates: UpdateSettings,
        backup: BackupSettings,
        advanced: AdvancedSettings,
    }

    /**
     * Settings (`/settings`; `/options` redirects here): a side navigation of
     * sections, and the section shown, one at a time (components/settings).
     *
     * The address's hash is the section (`/settings#devices`), so every
     * section has a deep link and the menu bar's Privacy item opens
     * `#privacy`. A single setting can have its own anchor
     * (`#mtp-setting`, from the sidebar): its section opens, scrolled to it.
     * The navigation's links replace the history entry rather than adding
     * one per section.
     *
     * Settings apply as soon as they change; each row says when it was
     * saved (stores/saveStatus.ts). The shared layout classes of the
     * sections (`.settings-*`) are defined here.
     */
    export default defineComponent({
        name: 'SettingsView',

        components: { RouterLink },

        computed: {
            ...mapStores(useUpdatesStore),

            sections(): typeof SETTINGS_SECTIONS { return SETTINGS_SECTIONS },

            target(): { section: SettingsSectionId, anchor: string | null } {
                return sectionFromHash(this.$route.hash)
            },

            current(): SettingsSectionId { return this.target.section },

            currentComponent(): Component { return COMPONENTS[this.current] },
        },

        watch: {
            async '$route.hash'() {
                await this.$nextTick()
                this.reveal()
            },
        },

        async mounted() {
            await this.$nextTick()
            this.reveal()
        },

        methods: {
            /** Shows the setting the address names, or the top of the section. */
            reveal() {
                const anchor = this.target.anchor
                const element = anchor ? document.getElementById(anchor) : null
                if (element) {
                    element.scrollIntoView({ block: 'center' })
                    return
                }
                const content = this.$refs.content as HTMLElement | undefined
                if (content) { content.scrollTop = 0 }
            },
        },
    })
</script>

<style lang="scss" scoped>
.settings-view {
    display: grid;
    grid-template-columns: 220px minmax(0, 1fr);
    height: 100%;
    overflow: hidden;
}

.settings-nav {
    padding: var(--space-6) var(--space-3) var(--space-5) var(--space-5);
    overflow-y: auto;
}

.settings-nav__title {
    margin: 0 0 var(--space-4) var(--space-3);
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: var(--display-weight);
    letter-spacing: var(--display-tracking);
    text-transform: uppercase;
    line-height: 1.1;
    color: var(--text-primary);
}

.settings-nav__list {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
}

.settings-nav__item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 7px var(--space-3);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--text-secondary);
    transition: background var(--transition), color var(--transition);

    > .vue-feather { flex-shrink: 0; }

    &:hover {
        background: color-mix(in srgb, var(--text-secondary) 8%, transparent);
        color: var(--text-primary);
    }

    &.-current {
        background: color-mix(in srgb, var(--secondary) 14%, transparent);
        border-color: color-mix(in srgb, var(--secondary) 22%, transparent);
        color: var(--secondary-text); // readable on the tint in every theme (themes.test.ts)
    }
}

.settings-nav__dot {
    margin-left: auto;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--success);
}

.settings-content {
    overflow-y: auto;
    padding: var(--space-6) var(--space-6) var(--space-7) var(--space-4);

    > :deep(*) { max-width: 760px; }
}
</style>

<style lang="scss">
// Layout shared by the sections (components/settings/*): global, `settings-` prefixed.
.settings-stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    width: 100%;
}

.settings-actions,
.settings-field-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
}

.settings-field-row { align-items: flex-end; width: 100%; }

.settings-field {
    flex: 1;
    min-width: 200px;
    max-width: 420px;

    &.-narrow input { width: 140px; }
}

.settings-path {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg-input);
}

.settings-path__value {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.settings-hint {
    display: block;
    margin: var(--space-1) 0 0;
    font-size: var(--font-size-xs);
    color: var(--text-secondary);
}

.settings-hints {
    margin: 0;
    padding-left: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--font-size-xs);
    line-height: var(--line-height);
    color: var(--text-secondary);

    &.-boxed {
        padding: var(--space-3) var(--space-4) var(--space-3) var(--space-6);
        border: 1px dashed var(--border);
        border-radius: var(--radius);
    }
}

.settings-warning,
.settings-error {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    font-size: var(--font-size-xs);
    line-height: var(--line-height);

    svg { flex-shrink: 0; margin-top: 1px; }
}

.settings-warning { color: var(--warning); background: var(--warning-muted); }
.settings-error { color: var(--danger); background: var(--danger-muted); }

.settings-copy-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
}

.settings-copy-row__label {
    flex-shrink: 0;
    min-width: 100px;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    color: var(--text-secondary);
}

.settings-copy-row__value {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
</style>
