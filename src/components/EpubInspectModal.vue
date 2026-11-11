<template>
    <BaseModal size="lg" title="EPUB Inspector" close-on-backdrop @close="$emit('close')">
        <EmptyState v-if="loading" loading title="Parsing EPUB…" heading-tag="p" class="inspect-state" />

        <EmptyState
            v-else-if="error"
            alert
            icon="alert-circle"
            :icon-size="28"
            title="Could not read this EPUB"
            :description="error"
            heading-tag="p"
            class="inspect-state"
        />

        <div v-else-if="data" class="inspect-body">
            <!-- OPF location -->
            <section class="inspect-section">
                <h3 class="inspect-section-title">OPF</h3>
                <div class="inspect-kv">
                    <span class="kv-key">Path</span>
                    <code class="kv-val">{{ data.opfPath }}</code>
                </div>
                <div class="inspect-kv">
                    <span class="kv-key">Dir</span>
                    <code class="kv-val">{{ data.opfDir || '(root)' }}</code>
                </div>
            </section>

            <!-- Cover declaration -->
            <section class="inspect-section">
                <h3 class="inspect-section-title">Cover</h3>
                <div v-if="data.coverDeclaration" class="cover-found">
                    <Badge tone="success" icon="check">Found</Badge>
                    <div class="inspect-kv">
                        <span class="kv-key">Method</span>
                        <code class="kv-val">{{ data.coverDeclaration.method }}</code>
                    </div>
                    <div class="inspect-kv">
                        <span class="kv-key">href</span>
                        <code class="kv-val">{{ data.coverDeclaration.href }}</code>
                    </div>
                    <div class="inspect-kv">
                        <span class="kv-key">media-type</span>
                        <code class="kv-val">{{ data.coverDeclaration.mediaType }}</code>
                    </div>
                    <div v-if="data.coverDeclaration.itemId" class="inspect-kv">
                        <span class="kv-key">item-id</span>
                        <code class="kv-val">{{ data.coverDeclaration.itemId }}</code>
                    </div>
                    <!-- Verify the cover file actually exists in the ZIP -->
                    <div class="inspect-kv">
                        <span class="kv-key">File in ZIP</span>
                        <Badge v-if="coverExistsInZip" tone="success" icon="check">Yes</Badge>
                        <Badge v-else tone="danger" icon="x">Missing</Badge>
                    </div>
                </div>
                <div v-else class="cover-missing">
                    <Badge tone="danger" icon="x">Not declared</Badge>
                    <p class="cover-missing-hint">No <code>properties="cover-image"</code> or <code>&lt;meta name="cover"&gt;</code> found in the OPF.</p>
                </div>
            </section>

            <!-- Manifest -->
            <section class="inspect-section">
                <h3 class="inspect-section-title">Manifest <span class="count">({{ data.manifest.length }})</span></h3>
                <div class="inspect-table-wrap">
                    <table class="inspect-table">
                        <thead>
                            <tr>
                                <th scope="col">id</th>
                                <th scope="col">href</th>
                                <th scope="col">media-type</th>
                                <th scope="col">properties</th>
                                <th scope="col">in ZIP</th>
                            </tr>
                        </thead>
                        <tbody>
                            <tr
                                v-for="item in data.manifest"
                                :key="item.id"
                                :class="{ 'row-cover': item.properties?.includes('cover-image'), 'row-missing': !item.exists }"
                            >
                                <td><code>{{ item.id }}</code></td>
                                <td><code>{{ item.href }}</code></td>
                                <td><code class="mime">{{ item.mediaType }}</code></td>
                                <td><code v-if="item.properties" class="props">{{ item.properties }}</code></td>
                                <td>
                                    <!-- Icon + text: the state isn't told by colour alone. -->
                                    <span class="zip-state" :class="item.exists ? '-ok' : '-missing'" :title="item.zipPath">
                                        <vue-feather :type="item.exists ? 'check' : 'x'" size="12" aria-hidden="true" />
                                        {{ item.exists ? 'yes' : 'missing' }}
                                    </span>
                                </td>
                            </tr>
                        </tbody>
                    </table>
                </div>
            </section>

            <!-- Spine -->
            <section class="inspect-section">
                <h3 class="inspect-section-title">Spine <span class="count">({{ data.spine.length }})</span></h3>
                <div class="spine-list">
                    <code v-for="(id, i) in data.spine" :key="i" class="spine-item">{{ id }}</code>
                </div>
            </section>

            <!-- All ZIP entries -->
            <section class="inspect-section">
                <h3 class="inspect-section-title">ZIP entries <span class="count">({{ data.zipEntries.length }})</span></h3>
                <div class="inspect-table-wrap">
                    <table class="inspect-table">
                        <thead>
                            <tr><th scope="col">name</th><th scope="col">size</th></tr>
                        </thead>
                        <tbody>
                            <tr v-for="entry in data.zipEntries" :key="entry.name">
                                <td><code>{{ entry.name }}</code></td>
                                <td class="size-col">{{ fmtSize(entry.size) }}</td>
                            </tr>
                        </tbody>
                    </table>
                </div>
            </section>
        </div>
    </BaseModal>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import BaseModal from '@/components/ui/BaseModal.vue'
    import Badge from '@/components/ui/Badge.vue'
    import EmptyState from '@/components/ui/EmptyState.vue'
    import * as api from '@/services/api'
    import { fmtSize } from '@/utils/format'
    import { errorMessage } from '@/utils/errors'
    import type { EpubStructure } from '@/types'

    /**
     * Developer tool (dev builds only, from the detail panel's ⋮ menu): the
     * structure of an EPUB as the backend reads it — OPF, cover declaration,
     * manifest, spine and ZIP entries. Its labels are EPUB vocabulary and
     * stay in English on purpose: it is not part of the translated UI.
     */
    export default defineComponent({
        name: 'EpubInspectModal',

        components: { BaseModal, Badge, EmptyState },

        props: {
            bookPath: { type: String, required: true },
        },

        emits: ['close'],

        data() {
            return {
                loading: true,
                error: null as string | null,
                data: null as EpubStructure | null,
            }
        },

        computed: {
            coverExistsInZip(): boolean {
                if (!this.data?.coverDeclaration) { return false }
                const href = this.data.coverDeclaration.href
                return this.data.manifest.find(m => m.href === href)?.exists ?? false
            },
        },

        async mounted() {
            try {
                this.data = await api.inspectEpub(this.bookPath)
            } catch (e) {
                this.error = errorMessage(e)
            } finally {
                this.loading = false
            }
        },

        methods: {
            fmtSize,
        },
    })
</script>

<style lang="scss" scoped>
.inspect-state { min-height: 200px; }

.inspect-body {
    display: flex;
    flex-direction: column;
    margin-top: calc(-1 * var(--space-4));
}

.inspect-section {
    padding: 18px 0;
    border-bottom: 1px solid var(--border);

    &:last-child { border-bottom: none; padding-bottom: 0; }
}

.inspect-section-title {
    margin-bottom: var(--space-3);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-bold);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-muted);

    .count { font-weight: var(--font-weight-regular); }
}

.inspect-kv {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    margin-bottom: 6px;
}

.kv-key {
    width: 80px;
    flex-shrink: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    color: var(--text-secondary);
}

.kv-val {
    font-size: var(--font-size-xs);
    color: var(--text-primary);
    word-break: break-all;
}

code {
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
}

.cover-found { display: flex; flex-direction: column; align-items: flex-start; gap: var(--space-1); }
.cover-missing { display: flex; flex-direction: column; align-items: flex-start; gap: var(--space-2); }
.cover-missing-hint {
    font-size: var(--font-size-xs);
    color: var(--text-muted);
}

.inspect-table-wrap {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
}

.inspect-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--font-size-xs);

    th {
        padding: 6px 10px;
        border-bottom: 1px solid var(--border);
        background: var(--bg);
        text-align: left;
        font-size: var(--font-size-xs);
        font-weight: var(--font-weight-bold);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        color: var(--text-muted);
    }

    td {
        padding: 6px 10px;
        border-bottom: 1px solid var(--border);
        vertical-align: middle;
        color: var(--text-primary);
    }

    tr:last-child td { border-bottom: none; }

    tr.row-cover td { background: var(--success-muted); }
    tr.row-missing td { background: var(--danger-muted); }

    .mime  { color: var(--text-secondary); }
    .props { color: var(--accent-text); }
    .size-col { text-align: right; white-space: nowrap; color: var(--text-muted); }
}

.zip-state {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    white-space: nowrap;

    &.-ok      { color: var(--success); }
    &.-missing { color: var(--danger); font-weight: var(--font-weight-semibold); }
}

.spine-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
}

.spine-item {
    padding: 3px var(--space-2);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    font-size: var(--font-size-xs);
    color: var(--text-secondary);
}
</style>
