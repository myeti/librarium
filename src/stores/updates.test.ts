import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const { check, relaunch, fetchMessages } = vi.hoisted(() => ({ check: vi.fn(), relaunch: vi.fn(), fetchMessages: vi.fn() }))
vi.mock('@tauri-apps/plugin-updater', () => ({ check }))
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch }))
vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
vi.mock('@/services/librarium-api', () => ({ fetchMessages }))

import { useUpdatesStore } from './updates'
import { useNotificationsStore } from './notifications'
import { useSettingsStore } from './settings'

/**
 * Like the real `Update` (a Tauri `Resource`): its resource id lives in a
 * private class field, which only the object itself can read — not a Vue
 * proxy wrapped around it. Calling a method through such a proxy throws.
 */
class FakeUpdate {
    readonly #rid: number
    readonly available = true
    readonly version = '0.6.0'
    readonly date = '2026-11-06'
    constructor(rid: number, readonly body: string, private readonly install: () => Promise<void>) { this.#rid = rid }
    get rid(): number { return this.#rid }
    downloadAndInstall(): Promise<void> {
        return this.rid > 0 ? this.install() : Promise.reject(new Error('no rid'))
    }
}

beforeEach(() => {
    setActivePinia(createPinia())
    check.mockReset()
    relaunch.mockReset()
    relaunch.mockResolvedValue(undefined)
    fetchMessages.mockReset()
    fetchMessages.mockResolvedValue([])
})

async function storeWithUpdate(install: () => Promise<void> = () => Promise.resolve()) {
    check.mockResolvedValue(new FakeUpdate(7, '  Faster scans.\n\nFixes the reader.  ', install))
    const store = useUpdatesStore()
    await store.checkReleases()
    return store
}

describe('updates store', () => {
    it('offers the release with its real notes', async () => {
        const store = await storeWithUpdate()
        expect(store.hasNewRelease).toBe(true)
        expect(store.newRelease).toMatchObject({ version: '0.6.0', changelog: 'Faster scans.\n\nFixes the reader.' })
    })

    it('says nothing when the update server is unreachable', async () => {
        vi.spyOn(console, 'warn').mockImplementation(() => {})
        check.mockRejectedValue(new Error('offline'))
        const store = useUpdatesStore()
        await store.checkReleases()
        expect(store.hasNewRelease).toBe(false)
        expect(useNotificationsStore().items).toEqual([])
    })

    it('installs, then relaunches into the new version', async () => {
        const install = vi.fn(() => Promise.resolve())
        const store = await storeWithUpdate(install)
        await store.downloadAndInstall()
        // Called on the object itself, not through a reactive proxy (which would throw).
        expect(install).toHaveBeenCalledTimes(1)
        expect(relaunch).toHaveBeenCalledTimes(1)
        expect(store.installError).toBeNull()
    })

    it('shows a failed install and lets the user try again', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {})
        const store = await storeWithUpdate(() => Promise.reject(new Error('signature mismatch')))
        await store.downloadAndInstall()
        expect(relaunch).not.toHaveBeenCalled()
        expect(store.installing).toBe(false)
        expect(store.installError).toContain('signature mismatch')
        const toasts = useNotificationsStore().items
        expect(toasts.map(n => n.type)).toEqual(['error'])
        expect(toasts[0].message).toContain('signature mismatch')
    })

    it('says so when the update is installed but the restart failed', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {})
        relaunch.mockRejectedValue(new Error('not allowed'))
        const store = await storeWithUpdate()
        await store.downloadAndInstall()
        expect(store.installing).toBe(false)
        expect(store.installError).toContain('restart')
    })

    it('ignores a second click while installing', async () => {
        let finish!: () => void
        const install = vi.fn(() => new Promise<void>(resolve => { finish = resolve }))
        const store = await storeWithUpdate(install)
        const first = store.downloadAndInstall()
        await store.downloadAndInstall()
        expect(install).toHaveBeenCalledTimes(1)
        expect(store.installing).toBe(true)
        finish()
        await first
    })
})

describe('checks at startup', () => {
    const news = { id: 4, translations: { en: { title: 'Hi', content: 'News' } }, constraintOs: null, constraintVersion: null, createdAt: '2026-11-01' }

    it('checks for a new version and fetches the news by default', async () => {
        check.mockResolvedValue(null)
        fetchMessages.mockResolvedValue([news])
        useSettingsStore().initFromConfig({})
        const store = useUpdatesStore()
        await store.checkAtStartup()
        expect(check).toHaveBeenCalledTimes(1)
        expect(fetchMessages).toHaveBeenCalledTimes(1)
        expect(store.pendingMessages).toEqual([news])
    })

    it('contacts neither GitHub nor the Librarium server when both are turned off', async () => {
        useSettingsStore().initFromConfig({ autoCheckUpdates: false, showNews: false })
        await useUpdatesStore().checkAtStartup()
        expect(check).not.toHaveBeenCalled()
        expect(fetchMessages).not.toHaveBeenCalled()
    })

    it('honours each setting on its own', async () => {
        check.mockResolvedValue(null)
        const settings = useSettingsStore()
        settings.initFromConfig({ autoCheckUpdates: false })
        await useUpdatesStore().checkAtStartup()
        expect(check).not.toHaveBeenCalled()
        expect(fetchMessages).toHaveBeenCalledTimes(1)

        fetchMessages.mockClear()
        settings.initFromConfig({ showNews: false })
        await useUpdatesStore().checkAtStartup()
        expect(check).toHaveBeenCalledTimes(1)
        expect(fetchMessages).not.toHaveBeenCalled()
    })

    it('still checks when asked to ("Check now"), whatever the setting', async () => {
        check.mockResolvedValue(null)
        useSettingsStore().initFromConfig({ autoCheckUpdates: false })
        await useUpdatesStore().checkReleases()
        expect(check).toHaveBeenCalledTimes(1)
    })
})
