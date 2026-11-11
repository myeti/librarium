import { describe, expect, it, vi } from 'vitest'
import { createMemoryHistory, createRouter } from 'vue-router'

vi.mock('@tauri-apps/api/core', () => import('@/test/tauri'))
import { routes } from './index'
import { sectionFromHash } from '@/components/settings/sections'

async function resolve(path: string) {
    const router = createRouter({ history: createMemoryHistory(), routes })
    await router.push(path)
    return router.currentRoute.value
}

describe('routes', () => {
    it('keeps the former addresses working', async () => {
        expect((await resolve('/options')).fullPath).toBe('/settings')
        const mtp = await resolve('/options#mtp-setting')
        expect([mtp.path, mtp.hash]).toEqual(['/settings', '#mtp-setting'])
        expect((await resolve('/store')).path).toBe('/discover')
        // The Transfers page became the sidebar's Activity popover.
        expect((await resolve('/transfers')).path).toBe('/library')
    })

    it('names the screens', async () => {
        expect((await resolve('/settings#devices')).name).toBe('settings')
        expect((await resolve('/discover')).name).toBe('discover')
    })
})

describe('settings deep links', () => {
    it('open a section, or the section holding a setting', () => {
        expect(sectionFromHash('#devices')).toEqual({ section: 'devices', anchor: null })
        expect(sectionFromHash('#mtp-setting')).toEqual({ section: 'devices', anchor: 'mtp-setting' })
        expect(sectionFromHash('#privacy')).toEqual({ section: 'privacy', anchor: null })
        expect(sectionFromHash('')).toEqual({ section: 'library', anchor: null })
        expect(sectionFromHash('#nonsense')).toEqual({ section: 'library', anchor: null })
    })
})
