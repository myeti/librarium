<template>
    <SettingsSection id="metadata">
        <SettingRow :label="$t('settings.metadataSources.title')" :description="$t('settings.metadataSources.description')" stacked>
            <ol class="metadata-sources">
                <li v-for="source in sources" :key="source.id" class="metadata-sources__item" :class="{ '-off': !source.used }">
                    <span class="metadata-sources__name">{{ source.name }}</span>
                    <Badge :tone="source.used ? 'success' : 'neutral'" :icon="source.used ? 'check' : 'slash'">{{ source.status }}</Badge>
                </li>
            </ol>
        </SettingRow>

        <SettingRow id="google-books-key" :label="$t('settings.googleBooks.title')" :description="$t('settings.googleBooks.description')" stacked>
            <div class="settings-field-row">
                <TextField
                    v-model="googleBooksKey"
                    class="settings-field"
                    :type="showKey ? 'text' : 'password'"
                    :label="$t('settings.googleBooks.label')"
                    :placeholder="$t('settings.googleBooks.placeholder')"
                    autocomplete="off"
                    spellcheck="false"
                    @blur="saveKey"
                    @keydown.enter.prevent="saveKey"
                />
                <IconButton
                    :icon="showKey ? 'eye-off' : 'eye'"
                    :label="showKey ? $t('settings.googleBooks.hide') : $t('settings.googleBooks.show')"
                    :pressed="showKey"
                    @click="showKey = !showKey"
                />
                <IconButton v-if="googleBooksKey" icon="x" :label="$t('settings.googleBooks.clear')" @click="clearKey" />
                <SaveStatus for="metadata.googleBooksKey" />
            </div>
        </SettingRow>
    </SettingsSection>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { useSettingsStore } from '@/stores/settings'
    import { useSaveStatusStore } from '@/stores/saveStatus'
    import { tidyupSourceLabel } from '@/services/tidyup'
    import SettingRow from '@/components/ui/SettingRow.vue'
    import TextField from '@/components/ui/TextField.vue'
    import IconButton from '@/components/ui/IconButton.vue'
    import Badge from '@/components/ui/Badge.vue'
    import SettingsSection from './SettingsSection.vue'
    import SaveStatus from './SaveStatus.vue'

    interface SourceRow {
        id: string
        name: string
        used: boolean
        status: string
    }

    /**
     * Settings › Metadata: where "Find metadata online" looks books up, in
     * the order it asks (Google Books first when it has a key, then Open
     * Library), and the Google Books API key (saved on blur or Enter, shown
     * on demand).
     */
    export default defineComponent({
        name: 'MetadataSettings',

        components: { SettingsSection, SettingRow, TextField, IconButton, Badge, SaveStatus },

        data() {
            return { googleBooksKey: '', showKey: false }
        },

        computed: {
            ...mapStores(useSettingsStore, useSaveStatusStore),

            sources(): SourceRow[] {
                const hasKey = !!this.settingsStore.googleBooksApiKey
                return [
                    {
                        id: 'googlebooks',
                        name: tidyupSourceLabel('googlebooks'),
                        used: hasKey,
                        status: hasKey ? this.$t('settings.metadataSources.googleBooksOn') : this.$t('settings.metadataSources.googleBooksOff'),
                    },
                    {
                        id: 'openlibrary',
                        name: tidyupSourceLabel('openlibrary'),
                        used: true,
                        status: this.$t('settings.metadataSources.openLibrary'),
                    },
                ]
            },
        },

        created() {
            this.googleBooksKey = this.settingsStore.googleBooksApiKey
        },

        methods: {
            saveKey() {
                const key = this.googleBooksKey.trim()
                this.googleBooksKey = key
                if (key === this.settingsStore.googleBooksApiKey) { return }
                void this.saveStatusStore.track('metadata.googleBooksKey', this.settingsStore.setGoogleBooksApiKey(key))
            },

            clearKey() {
                this.googleBooksKey = ''
                this.saveKey()
            },
        },
    })
</script>

<style lang="scss" scoped>
.metadata-sources {
    margin: 0;
    padding: 0;
    list-style: none;
    counter-reset: source;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
}

.metadata-sources__item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    counter-increment: source;

    &::before {
        content: counter(source) '.';
        min-width: 1.2em;
        font-size: var(--font-size-sm);
        color: var(--text-muted);
    }
}

.metadata-sources__name {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--text-primary);

    .-off & { color: var(--text-secondary); }
}
</style>
