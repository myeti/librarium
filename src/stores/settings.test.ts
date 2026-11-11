import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { backendError, mockInvoke } from '@/test/tauri'
import { stubMatchMedia } from '@/test/matchMedia'
import { REDUCED_MOTION_QUERY, useSystemStore } from './system'
import { useSettingsStore } from './settings'
import type { AppConfig } from '@/services/appConfig'

describe('settings store: background effects', () => {
    let media: ReturnType<typeof stubMatchMedia>

    function settingsWith(config: AppConfig, reducedMotion: boolean) {
        media = stubMatchMedia({ [REDUCED_MOTION_QUERY]: reducedMotion })
        setActivePinia(createPinia())
        const settings = useSettingsStore()
        settings.initFromConfig(config)
        return settings
    }

    beforeEach(() => {
        mockInvoke.reset()
        mockInvoke.on('set_config', () => undefined)
    })

    afterEach(() => {
        useSystemStore().uninstall()
        vi.unstubAllGlobals()
    })

    it('default to Subtle', () => {
        expect(settingsWith({}, false).effectsLevel).toBe('subtle')
    })

    it('default to Off when the system asks to reduce motion, and follow it live', () => {
        const settings = settingsWith({}, true)
        expect(settings.effectsLevel).toBe('off')
        useSystemStore().install()
        media.set(REDUCED_MOTION_QUERY, false)
        expect(settings.effectsLevel).toBe('subtle')
    })

    it('use the user’s choice once there is one, whatever the system says', async () => {
        const settings = settingsWith({}, true)
        await settings.setEffects('on')
        expect(settings.effectsLevel).toBe('on')
        expect(mockInvoke.callsTo('set_config').map(c => c.args)).toEqual([{ updates: { effects: 'on' } }])
        expect(settingsWith({ effects: 'subtle' }, true).effectsLevel).toBe('subtle')
    })

    it('ignore a saved value this version doesn’t know', () => {
        expect(settingsWith({ effects: 'max' }, false).effectsLevel).toBe('subtle')
    })
})

describe('settings store: sounds and saving', () => {
    beforeEach(() => {
        setActivePinia(createPinia())
        mockInvoke.reset()
    })

    it('keeps the launch sound off unless it was turned on', () => {
        const settings = useSettingsStore()
        settings.initFromConfig({})
        expect(settings.launchSound).toBe(false)
        settings.initFromConfig({ launchSound: true })
        expect(settings.launchSound).toBe(true)
    })

    it('clamps the volume, and ignores a damaged saved one', () => {
        const settings = useSettingsStore()
        settings.initFromConfig({ soundVolume: Number.NaN })
        expect(settings.soundVolume).toBe(0.8)
        settings.initFromConfig({ soundVolume: 3 })
        expect(settings.soundVolume).toBe(1)
    })

    it('applies a setting at once, and rejects when it couldn’t be saved', async () => {
        mockInvoke.on('set_config', () => backendError({ code: 'IO', message: 'read-only volume' }))
        const settings = useSettingsStore()
        await expect(settings.setDeviceSubfolder('Books')).rejects.toMatchObject({ message: 'read-only volume' })
        expect(settings.deviceSubfolder).toBe('Books')
    })
})

describe('settings store: update check and news', () => {
    beforeEach(() => {
        setActivePinia(createPinia())
        mockInvoke.reset()
        mockInvoke.on('set_config', () => undefined)
    })

    it('are on unless turned off', () => {
        const settings = useSettingsStore()
        settings.initFromConfig({})
        expect([settings.autoCheckUpdates, settings.showNews]).toEqual([true, true])
        settings.initFromConfig({ autoCheckUpdates: false, showNews: false })
        expect([settings.autoCheckUpdates, settings.showNews]).toEqual([false, false])
    })

    it('are saved in the settings file', async () => {
        const settings = useSettingsStore()
        await settings.setAutoCheckUpdates(false)
        await settings.setShowNews(false)
        expect(mockInvoke.callsTo('set_config').map(c => c.args)).toEqual([
            { updates: { autoCheckUpdates: false } },
            { updates: { showNews: false } },
        ])
    })
})
