<template>
    <BaseModal
        bare
        :width="520"
        :aria-label="$t('about.title')"
        close-on-backdrop
        @close="$emit('close')"
    >
        <div class="about-card">
            <IconButton
                icon="x"
                :label="$t('common.close')"
                class="about-close"
                @click="$emit('close')"
            />

            <!-- eslint-disable-next-line vue/no-v-html -- bundled SVG asset (assets/icons), not user data -->
            <span class="about-icon" aria-hidden="true" v-html="libraryIcon"></span>
            <h2 class="about-name">LIBRARIUM</h2>

            <!-- Version / build / author info -->
            <div class="about-meta">
                <span class="about-version">v{{ appVersion }} / {{ buildDate }}</span>
                <span class="about-exlibris">ex libris <strong>Magalon</strong></span>
            </div>

            <!-- Supported formats -->
            <section class="about-section">
                <h3 class="about-heading">{{ $t('sidebar.supportedFormats') }}</h3>
                <div class="format-list">
                    <Badge v-for="fmt in FORMATS" :key="fmt" :tone="fmt">{{ fmt.toUpperCase() }}</Badge>
                </div>
            </section>

            <!-- Compatible devices -->
            <section class="about-section">
                <h3 class="about-heading">{{ $t('sidebar.compatibleDevices') }}</h3>
                <div class="device-columns">
                    <div v-for="group in DEVICE_GROUPS" :key="group.brand" class="device-col">
                        <h4 class="device-col-brand">{{ group.brand }}</h4>
                        <ul class="device-list">
                            <li v-for="d in group.devices" :key="d.model" class="device-row">
                                <span class="device-row-model">{{ d.model }}</span>
                                <Badge :tone="d.format">{{ d.format.toUpperCase() }}</Badge>
                            </li>
                        </ul>
                        <!-- 2024 Kindles: MTP, experimental, on by default (COMPATIBILITY.md). -->
                        <template v-if="group.mtpDevices">
                            <h5 class="device-col-sub">{{ $t('about.mtpHeading') }}</h5>
                            <ul class="device-list -mtp">
                                <li v-for="d in group.mtpDevices" :key="d.model" class="device-row">
                                    <span class="device-row-model">{{ d.model }}</span>
                                    <Badge :tone="d.format">{{ d.format.toUpperCase() }}</Badge>
                                </li>
                            </ul>
                        </template>
                    </div>
                </div>
                <p class="device-note">{{ $t('about.mtpNote') }}</p>
            </section>

            <!-- Third-party credits -->
            <section class="about-section">
                <h3 class="about-heading">{{ $t('sidebar.credits') }}</h3>
                <ul class="credit-list">
                    <li v-for="credit in CREDITS" :key="credit.name" class="credit-row">
                        <span class="credit-name">
                            <vue-feather v-if="credit.kind" :type="CREDIT_ICONS[credit.kind]" size="12" class="credit-icon" aria-hidden="true" />
                            {{ credit.name }}
                        </span>
                        <span class="credit-right">
                            <span v-if="credit.license" class="credit-license">{{ credit.license }}</span>
                            <span v-if="credit.by" class="credit-by">{{ credit.by }}</span>
                        </span>
                    </li>
                </ul>
                <p class="credit-more">{{ $t('about.moreCredits') }}</p>
            </section>
        </div>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import IconButton from '@/components/ui/IconButton.vue'
    import Badge from '@/components/ui/Badge.vue'
    import { version as appVersion } from '../../package.json'
    import libraryIconSvg from '@/assets/icons/library.svg?raw'

    type DeviceEntry = { model: string; format: 'epub' | 'azw3' }

    /**
     * The e-readers listed as compatible; keep in step with COMPATIBILITY.md.
     * `devices` connect as a USB drive. `mtpDevices` are the Kindles from
     * 2024 on, which use MTP: their support is experimental and off by
     * default (Settings › Devices), so they are listed apart.
     */
    const DEVICE_GROUPS: Array<{ brand: string; devices: DeviceEntry[]; mtpDevices?: DeviceEntry[] }> = [
        {
            brand: 'Kindle',
            devices: [
                { model: 'Paperwhite 3 — 7th gen',         format: 'azw3' },
                { model: 'Paperwhite 4 — 10th gen',        format: 'azw3' },
                { model: 'Paperwhite 5 — 11th gen',        format: 'azw3' },
                { model: 'Paperwhite SE — 11th gen',       format: 'azw3' },
                { model: 'Oasis 2 — 9th gen',              format: 'azw3' },
                { model: 'Oasis 3 — 10th gen',             format: 'azw3' },
                { model: 'Kindle — 10th gen',              format: 'azw3' },
                { model: 'Kindle — 11th gen',              format: 'azw3' },
                { model: 'Scribe — 1st gen',               format: 'azw3' },
            ],
            mtpDevices: [
                { model: 'Paperwhite — 12th gen',          format: 'azw3' },
                { model: 'Colorsoft',                      format: 'azw3' },
                { model: 'Kindle — 2024',                  format: 'azw3' },
                { model: 'Scribe — 2024',                  format: 'azw3' },
            ],
        },
        {
            brand: 'Kobo',
            devices: [
                { model: 'Clara HD',      format: 'epub' },
                { model: 'Clara 2E',      format: 'epub' },
                { model: 'Clara Colour',  format: 'epub' },
                { model: 'Libra H2O',     format: 'epub' },
                { model: 'Libra 2',       format: 'epub' },
                { model: 'Libra Colour',  format: 'epub' },
                { model: 'Forma',         format: 'epub' },
                { model: 'Sage',          format: 'epub' },
                { model: 'Elipsa',        format: 'epub' },
                { model: 'Elipsa 2E',     format: 'epub' },
                { model: 'Nia',           format: 'epub' },
            ],
        },
    ]

    type CreditKind = 'audio' | 'photo' | 'font'

    const CREDIT_ICONS: Record<CreditKind, string> = { audio: 'volume-2', photo: 'camera', font: 'type' }

    /**
     * The main third-party libraries and assets, with their licences. The
     * complete list (every Rust crate and npm package in the app) is in
     * THIRD_PARTY_NOTICES.md, generated by `npm run notices`.
     */
    const CREDITS: Array<{ name: string; license: string; by: string; kind?: CreditKind }> = [
        { name: 'Feather Icons',        license: 'MIT',              by: 'Cole Bemis' },
        { name: 'epub.js',              license: 'BSD-2-Clause',     by: 'FuturePress' },
        { name: 'boko',                 license: 'GPL-3.0-or-later', by: '' },
        { name: 'lopdf',                license: 'MIT',              by: 'Junfeng Liu' },
        { name: 'quick-xml',            license: 'MIT',              by: 'Johann Tuffe & contributors' },
        { name: 'zip',                  license: 'MIT',              by: 'zip-rs contributors' },
        { name: 'axum',                 license: 'MIT',              by: 'Tokio Contributors' },
        { name: 'whatlang',             license: 'MIT',              by: 'Serhii Potapov' },
        { name: 'Vue.js',               license: 'MIT',              by: 'Evan You' },
        { name: 'Pinia',                license: 'MIT',              by: 'Eduardo San Martin Morote' },
        { name: 'Tauri',                license: 'MIT / Apache-2.0', by: 'Tauri Contributors' },
        { name: 'Inter',                license: 'OFL-1.1',          by: 'Rasmus Andersson', kind: 'font' },
        { name: 'Cinzel',               license: 'OFL-1.1',          by: 'Natanael Gama', kind: 'font' },
        { name: 'Poiret One',           license: 'OFL-1.1',          by: 'Denis Masharov', kind: 'font' },
        { name: 'Fraunces',             license: 'OFL-1.1',          by: 'Undercase Type', kind: 'font' },
        { name: 'Italiana',             license: 'OFL-1.1',          by: 'Santiago Orozco', kind: 'font' },
        { name: 'Cormorant Garamond',   license: 'OFL-1.1',          by: 'Christian Thalmann', kind: 'font' },
        { name: 'Reading background',   license: 'Unsplash',         by: 'Pavan Trikutam', kind: 'photo' },
        { name: 'Sound effects',        license: '',                 by: 'ZapSplat', kind: 'audio' },
    ]

    /**
     * About Librarium: version, supported formats and devices, credits.
     * Nothing to lose here, so a click outside closes it too.
     */
    export default defineComponent({
        name: 'AboutModal',

        components: { BaseModal, IconButton, Badge },

        emits: ['close'],

        data() {
            return {
                libraryIcon: libraryIconSvg,
                buildDate: __BUILD_DATE__,
                appVersion,
                CREDITS,
                CREDIT_ICONS,
                DEVICE_GROUPS,
                FORMATS: ['epub', 'pdf', 'mobi', 'azw3', 'cbz'] as const,
            }
        },
    })
</script>

<style lang="scss" scoped>
.about-card {
    position: relative;
    padding: 44px 52px 40px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
}

.about-close {
    position: absolute;
    top: 12px;
    right: 12px;
}

.about-icon {
    margin-bottom: var(--space-2);
    filter: drop-shadow(0 4px 12px color-mix(in srgb, var(--accent) 35%, transparent));

    :deep(svg) {
        width: 64px;
        height: auto;
        display: block;
        border-radius: 14px;
    }
}

.about-name {
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: var(--display-weight);
    letter-spacing: var(--display-tracking);
    text-transform: uppercase;
    color: var(--text-primary);
}

.about-meta {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    margin-bottom: var(--space-1);
}

.about-version {
    font-size: var(--font-size-xs);
    color: var(--text-muted);
    letter-spacing: 0.06em;
}

.about-exlibris {
    font-size: var(--font-size-sm);
    color: var(--text-secondary);
    letter-spacing: 0.01em;

    strong {
        font-weight: var(--font-weight-bold);
        color: var(--text-primary);
    }
}

.about-section {
    width: 100%;
    margin-top: 20px;
}

.about-heading {
    display: block;
    margin-bottom: var(--space-2);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-bold);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-muted);
}

.format-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
}

.device-columns {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0 20px;
}

.device-col-brand {
    display: block;
    margin-bottom: var(--space-1);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-bold);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-secondary);
}

.device-list,
.credit-list {
    margin: 0;
    padding: 0;
    list-style: none;
}

.device-col-sub {
    margin: var(--space-3) 0 var(--space-1);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    color: var(--warning);
}

.device-note {
    margin: var(--space-2) 0 0;
    font-size: var(--font-size-xs);
    line-height: var(--line-height);
    color: var(--text-secondary);
}

.device-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: var(--space-1) 0;
    border-bottom: 1px solid var(--border);

    &:last-child { border-bottom: none; }
}

.device-row-model {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--font-size-xs);
    color: var(--text-primary);
}

.credit-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
    padding: 6px 0;
    border-bottom: 1px solid var(--border);

    &:last-child { border-bottom: none; }
}

.credit-name {
    display: flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
}

.credit-icon {
    flex-shrink: 0;
    color: var(--accent-text);
}

.credit-right {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-2);
}

.credit-license {
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: color-mix(in srgb, var(--text-secondary) 6%, transparent);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.03em;
    white-space: nowrap;
    color: var(--text-muted);
}

.credit-by {
    font-size: var(--font-size-xs);
    white-space: nowrap;
    color: var(--text-secondary);
}

.credit-more {
    margin-top: var(--space-3);
    font-size: var(--font-size-xs);
    color: var(--text-muted);
    text-align: center;
}
</style>
