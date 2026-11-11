import { defineStore } from 'pinia'
import { saveConfig, saveConfigOrThrow } from '@/services/appConfig'

export interface OpdsCatalogSource {
    id:      string
    url:     string
    name:    string
    apiKey:  string
}

function hostnameFromUrl(url: string): string {
    try { return new URL(url).hostname } catch { return url }
}

/** Whether `url` can be a catalog address: an absolute http(s) URL. */
export function isCatalogUrl(url: string): boolean {
    try {
        const parsed = new URL(url.trim())
        return (parsed.protocol === 'http:' || parsed.protocol === 'https:') && parsed.hostname !== ''
    } catch {
        return false
    }
}

export const useOpdsSourcesStore = defineStore('opdsSources', {
    state: () => ({
        sources: [] as OpdsCatalogSource[],
    }),

    getters: {
        displayName: () => (source: OpdsCatalogSource) =>
            source.name.trim() || hostnameFromUrl(source.url),
    },

    actions: {
        initFromConfig(catalogs: OpdsCatalogSource[]) {
            this.sources = catalogs
        },

        /**
         * Adds a catalog (Discover searches it at once). Resolves once saved;
         * rejects when it couldn't be saved (it is still used this session).
         */
        add(url: string, name: string, apiKey: string): Promise<void> {
            this.sources.push({ id: crypto.randomUUID(), url: url.trim(), name: name.trim(), apiKey: apiKey.trim() })
            return saveConfigOrThrow({ opdsCatalogs: this.sources })
        },

        /** Removes a catalog; same promise as `add`. */
        remove(id: string): Promise<void> {
            this.sources = this.sources.filter(s => s.id !== id)
            return saveConfigOrThrow({ opdsCatalogs: this.sources })
        },

        update(id: string, fields: Partial<Omit<OpdsCatalogSource, 'id'>>) {
            const idx = this.sources.findIndex(s => s.id === id)
            if (idx !== -1) {
                this.sources[idx] = { ...this.sources[idx], ...fields }
                void saveConfig({ opdsCatalogs: this.sources })
            }
        },
    },
})
