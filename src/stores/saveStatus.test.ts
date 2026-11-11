import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { SAVED_VISIBLE_MS, useSaveStatusStore } from './saveStatus'

beforeEach(() => {
    setActivePinia(createPinia())
    vi.useFakeTimers()
    vi.spyOn(console, 'error').mockImplementation(() => {})
})

afterEach(() => { vi.useRealTimers() })

describe('save status', () => {
    it('says "saved" for a moment after a save', async () => {
        const status = useSaveStatusStore()
        await expect(status.track('devices.subfolder', Promise.resolve())).resolves.toBe(true)
        expect(status.statusOf('devices.subfolder')).toEqual({ state: 'saved', error: '' })
        vi.advanceTimersByTime(SAVED_VISIBLE_MS)
        expect(status.statusOf('devices.subfolder')).toBeNull()
    })

    it('says why until the next attempt when it failed (and never rejects)', async () => {
        const status = useSaveStatusStore()
        const saved = await status.track('opds.port', () => Promise.reject(new Error('disk full')))
        expect(saved).toBe(false)
        vi.advanceTimersByTime(SAVED_VISIBLE_MS * 4)
        expect(status.statusOf('opds.port')).toEqual({ state: 'error', error: 'disk full' })
        await status.track('opds.port', Promise.resolve())
        expect(status.statusOf('opds.port')?.state).toBe('saved')
    })

    it('a new save restarts the "saved" delay', async () => {
        const status = useSaveStatusStore()
        await status.track('k', Promise.resolve())
        vi.advanceTimersByTime(SAVED_VISIBLE_MS - 100)
        await status.track('k', Promise.resolve())
        vi.advanceTimersByTime(200)
        expect(status.statusOf('k')?.state).toBe('saved')
    })
})
