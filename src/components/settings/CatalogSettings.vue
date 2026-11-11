<template>
    <SettingsSection id="catalogs">
        <SettingRow :label="$t('settings.opdsCatalogs.title')" :description="$t('settings.opdsCatalogs.description')" stacked>
            <div class="settings-stack">
                <ul v-if="opdsSourcesStore.sources.length > 0" class="catalog-list">
                    <li v-for="source in opdsSourcesStore.sources" :key="source.id" class="catalog-list__item">
                        <vue-feather type="globe" size="14" class="catalog-list__icon" aria-hidden="true" />
                        <span class="catalog-list__text">
                            <span class="catalog-list__name">{{ opdsSourcesStore.displayName(source) }}</span>
                            <span class="catalog-list__url">{{ source.url }}</span>
                        </span>
                        <IconButton
                            icon="trash-2"
                            size="sm"
                            danger
                            :label="$t('settings.opdsCatalogs.remove', { name: opdsSourcesStore.displayName(source) })"
                            @click="pendingRemoval = source"
                        />
                    </li>
                </ul>
                <p v-else class="settings-hint">{{ $t('settings.opdsCatalogs.empty') }}</p>
                <SaveStatus for="catalogs.list" />
            </div>
        </SettingRow>

        <SettingRow :label="$t('settings.opdsCatalogs.addTitle')" stacked>
            <!-- novalidate: the address is checked by addCatalog, which says why inline. -->
            <form class="catalog-form" novalidate @submit.prevent="addCatalog">
                <TextField
                    v-model="url"
                    type="url"
                    required
                    :label="$t('settings.opdsCatalogs.url')"
                    :placeholder="$t('settings.opdsCatalogs.urlPh')"
                    :error="urlError"
                    autocomplete="off"
                    spellcheck="false"
                />
                <TextField
                    v-model="name"
                    :label="$t('settings.opdsCatalogs.name')"
                    :placeholder="$t('settings.opdsCatalogs.namePh')"
                    autocomplete="off"
                />
                <div class="settings-field-row">
                    <TextField
                        v-model="apiKey"
                        class="settings-field"
                        :type="showKey ? 'text' : 'password'"
                        :label="$t('settings.opdsCatalogs.apiKey')"
                        :placeholder="$t('settings.opdsCatalogs.apiKeyPh')"
                        autocomplete="off"
                    />
                    <IconButton
                        :icon="showKey ? 'eye-off' : 'eye'"
                        :label="showKey ? $t('settings.opdsCatalogs.hideKey') : $t('settings.opdsCatalogs.showKey')"
                        :pressed="showKey"
                        @click="showKey = !showKey"
                    />
                </div>
                <div class="settings-actions">
                    <BaseButton type="submit" variant="primary" icon="plus" :disabled="!url.trim()">
                        {{ $t('settings.opdsCatalogs.add') }}
                    </BaseButton>
                </div>
            </form>
        </SettingRow>

        <ConfirmDialog
            v-if="pendingRemoval"
            danger
            :title="$t('settings.opdsCatalogs.removeTitle', { name: opdsSourcesStore.displayName(pendingRemoval) })"
            :message="$t('settings.opdsCatalogs.removeBody')"
            :confirm-label="$t('settings.opdsCatalogs.removeConfirm')"
            @confirm="removeCatalog"
            @cancel="pendingRemoval = null"
        />
    </SettingsSection>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { isCatalogUrl, useOpdsSourcesStore, type OpdsCatalogSource } from '@/stores/opdsSources'
    import { useSaveStatusStore } from '@/stores/saveStatus'
    import { analytics } from '@/services/analytics'
    import SettingRow from '@/components/ui/SettingRow.vue'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import IconButton from '@/components/ui/IconButton.vue'
    import TextField from '@/components/ui/TextField.vue'
    import ConfirmDialog from '@/components/ConfirmDialog.vue'
    import SettingsSection from './SettingsSection.vue'
    import SaveStatus from './SaveStatus.vue'

    /**
     * Settings › Catalogs: the OPDS 2.0 catalogs Discover searches besides
     * Project Gutenberg and Open Library. Removing one is confirmed first;
     * adding one checks that the address is an http(s) URL.
     */
    export default defineComponent({
        name: 'CatalogSettings',

        components: { SettingsSection, SettingRow, BaseButton, IconButton, TextField, ConfirmDialog, SaveStatus },

        data(): {
            url: string
            name: string
            apiKey: string
            showKey: boolean
            urlError: string
            pendingRemoval: OpdsCatalogSource | null
        } {
            return { url: '', name: '', apiKey: '', showKey: false, urlError: '', pendingRemoval: null }
        },

        computed: {
            ...mapStores(useOpdsSourcesStore, useSaveStatusStore),
        },

        watch: {
            url() { this.urlError = '' },
        },

        methods: {
            addCatalog() {
                if (!isCatalogUrl(this.url)) {
                    this.urlError = this.$t('settings.opdsCatalogs.urlInvalid')
                    return
                }
                void this.saveStatusStore.track('catalogs.list', this.opdsSourcesStore.add(this.url, this.name, this.apiKey))
                analytics.track('opds_catalog_add')
                this.url = ''
                this.name = ''
                this.apiKey = ''
                this.showKey = false
            },

            removeCatalog() {
                const source = this.pendingRemoval
                this.pendingRemoval = null
                if (source) { void this.saveStatusStore.track('catalogs.list', this.opdsSourcesStore.remove(source.id)) }
            },
        },
    })
</script>

<style lang="scss" scoped>
.catalog-list {
    margin: 0;
    padding: 0;
    list-style: none;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
}

.catalog-list__item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-3);

    & + & { border-top: 1px solid var(--border); }
}

.catalog-list__icon {
    color: var(--text-muted);
    flex-shrink: 0;
}

.catalog-list__text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
}

.catalog-list__name {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--text-primary);
}

.catalog-list__url {
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.catalog-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    width: 100%;
    max-width: 560px;
}
</style>
